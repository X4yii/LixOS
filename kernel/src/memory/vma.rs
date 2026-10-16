use crate::memory::pmm::{self, PAGE_SIZE};
use crate::memory::vmm;
use alloc::vec::Vec;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmaFlags(pub u32);
impl VmaFlags {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXEC: Self = Self(1 << 2);
    pub const USER: Self = Self(1 << 3);
    pub const COW: Self = Self(1 << 4);
    pub const ANONYMOUS: Self = Self(1 << 5);
    pub const STACK: Self = Self(1 << 6);
    pub const HEAP: Self = Self(1 << 7);
    #[inline]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
    #[inline]
    pub const fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmaBacking {
    Anonymous,
    Stack,
    Heap,
    File { offset: u64 },
    Vmo { handle: u32, offset: u64 },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vma {
    pub start: u64,
    pub end: u64,
    pub flags: VmaFlags,
    pub backing: VmaBacking,
}
impl Vma {
    pub fn new(
        start: u64,
        end: u64,
        flags: VmaFlags,
        backing: VmaBacking,
    ) -> Result<Self, &'static str> {
        if start >= end {
            return Err("Dirección de inicio de VMA debe ser menor que fin");
        }
        if start % PAGE_SIZE as u64 != 0 || end % PAGE_SIZE as u64 != 0 {
            return Err("Límites de VMA deben estar alineados a página de 4KB");
        }
        if end > 0x0000_8000_0000_0000 {
            return Err("VMA fuera del espacio de usuario canónico");
        }
        Ok(Self {
            start,
            end,
            flags,
            backing,
        })
    }
    #[inline]
    pub fn contains(&self, addr: u64) -> bool {
        addr >= self.start && addr < self.end
    }
    #[inline]
    pub fn size(&self) -> usize {
        (self.end - self.start) as usize
    }
    #[inline]
    pub fn page_count(&self) -> usize {
        self.size() / PAGE_SIZE
    }
}
#[derive(Debug, Clone, Default)]
pub struct VmaManager {
    vmas: Vec<Vma>,
}
impl VmaManager {
    pub const fn new() -> Self {
        Self { vmas: Vec::new() }
    }
    pub fn vmas(&self) -> &[Vma] {
        &self.vmas
    }
    pub fn add_vma(&mut self, vma: Vma) -> Result<(), &'static str> {
        for existing in &self.vmas {
            if vma.start < existing.end && existing.start < vma.end {
                return Err("VMA se solapa con una región existente");
            }
        }
        let idx = self
            .vmas
            .binary_search_by_key(&vma.start, |v| v.start)
            .unwrap_or_else(|insert_pos| insert_pos);
        self.vmas.insert(idx, vma);
        Ok(())
    }
    pub fn find_vma(&self, addr: u64) -> Option<&Vma> {
        let index = self.vmas.partition_point(|vma| vma.start <= addr);
        let vma = self.vmas.get(index.checked_sub(1)?)?;
        vma.contains(addr).then_some(vma)
    }
    pub fn find_vma_mut(&mut self, addr: u64) -> Option<&mut Vma> {
        let index = self.vmas.partition_point(|vma| vma.start <= addr);
        let vma = self.vmas.get_mut(index.checked_sub(1)?)?;
        if vma.contains(addr) {
            Some(vma)
        } else {
            None
        }
    }
    pub fn remove_vma(&mut self, start: u64) -> Option<Vma> {
        if let Ok(idx) = self.vmas.binary_search_by_key(&start, |v| v.start) {
            Some(self.vmas.remove(idx))
        } else {
            None
        }
    }
    pub fn find_free_range(&self, len: usize) -> Option<u64> {
        let mut candidate = 0x0000_1000_0000_0000u64;
        let len_u64 = len as u64;
        for vma in &self.vmas {
            if vma.end <= candidate {
                continue;
            }
            if vma.start > candidate && vma.start - candidate >= len_u64 {
                return Some(candidate);
            }
            candidate = candidate.max(vma.end);
        }
        if candidate.checked_add(len_u64)? <= 0x0000_7F00_0000_0000u64 {
            Some(candidate)
        } else {
            None
        }
    }
    pub fn unmap_range(&mut self, start: u64, end: u64) {
        let mut new_vmas = Vec::with_capacity(self.vmas.len());
        for vma in self.vmas.drain(..) {
            if vma.end <= start || vma.start >= end {
                new_vmas.push(vma);
            } else if vma.start < start && vma.end > end {
                let left = Vma {
                    start: vma.start,
                    end: start,
                    flags: vma.flags,
                    backing: vma.backing.clone(),
                };
                let right = Vma {
                    start: end,
                    end: vma.end,
                    flags: vma.flags,
                    backing: vma.backing,
                };
                new_vmas.push(left);
                new_vmas.push(right);
            } else if vma.start < start {
                let left = Vma {
                    start: vma.start,
                    end: start,
                    flags: vma.flags,
                    backing: vma.backing,
                };
                new_vmas.push(left);
            } else if vma.end > end {
                let right = Vma {
                    start: end,
                    end: vma.end,
                    flags: vma.flags,
                    backing: vma.backing,
                };
                new_vmas.push(right);
            }
        }
        self.vmas = new_vmas;
    }
    pub fn handle_page_fault(
        &mut self,
        pml4_frame: usize,
        fault_addr: u64,
        error_code: u64,
    ) -> Result<(), &'static str> {
        let vma = self
            .find_vma(fault_addr)
            .ok_or("Dirección de fallo no pertenece a ninguna VMA")?;
        let is_write_fault = (error_code & (1 << 1)) != 0;
        let is_instruction_fetch = (error_code & (1 << 4)) != 0;
        let is_protection_violation = (error_code & 1) != 0;
        if is_instruction_fetch && !vma.flags.contains(VmaFlags::EXEC) {
            return Err("Ejecución de instrucción no permitida en VMA no ejecutable");
        }
        if is_write_fault
            && !vma.flags.contains(VmaFlags::WRITE)
            && !vma.flags.contains(VmaFlags::COW)
        {
            return Err("Escritura no permitida en VMA de solo lectura");
        }
        let aligned_page = fault_addr & !(PAGE_SIZE as u64 - 1);
        if is_protection_violation && is_write_fault && vma.flags.contains(VmaFlags::COW) {
            let orig_frame = unsafe { vmm::get_mapped_frame(pml4_frame, aligned_page) }
                .ok_or("No existe frame mapeado para resolver COW")?;
            let refcount = pmm::get_ref(orig_frame);
            if refcount > 1 {
                let new_frame = pmm::alloc_frame().ok_or("Sin memoria física para resolver COW")?;
                let hhdm = crate::memory::address::hhdm_offset().get();
                let new_page_virt = (hhdm + (new_frame * PAGE_SIZE) as u64) as *mut u8;
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        aligned_page as *const u8,
                        new_page_virt,
                        PAGE_SIZE,
                    );
                    vmm::remap_page(
                        pml4_frame,
                        aligned_page,
                        (new_frame * PAGE_SIZE) as u64,
                        true,
                        true,
                        !vma.flags.contains(VmaFlags::EXEC),
                    )?;
                    crate::arch::x86_64::tlb::flush_tlb_global(aligned_page, 1);
                }
                pmm::free_frame(orig_frame);
            } else {
                unsafe {
                    vmm::remap_page(
                        pml4_frame,
                        aligned_page,
                        (orig_frame * PAGE_SIZE) as u64,
                        true,
                        true,
                        !vma.flags.contains(VmaFlags::EXEC),
                    )?;
                    crate::arch::x86_64::tlb::flush_tlb_global(aligned_page, 1);
                }
            }
            return Ok(());
        }
        if !is_protection_violation {
            if let Some(slot_id) = unsafe { vmm::get_swap_slot(pml4_frame, aligned_page) } {
                let new_frame = pmm::alloc_frame()
                    .ok_or("Sin memoria física para recuperar página swappeada")?;
                let hhdm = crate::memory::address::hhdm_offset().get();
                let new_page_virt = (hhdm + (new_frame * PAGE_SIZE) as u64) as *mut u8;
                let page_slice =
                    unsafe { core::slice::from_raw_parts_mut(new_page_virt, PAGE_SIZE) };
                crate::memory::zram::decompress_page(slot_id, page_slice)?;
                let is_writable = vma.flags.contains(VmaFlags::WRITE);
                let is_nx = !vma.flags.contains(VmaFlags::EXEC);
                unsafe {
                    vmm::map_page(
                        pml4_frame,
                        aligned_page,
                        (new_frame * PAGE_SIZE) as u64,
                        true,
                        is_writable,
                        is_nx,
                    )?;
                    crate::arch::x86_64::tlb::flush_tlb_global(aligned_page, 1);
                }
                return Ok(());
            }
            let new_frame = pmm::alloc_frame().ok_or("Sin memoria física para demand paging")?;
            let hhdm = crate::memory::address::hhdm_offset().get();
            let new_page_virt = (hhdm + (new_frame * PAGE_SIZE) as u64) as *mut u8;
            unsafe {
                core::ptr::write_bytes(new_page_virt, 0, PAGE_SIZE);
                let is_writable = vma.flags.contains(VmaFlags::WRITE);
                let is_nx = !vma.flags.contains(VmaFlags::EXEC);
                vmm::map_page(
                    pml4_frame,
                    aligned_page,
                    (new_frame * PAGE_SIZE) as u64,
                    true,
                    is_writable,
                    is_nx,
                )?;
                crate::arch::x86_64::tlb::flush_tlb_global(aligned_page, 1);
            }
            return Ok(());
        }
        Err("Page fault no recuperable")
    }
}
