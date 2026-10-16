use crate::memory::pmm::{self, PAGE_SIZE};
use crate::sync::IrqSafeSpinlock;
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};
const PAGE_PRESENT: u64 = 1 << 0;
const PAGE_WRITABLE: u64 = 1 << 1;
const PAGE_USER: u64 = 1 << 2;
const PAGE_NX: u64 = 1 << 63;
const PAGE_HUGE: u64 = 1 << 7;
const PAGE_WRITE_THROUGH: u64 = 1 << 3;
const PAGE_CACHE_DISABLE: u64 = 1 << 4;
const USER_ADDRESS_LIMIT: u64 = 0x0000_8000_0000_0000;
const MMIO_END: u64 = 0xFFFF_E000_0000_0000;
static NEXT_MMIO_VADDR: AtomicU64 = AtomicU64::new(crate::memory::address::layout::MMIO_START);
static PT_LOCK: IrqSafeSpinlock<()> = IrqSafeSpinlock::new(());
#[repr(C, align(4096))]
pub struct PageTable {
    pub entries: [u64; 512],
}
impl PageTable {
    pub fn new() -> Option<usize> {
        let frame = pmm::alloc_frame()?;
        let hhdm = hhdm_offset();
        let pt_vaddr = hhdm + (frame * PAGE_SIZE) as u64;
        unsafe {
            ptr::write_bytes(pt_vaddr as *mut u8, 0, PAGE_SIZE);
        }
        Some(frame)
    }
    pub fn clear(&mut self) {
        self.entries.fill(0);
    }
}
fn hhdm_offset() -> u64 {
    crate::memory::address::hhdm_offset().get()
}
unsafe fn phys_to_virt(phys_addr: u64) -> *mut PageTable {
    let physical = crate::memory::address::PhysAddr::new(phys_addr);
    crate::memory::address::hhdm_offset()
        .map_phys(physical)
        .unwrap_or_else(|error| panic!("invalid page-table physical address: {error}"))
        .get() as *mut PageTable
}
pub fn validate_user_range(ptr: u64, len: usize, write: bool) -> Result<(), &'static str> {
    if len == 0 {
        return Ok(());
    }
    let _guard = PT_LOCK.lock();
    let end = ptr
        .checked_add(len as u64)
        .ok_or("Rango de usuario desbordado")?;
    if ptr < PAGE_SIZE as u64 || end > USER_ADDRESS_LIMIT || end <= ptr {
        return Err("Rango fuera del espacio de usuario");
    }
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
    }
    let pml4_phys = cr3 & 0x000F_FFFF_FFFF_F000;
    let mut page = ptr & !(PAGE_SIZE as u64 - 1);
    let last_page = (end - 1) & !(PAGE_SIZE as u64 - 1);
    loop {
        let (pml4_index, pdpt_index, pd_index, pt_index) = (
            ((page >> 39) & 0x1ff) as usize,
            ((page >> 30) & 0x1ff) as usize,
            ((page >> 21) & 0x1ff) as usize,
            ((page >> 12) & 0x1ff) as usize,
        );
        unsafe {
            let pml4 = &*phys_to_virt(pml4_phys);
            let pml4e = pml4.entries[pml4_index];
            if !user_entry_accessible(pml4e, write) {
                return Err("Página de usuario no accesible");
            }
            let pdpt = &*phys_to_virt(pml4e & 0x000F_FFFF_FFFF_F000);
            let pdpte = pdpt.entries[pdpt_index];
            if !user_entry_accessible(pdpte, write) {
                return Err("Página de usuario no accesible");
            }
            if pdpte & PAGE_HUGE != 0 {
                if page == last_page {
                    return Ok(());
                }
                page = ((page >> 30) + 1) << 30;
                continue;
            }
            let pd = &*phys_to_virt(pdpte & 0x000F_FFFF_FFFF_F000);
            let pde = pd.entries[pd_index];
            if !user_entry_accessible(pde, write) {
                return Err("Página de usuario no accesible");
            }
            if pde & PAGE_HUGE != 0 {
                if page == last_page {
                    return Ok(());
                }
                page = ((page >> 21) + 1) << 21;
                continue;
            }
            let pt = &*phys_to_virt(pde & 0x000F_FFFF_FFFF_F000);
            if !user_entry_accessible(pt.entries[pt_index], write) {
                return Err("Página de usuario no accesible");
            }
        }
        if page == last_page {
            return Ok(());
        }
        page = page
            .checked_add(PAGE_SIZE as u64)
            .ok_or("Rango de usuario desbordado")?;
    }
}
#[inline]
fn user_entry_accessible(entry: u64, write: bool) -> bool {
    entry & (PAGE_PRESENT | PAGE_USER) == (PAGE_PRESENT | PAGE_USER)
        && (!write || entry & PAGE_WRITABLE != 0)
}
pub fn init() {
    crate::kprintln!("[vmm] Inicializando VMM...");
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3);
    }
    crate::kprintln!("[vmm] Directorio base actual (Limine PML4): {:#x}", cr3);
}
pub unsafe fn map_page(
    pml4_frame: usize,
    virt_addr: u64,
    phys_addr: u64,
    user: bool,
    writable: bool,
    nx: bool,
) -> Result<(), &'static str> {
    map_page_with_cache(
        pml4_frame, virt_addr, phys_addr, user, writable, nx, false, false,
    )
}
pub unsafe fn remap_page(
    pml4_frame: usize,
    virt_addr: u64,
    phys_addr: u64,
    user: bool,
    writable: bool,
    nx: bool,
) -> Result<(), &'static str> {
    map_page_with_cache(
        pml4_frame, virt_addr, phys_addr, user, writable, nx, false, true,
    )
}
unsafe fn map_page_with_cache(
    pml4_frame: usize,
    virt_addr: u64,
    phys_addr: u64,
    user: bool,
    writable: bool,
    nx: bool,
    uncached: bool,
    replace: bool,
) -> Result<(), &'static str> {
    if virt_addr & (PAGE_SIZE as u64 - 1) != 0 || phys_addr & (PAGE_SIZE as u64 - 1) != 0 {
        return Err("Dirección virtual o física sin alinear a página");
    }
    if user && virt_addr >= USER_ADDRESS_LIMIT {
        return Err("Mapping de usuario fuera del rango permitido");
    }
    let _guard = PT_LOCK.lock();
    let pml4_index = ((virt_addr >> 39) & 0x1FF) as usize;
    let pdpt_index = ((virt_addr >> 30) & 0x1FF) as usize;
    let pd_index = ((virt_addr >> 21) & 0x1FF) as usize;
    let pt_index = ((virt_addr >> 12) & 0x1FF) as usize;
    let pml4 = &mut *phys_to_virt((pml4_frame * PAGE_SIZE) as u64);
    if pml4.entries[pml4_index] & PAGE_PRESENT == 0 {
        let frame = PageTable::new().ok_or("No memory for PDPT")?;
        let mut flags = PAGE_PRESENT | PAGE_WRITABLE;
        if user {
            flags |= PAGE_USER;
        }
        pml4.entries[pml4_index] = (frame * PAGE_SIZE) as u64 | flags;
    } else if user && pml4.entries[pml4_index] & PAGE_USER == 0 {
        return Err("Conflicto con mapping supervisor existente");
    }
    let pdpt_phys = pml4.entries[pml4_index] & 0x000FFFFFFFFFF000;
    let pdpt = &mut *phys_to_virt(pdpt_phys);
    if pdpt.entries[pdpt_index] & PAGE_PRESENT == 0 {
        let frame = PageTable::new().ok_or("No memory for PD")?;
        let mut flags = PAGE_PRESENT | PAGE_WRITABLE;
        if user {
            flags |= PAGE_USER;
        }
        pdpt.entries[pdpt_index] = (frame * PAGE_SIZE) as u64 | flags;
    } else if pdpt.entries[pdpt_index] & PAGE_HUGE != 0 {
        return Err("No se puede reemplazar un mapping enorme");
    } else if user && pdpt.entries[pdpt_index] & PAGE_USER == 0 {
        return Err("Conflicto con mapping supervisor existente");
    }
    let pd_phys = pdpt.entries[pdpt_index] & 0x000FFFFFFFFFF000;
    let pd = &mut *phys_to_virt(pd_phys);
    if pd.entries[pd_index] & PAGE_PRESENT == 0 {
        let frame = PageTable::new().ok_or("No memory for PT")?;
        let mut flags = PAGE_PRESENT | PAGE_WRITABLE;
        if user {
            flags |= PAGE_USER;
        }
        pd.entries[pd_index] = (frame * PAGE_SIZE) as u64 | flags;
    } else if pd.entries[pd_index] & PAGE_HUGE != 0 {
        return Err("No se puede reemplazar un mapping enorme");
    } else if user && pd.entries[pd_index] & PAGE_USER == 0 {
        return Err("Conflicto con mapping supervisor existente");
    }
    let pt_phys = pd.entries[pd_index] & 0x000FFFFFFFFFF000;
    let pt = &mut *phys_to_virt(pt_phys);
    if pt.entries[pt_index] & PAGE_PRESENT != 0 && !replace {
        return Err("La página virtual ya está mapeada");
    }
    let mut flags = PAGE_PRESENT;
    if writable {
        flags |= PAGE_WRITABLE;
    }
    if user {
        flags |= PAGE_USER;
    }
    if nx {
        flags |= PAGE_NX;
    }
    if uncached {
        flags |= PAGE_CACHE_DISABLE | PAGE_WRITE_THROUGH;
    }
    pt.entries[pt_index] = (phys_addr & 0x000FFFFFFFFFF000) | flags;
    crate::arch::x86_64::tlb::flush_tlb_global(virt_addr, 1);
    Ok(())
}
pub unsafe fn map_mmio_region(
    pml4_frame: usize,
    physical: crate::memory::address::PhysAddr,
    length: usize,
) -> Result<crate::memory::address::VirtAddr, &'static str> {
    if length == 0 {
        return Err("MMIO mapping length must be non-zero");
    }
    let page_size = PAGE_SIZE as u64;
    let phys_end = physical
        .get()
        .checked_add(length as u64)
        .ok_or("MMIO range overflow")?;
    if phys_end > 0x0010_0000_0000_0000 {
        return Err("MMIO physical range exceeds x86_64 page-table address width");
    }
    let phys_base = physical.get() & !(page_size - 1);
    let offset = physical.get() - phys_base;
    let span = offset
        .checked_add(length as u64)
        .ok_or("MMIO range overflow")?;
    let page_count = span
        .checked_add(page_size - 1)
        .ok_or("MMIO range overflow")?
        / page_size;
    let mapped_size = page_count
        .checked_mul(page_size)
        .ok_or("MMIO range overflow")?;
    let mut virt_base = NEXT_MMIO_VADDR.load(Ordering::Acquire);
    loop {
        let next = virt_base
            .checked_add(mapped_size)
            .ok_or("MMIO virtual region exhausted")?;
        if next > MMIO_END {
            return Err("MMIO virtual region exhausted");
        }
        match NEXT_MMIO_VADDR.compare_exchange_weak(
            virt_base,
            next,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => break,
            Err(current) => virt_base = current,
        }
    }
    let mut mapped = 0u64;
    while mapped < mapped_size {
        let result = map_page_with_cache(
            pml4_frame,
            virt_base + mapped,
            phys_base + mapped,
            false,
            true,
            true,
            true,
            false,
        );
        if let Err(error) = result {
            while mapped > 0 {
                mapped -= page_size;
                let _ = unmap_page(pml4_frame, virt_base + mapped, false);
            }
            let _ = NEXT_MMIO_VADDR.compare_exchange(
                virt_base + mapped_size,
                virt_base,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            return Err(error);
        }
        mapped += page_size;
    }
    crate::memory::address::VirtAddr::new(virt_base + offset)
        .map_err(|_| "MMIO mapping produced a non-canonical address")
}
pub unsafe fn unmap_page(
    pml4_frame: usize,
    virt_addr: u64,
    free_data_frame: bool,
) -> Result<(), &'static str> {
    if virt_addr & (PAGE_SIZE as u64 - 1) != 0 {
        return Err("Dirección virtual sin alinear a página");
    }
    let _guard = PT_LOCK.lock();
    let indices = [
        ((virt_addr >> 39) & 0x1ff) as usize,
        ((virt_addr >> 30) & 0x1ff) as usize,
        ((virt_addr >> 21) & 0x1ff) as usize,
        ((virt_addr >> 12) & 0x1ff) as usize,
    ];
    let pml4 = &mut *phys_to_virt((pml4_frame * PAGE_SIZE) as u64);
    let pml4e = pml4.entries[indices[0]];
    if pml4e & PAGE_PRESENT == 0 {
        return Ok(());
    }
    let pdpt_phys = pml4e & 0x000F_FFFF_FFFF_F000;
    let pdpt = &mut *phys_to_virt(pdpt_phys);
    let pdpte = pdpt.entries[indices[1]];
    if pdpte & PAGE_PRESENT == 0 {
        return Ok(());
    }
    if pdpte & PAGE_HUGE != 0 {
        return Err("No se puede retirar parcialmente un mapping enorme");
    }
    let pd_phys = pdpte & 0x000F_FFFF_FFFF_F000;
    let pd = &mut *phys_to_virt(pd_phys);
    let pde = pd.entries[indices[2]];
    if pde & PAGE_PRESENT == 0 {
        return Ok(());
    }
    if pde & PAGE_HUGE != 0 {
        return Err("No se puede retirar parcialmente un mapping enorme");
    }
    let pt_phys = pde & 0x000F_FFFF_FFFF_F000;
    let pt = &mut *phys_to_virt(pt_phys);
    let pte = pt.entries[indices[3]];
    if pte & PAGE_PRESENT != 0 {
        pt.entries[indices[3]] = 0;
        if free_data_frame {
            pmm::free_frame(((pte & 0x000F_FFFF_FFFF_F000) / PAGE_SIZE as u64) as usize);
        }
    }
    crate::arch::x86_64::tlb::flush_tlb_global(virt_addr, 1);
    if indices[0] < 256 && pt.entries.iter().all(|entry| *entry == 0) {
        pd.entries[indices[2]] = 0;
        pmm::free_frame((pt_phys / PAGE_SIZE as u64) as usize);
        if pd.entries.iter().all(|entry| *entry == 0) {
            pdpt.entries[indices[1]] = 0;
            pmm::free_frame((pd_phys / PAGE_SIZE as u64) as usize);
            if pdpt.entries.iter().all(|entry| *entry == 0) {
                pml4.entries[indices[0]] = 0;
                pmm::free_frame((pdpt_phys / PAGE_SIZE as u64) as usize);
            }
        }
    }
    Ok(())
}
pub unsafe fn create_userspace_pml4() -> Result<usize, &'static str> {
    let _guard = PT_LOCK.lock();
    let frame = PageTable::new().ok_or("No memory for PML4")?;
    let new_pml4_ptr = phys_to_virt((frame * PAGE_SIZE) as u64);
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3);
    let current_pml4_phys = cr3 & 0x000FFFFFFFFFF000;
    let current_pml4_ptr = phys_to_virt(current_pml4_phys);
    let new_pml4 = &mut *new_pml4_ptr;
    let current_pml4 = &*current_pml4_ptr;
    for i in 256..512 {
        new_pml4.entries[i] = current_pml4.entries[i] & !PAGE_USER;
    }
    Ok(frame)
}
pub unsafe fn destroy_userspace_pml4(pml4_frame: usize) {
    let _guard = PT_LOCK.lock();
    let pml4 = &mut *phys_to_virt((pml4_frame * PAGE_SIZE) as u64);
    for i in 0..256 {
        let pml4e = pml4.entries[i];
        if pml4e & PAGE_PRESENT != 0 {
            let pdpt_phys = pml4e & 0x000FFFFFFFFFF000;
            let pdpt = &mut *phys_to_virt(pdpt_phys);
            for j in 0..512 {
                let pdpte = pdpt.entries[j];
                if pdpte & PAGE_PRESENT != 0 {
                    if pdpte & PAGE_HUGE != 0 {
                        continue;
                    }
                    let pd_phys = pdpte & 0x000FFFFFFFFFF000;
                    let pd = &mut *phys_to_virt(pd_phys);
                    for k in 0..512 {
                        let pde = pd.entries[k];
                        if pde & PAGE_PRESENT != 0 {
                            if pde & PAGE_HUGE != 0 {
                                continue;
                            }
                            let pt_phys = pde & 0x000FFFFFFFFFF000;
                            let pt = &mut *phys_to_virt(pt_phys);
                            for m in 0..512 {
                                let pte = pt.entries[m];
                                if pte & PAGE_PRESENT != 0 {
                                    let frame_phys = pte & 0x000FFFFFFFFFF000;
                                    pmm::free_frame((frame_phys / PAGE_SIZE as u64) as usize);
                                }
                            }
                            pmm::free_frame((pt_phys / PAGE_SIZE as u64) as usize);
                        }
                    }
                    pmm::free_frame((pd_phys / PAGE_SIZE as u64) as usize);
                }
            }
            pmm::free_frame((pdpt_phys / PAGE_SIZE as u64) as usize);
        }
    }
    pmm::free_frame(pml4_frame);
}
pub unsafe fn get_mapped_frame(pml4_frame: usize, virt_addr: u64) -> Option<usize> {
    let _guard = PT_LOCK.lock();
    let indices = [
        ((virt_addr >> 39) & 0x1ff) as usize,
        ((virt_addr >> 30) & 0x1ff) as usize,
        ((virt_addr >> 21) & 0x1ff) as usize,
        ((virt_addr >> 12) & 0x1ff) as usize,
    ];
    let pml4 = &*phys_to_virt((pml4_frame * PAGE_SIZE) as u64);
    let pml4e = pml4.entries[indices[0]];
    if pml4e & PAGE_PRESENT == 0 {
        return None;
    }
    let pdpt = &*phys_to_virt(pml4e & 0x000F_FFFF_FFFF_F000);
    let pdpte = pdpt.entries[indices[1]];
    if pdpte & PAGE_PRESENT == 0 {
        return None;
    }
    let pd = &*phys_to_virt(pdpte & 0x000F_FFFF_FFFF_F000);
    let pde = pd.entries[indices[2]];
    if pde & PAGE_PRESENT == 0 {
        return None;
    }
    let pt = &*phys_to_virt(pde & 0x000F_FFFF_FFFF_F000);
    let pte = pt.entries[indices[3]];
    if pte & PAGE_PRESENT == 0 {
        return None;
    }
    Some(((pte & 0x000F_FFFF_FFFF_F000) / PAGE_SIZE as u64) as usize)
}
pub const PAGE_SWAPPED: u64 = 1 << 9;
pub unsafe fn swap_out_page(
    pml4_frame: usize,
    virt_addr: u64,
    slot_id: u32,
) -> Result<usize, &'static str> {
    let _guard = PT_LOCK.lock();
    let indices = [
        ((virt_addr >> 39) & 0x1ff) as usize,
        ((virt_addr >> 30) & 0x1ff) as usize,
        ((virt_addr >> 21) & 0x1ff) as usize,
        ((virt_addr >> 12) & 0x1ff) as usize,
    ];
    let pml4 = &mut *phys_to_virt((pml4_frame * PAGE_SIZE) as u64);
    let pml4e = pml4.entries[indices[0]];
    if pml4e & PAGE_PRESENT == 0 {
        return Err("PML4 entry not present");
    }
    let pdpt = &mut *phys_to_virt(pml4e & 0x000F_FFFF_FFFF_F000);
    let pdpte = pdpt.entries[indices[1]];
    if pdpte & PAGE_PRESENT == 0 {
        return Err("PDPT entry not present");
    }
    let pd = &mut *phys_to_virt(pdpte & 0x000F_FFFF_FFFF_F000);
    let pde = pd.entries[indices[2]];
    if pde & PAGE_PRESENT == 0 {
        return Err("PD entry not present");
    }
    let pt = &mut *phys_to_virt(pde & 0x000F_FFFF_FFFF_F000);
    let pte = pt.entries[indices[3]];
    if pte & PAGE_PRESENT == 0 {
        return Err("PTE not present");
    }
    let phys_frame = ((pte & 0x000F_FFFF_FFFF_F000) / PAGE_SIZE as u64) as usize;
    pt.entries[indices[3]] = PAGE_SWAPPED | ((slot_id as u64) << 12);
    crate::arch::x86_64::tlb::flush_tlb_global(virt_addr, 1);
    Ok(phys_frame)
}
pub unsafe fn get_swap_slot(pml4_frame: usize, virt_addr: u64) -> Option<u32> {
    let _guard = PT_LOCK.lock();
    let indices = [
        ((virt_addr >> 39) & 0x1ff) as usize,
        ((virt_addr >> 30) & 0x1ff) as usize,
        ((virt_addr >> 21) & 0x1ff) as usize,
        ((virt_addr >> 12) & 0x1ff) as usize,
    ];
    let pml4 = &*phys_to_virt((pml4_frame * PAGE_SIZE) as u64);
    let pml4e = pml4.entries[indices[0]];
    if pml4e & PAGE_PRESENT == 0 {
        return None;
    }
    let pdpt = &*phys_to_virt(pml4e & 0x000F_FFFF_FFFF_F000);
    let pdpte = pdpt.entries[indices[1]];
    if pdpte & PAGE_PRESENT == 0 {
        return None;
    }
    let pd = &*phys_to_virt(pdpte & 0x000F_FFFF_FFFF_F000);
    let pde = pd.entries[indices[2]];
    if pde & PAGE_PRESENT == 0 {
        return None;
    }
    let pt = &*phys_to_virt(pde & 0x000F_FFFF_FFFF_F000);
    let pte = pt.entries[indices[3]];
    if pte & PAGE_PRESENT == 0 && pte & PAGE_SWAPPED != 0 {
        Some(((pte >> 12) & 0xFFFF_FFFF) as u32)
    } else {
        None
    }
}
