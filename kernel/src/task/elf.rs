use crate::memory::address_space::AddressSpace;
use crate::memory::pmm::{self, PAGE_SIZE};
use xmas_elf::program::{ProgramHeader, Type};
use xmas_elf::ElfFile;
const USER_ADDRESS_LIMIT: u64 = 0x0000_8000_0000_0000;
const USER_STACK_TOP: u64 = 0x0000_7FFF_FFFF_F000;
const USER_STACK_PAGES: u64 = 16;
const MAX_ELF_MAPPED_BYTES: u64 = 256 * 1024 * 1024;
fn load_segment(ph: &ProgramHeader<'_>) -> bool {
    matches!(ph.get_type(), Ok(Type::Load))
}
fn segment_page_range(ph: &ProgramHeader<'_>) -> Result<(u64, u64), &'static str> {
    let vaddr = ph.virtual_addr();
    let mem_size = ph.mem_size();
    let end = vaddr
        .checked_add(mem_size)
        .ok_or("Overflow en segmento ELF")?;
    let rounded_end = end
        .checked_add(PAGE_SIZE as u64 - 1)
        .ok_or("Overflow al alinear segmento ELF")?
        & !(PAGE_SIZE as u64 - 1);
    Ok((vaddr & !(PAGE_SIZE as u64 - 1), rounded_end))
}
fn validate_load_segments(elf: &ElfFile<'_>, elf_data: &[u8]) -> Result<(), &'static str> {
    let entry = elf.header.pt2.entry_point();
    let stack_start = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE as u64;
    let mut mapped_bytes = 0u64;
    let mut has_load_segment = false;
    let mut entry_is_executable = false;
    for (segment_index, ph) in elf.program_iter().enumerate() {
        if !load_segment(&ph) {
            continue;
        }
        has_load_segment = true;
        let vaddr = ph.virtual_addr();
        let mem_size = ph.mem_size();
        let file_size = ph.file_size();
        let offset = ph.offset();
        let end = vaddr
            .checked_add(mem_size)
            .ok_or("Overflow en segmento ELF")?;
        let file_end = offset
            .checked_add(file_size)
            .ok_or("Overflow en datos ELF")?;
        if mem_size == 0 || file_size > mem_size || file_end > elf_data.len() as u64 {
            return Err("Tamaño de segmento ELF inválido");
        }
        if vaddr < PAGE_SIZE as u64 || end > USER_ADDRESS_LIMIT {
            return Err("Segmento ELF fuera del espacio de usuario");
        }
        let align = ph.align();
        if align > 1 && (!align.is_power_of_two() || vaddr % align != offset % align) {
            return Err("Alineación de segmento ELF inválida");
        }
        let flags = ph.flags();
        if flags.is_write() && flags.is_execute() {
            return Err("Segmento ELF viola W^X");
        }
        if flags.is_execute() && entry >= vaddr && entry < end {
            entry_is_executable = true;
        }
        let (start_page, end_page) = segment_page_range(&ph)?;
        if end_page > stack_start {
            return Err("Segmento ELF invade la pila reservada");
        }
        mapped_bytes = mapped_bytes
            .checked_add(end_page - start_page)
            .ok_or("Tamaño de imagen ELF desbordado")?;
        if mapped_bytes > MAX_ELF_MAPPED_BYTES {
            return Err("Imagen ELF excede el límite de memoria permitido");
        }
        for (other_index, other) in elf.program_iter().enumerate() {
            if other_index == segment_index || !load_segment(&other) {
                continue;
            }
            let (other_start, other_end) = segment_page_range(&other)?;
            if start_page < other_end && other_start < end_page {
                return Err("Segmentos ELF comparten páginas virtuales");
            }
        }
    }
    if !has_load_segment || !entry_is_executable {
        return Err("ELF sin segmento cargable ejecutable válido");
    }
    Ok(())
}
pub fn load_elf(
    elf_data: &[u8],
    name: &str,
    privilege: u8,
    rights: lix_sys::Rights,
) -> Result<u64, &'static str> {
    if privilege != 3 {
        return Err(
            "Los ELF de usuario deben ejecutarse en Ring 3; use capabilities para elevar autoridad",
        );
    }
    let elf = ElfFile::new(elf_data).map_err(|_| "Archivo ELF inválido")?;
    if elf.header.pt1.class() != xmas_elf::header::Class::SixtyFour {
        return Err("ELF no es de 64 bits");
    }
    if elf.header.pt2.type_().as_type() != xmas_elf::header::Type::Executable {
        return Err("ELF no es un ejecutable");
    }
    validate_load_segments(&elf, elf_data)?;
    let addr_space = AddressSpace::new()?;
    let hhdm_offset = crate::memory::address::hhdm_offset();
    let mut mapped_user_bytes = 0u64;
    let mut vma_mgr = crate::memory::vma::VmaManager::new();
    for ph in elf.program_iter() {
        if !load_segment(&ph) {
            continue;
        }
        let vaddr = ph.virtual_addr();
        let file_size = ph.file_size();
        let offset = ph.offset();
        let (_, end) = segment_page_range(&ph)?;
        let start = vaddr & !(PAGE_SIZE as u64 - 1);
        let flags = ph.flags();
        let mut vflags = crate::memory::vma::VmaFlags::USER;
        if flags.is_read() {
            vflags = vflags.union(crate::memory::vma::VmaFlags::READ);
        }
        if flags.is_write() {
            vflags = vflags.union(crate::memory::vma::VmaFlags::WRITE);
        }
        if flags.is_execute() {
            vflags = vflags.union(crate::memory::vma::VmaFlags::EXEC);
        }
        let _ = vma_mgr.add_vma(crate::memory::vma::Vma::new(
            start,
            end,
            vflags,
            crate::memory::vma::VmaBacking::Anonymous,
        )?);
        for page in (start..end).step_by(PAGE_SIZE) {
            let phys_frame = pmm::alloc_frame().ok_or("Sin memoria física para ELF")?;
            if let Err(error) = addr_space.map_page(
                page,
                (phys_frame * PAGE_SIZE) as u64,
                true,
                flags.is_write(),
                !flags.is_execute(),
            ) {
                pmm::free_frame(phys_frame);
                return Err(error);
            }
            mapped_user_bytes = mapped_user_bytes.saturating_add(PAGE_SIZE as u64);
            let destination = hhdm_offset
                .map_phys(crate::memory::address::PhysAddr::new(
                    (phys_frame * PAGE_SIZE) as u64,
                ))
                .map_err(|_| "Frame ELF fuera del HHDM")?
                .get() as *mut u8;
            unsafe {
                core::ptr::write_bytes(destination, 0, PAGE_SIZE);
            }
            let page_end = page + PAGE_SIZE as u64;
            let file_end = vaddr + file_size;
            let copy_start = page.max(vaddr);
            let copy_end = page_end.min(file_end);
            if copy_start < copy_end {
                let source_offset = offset + (copy_start - vaddr);
                let destination_offset = (copy_start - page) as usize;
                let copy_len = (copy_end - copy_start) as usize;
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        elf_data.as_ptr().add(source_offset as usize),
                        destination.add(destination_offset),
                        copy_len,
                    );
                }
            }
        }
    }
    let stack_start = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE as u64;
    let stack_flags = crate::memory::vma::VmaFlags::READ
        .union(crate::memory::vma::VmaFlags::WRITE)
        .union(crate::memory::vma::VmaFlags::USER)
        .union(crate::memory::vma::VmaFlags::STACK);
    let _ = vma_mgr.add_vma(crate::memory::vma::Vma::new(
        stack_start,
        USER_STACK_TOP,
        stack_flags,
        crate::memory::vma::VmaBacking::Stack,
    )?);
    for page in (stack_start..USER_STACK_TOP).step_by(PAGE_SIZE) {
        let phys_frame = pmm::alloc_frame().ok_or("Sin memoria para stack Ring 3")?;
        if let Err(error) =
            addr_space.map_page(page, (phys_frame * PAGE_SIZE) as u64, true, true, true)
        {
            pmm::free_frame(phys_frame);
            return Err(error);
        }
        mapped_user_bytes = mapped_user_bytes.saturating_add(PAGE_SIZE as u64);
        let destination = hhdm_offset
            .map_phys(crate::memory::address::PhysAddr::new(
                (phys_frame * PAGE_SIZE) as u64,
            ))
            .map_err(|_| "Stack ELF fuera del HHDM")?
            .get() as *mut u8;
        unsafe {
            core::ptr::write_bytes(destination, 0, PAGE_SIZE);
        }
    }
    let pml4 = addr_space.commit();
    let rights = crate::task::rights::grant_for(name, rights);
    crate::task::scheduler::spawn_user_process(
        elf.header.pt2.entry_point(),
        USER_STACK_TOP,
        pml4,
        name,
        privilege,
        mapped_user_bytes,
        vma_mgr,
        rights,
    )
}
