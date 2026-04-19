use crate::memory::pmm::{self, PAGE_SIZE};
use core::ptr;

const PAGE_PRESENT: u64 = 1 << 0;
const PAGE_WRITABLE: u64 = 1 << 1;
const PAGE_USER: u64 = 1 << 2;
const PAGE_NX: u64 = 1 << 63;

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
    crate::HHDM_REQUEST.get_response().map(|r| r.offset()).unwrap_or(0xffff800000000000)
}

unsafe fn phys_to_virt(phys_addr: u64) -> *mut PageTable {
    (hhdm_offset() + phys_addr) as *mut PageTable
}

pub fn init() {
    crate::kprintln!("[vmm] Inicializando VMM...");
    
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3);
    }
    
    crate::kprintln!("[vmm] Directorio base actual (Limine PML4): {:#x}", cr3);
    
}

pub unsafe fn map_page(pml4_frame: usize, virt_addr: u64, phys_addr: u64, user: bool, writable: bool, nx: bool) -> Result<(), &'static str> {
    let pml4_index = ((virt_addr >> 39) & 0x1FF) as usize;
    let pdpt_index = ((virt_addr >> 30) & 0x1FF) as usize;
    let pd_index = ((virt_addr >> 21) & 0x1FF) as usize;
    let pt_index = ((virt_addr >> 12) & 0x1FF) as usize;

    let pml4 = &mut *phys_to_virt((pml4_frame * PAGE_SIZE) as u64);

    if pml4.entries[pml4_index] & PAGE_PRESENT == 0 {
        let frame = PageTable::new().ok_or("No memory for PDPT")?;
        let mut flags = PAGE_PRESENT | PAGE_WRITABLE;
        if user { flags |= PAGE_USER; }
        pml4.entries[pml4_index] = (frame * PAGE_SIZE) as u64 | flags;
    }
    let pdpt_phys = pml4.entries[pml4_index] & 0x000FFFFFFFFFF000;
    let pdpt = &mut *phys_to_virt(pdpt_phys);

    if pdpt.entries[pdpt_index] & PAGE_PRESENT == 0 {
        let frame = PageTable::new().ok_or("No memory for PD")?;
        let mut flags = PAGE_PRESENT | PAGE_WRITABLE;
        if user { flags |= PAGE_USER; }
        pdpt.entries[pdpt_index] = (frame * PAGE_SIZE) as u64 | flags;
    }
    let pd_phys = pdpt.entries[pdpt_index] & 0x000FFFFFFFFFF000;
    let pd = &mut *phys_to_virt(pd_phys);

    if pd.entries[pd_index] & PAGE_PRESENT == 0 {
        let frame = PageTable::new().ok_or("No memory for PT")?;
        let mut flags = PAGE_PRESENT | PAGE_WRITABLE;
        if user { flags |= PAGE_USER; }
        pd.entries[pd_index] = (frame * PAGE_SIZE) as u64 | flags;
    }
    let pt_phys = pd.entries[pd_index] & 0x000FFFFFFFFFF000;
    let pt = &mut *phys_to_virt(pt_phys);

    let mut flags = PAGE_PRESENT;
    if writable { flags |= PAGE_WRITABLE; }
    if user { flags |= PAGE_USER; }
    if nx { flags |= PAGE_NX; }

    pt.entries[pt_index] = (phys_addr & 0x000FFFFFFFFFF000) | flags;

    core::arch::asm!("invlpg [{}]", in(reg) virt_addr);

    Ok(())
}

pub unsafe fn create_userspace_pml4() -> Result<usize, &'static str> {
    let frame = PageTable::new().ok_or("No memory for PML4")?;
    let new_pml4_ptr = phys_to_virt((frame * PAGE_SIZE) as u64);
    
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3);
    let current_pml4_phys = cr3 & 0x000FFFFFFFFFF000;
    let current_pml4_ptr = phys_to_virt(current_pml4_phys);
    
    let new_pml4 = &mut *new_pml4_ptr;
    let current_pml4 = &*current_pml4_ptr;
    
    for i in 256..512 {
        new_pml4.entries[i] = current_pml4.entries[i];
    }
    
    Ok(frame)
}

pub unsafe fn destroy_userspace_pml4(pml4_frame: usize) {
    let pml4 = &mut *phys_to_virt((pml4_frame * PAGE_SIZE) as u64);
    
    for i in 0..256 {
        let pml4e = pml4.entries[i];
        if pml4e & PAGE_PRESENT != 0 {
            let pdpt_phys = pml4e & 0x000FFFFFFFFFF000;
            let pdpt = &mut *phys_to_virt(pdpt_phys);
            
            for j in 0..512 {
                let pdpte = pdpt.entries[j];
                if pdpte & PAGE_PRESENT != 0 {
                    let pd_phys = pdpte & 0x000FFFFFFFFFF000;
                    let pd = &mut *phys_to_virt(pd_phys);
                    
                    for k in 0..512 {
                        let pde = pd.entries[k];
                        if pde & PAGE_PRESENT != 0 {
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