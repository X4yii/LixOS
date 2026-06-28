use alloc::string::String;
use alloc::vec::Vec;
use xmas_elf::ElfFile;
use xmas_elf::program::Type;
use crate::memory::vmm::{map_page, create_userspace_pml4};

pub fn load_elf(elf_data: &[u8], name: &str, privilege: u8) -> Result<u64, &'static str> {
    let elf = ElfFile::new(elf_data).map_err(|_| "Archivo ELF inválido")?;

    if elf.header.pt1.class() != xmas_elf::header::Class::SixtyFour {
        return Err("ELF no es de 64 bits");
    }
    if elf.header.pt2.type_().as_type() != xmas_elf::header::Type::Executable {
        return Err("ELF no es un ejecutable");
    }

    let new_pml4 = unsafe { crate::memory::vmm::create_userspace_pml4() }?;

    for ph in elf.program_iter() {
        if let Ok(Type::Load) = ph.get_type() {
            let vaddr = ph.virtual_addr();
            let mem_size = ph.mem_size();
            let file_size = ph.file_size();
            let offset = ph.offset();

            let flags = ph.flags();
            let writable = flags.is_write();
            let executable = flags.is_execute();

            let start_page = vaddr & !0xFFF;
            let end_page = (vaddr + mem_size + 0xFFF) & !0xFFF;

            if vaddr.checked_add(mem_size).map_or(true, |end| end > 0x0000_8000_0000_0000) {
                return Err("Segmento ELF fuera del espacio de usuario (¿enlazado con linker del kernel?)");
            }

            for page in (start_page..end_page).step_by(4096) {
                let phys_frame = crate::memory::pmm::alloc_frame()
                    .ok_or("Sin memoria física para ELF")?;
                
                unsafe {
                    map_page(new_pml4, page, phys_frame as u64 * 4096, true, writable, !executable)?;
                }

                let page_offset = if page > vaddr { page - vaddr } else { 0 };
                
                let hhdm_offset = crate::HHDM_REQUEST.get_response().map(|r| r.offset()).unwrap_or(0xffff800000000000);
                let dest_ptr = (hhdm_offset + (phys_frame as u64 * 4096)) as *mut u8;
                
                if page_offset < file_size {
                    let page_start_in_seg = if vaddr > page { 0 } else { page - vaddr };
                    let dest_offset = if vaddr > page { vaddr - page } else { 0 };
                    
                    let copy_len = core::cmp::min(4096 - dest_offset, file_size - page_start_in_seg);
                    
                    unsafe {
                        if dest_offset > 0 {
                            core::ptr::write_bytes(dest_ptr, 0, dest_offset as usize);
                        }
                        core::ptr::copy_nonoverlapping(
                            elf_data.as_ptr().add(offset as usize + page_start_in_seg as usize),
                            dest_ptr.add(dest_offset as usize),
                            copy_len as usize
                        );
                        let end_offset = dest_offset + copy_len;
                        if end_offset < 4096 {
                            core::ptr::write_bytes(dest_ptr.add(end_offset as usize), 0, (4096 - end_offset) as usize);
                        }
                    }
                } else {
                    unsafe {
                        core::ptr::write_bytes(dest_ptr, 0, 4096);
                    }
                }
            }
        }
    }

    let stack_top = 0x0000_7FFF_FFFF_F000u64;
    let stack_pages = 4;
    for i in 0..stack_pages {
        let page = stack_top - (i + 1) * 4096;
        let phys_frame = crate::memory::pmm::alloc_frame().ok_or("Sin memoria para stack Ring 3")?;
        unsafe {
            map_page(new_pml4, page, phys_frame as u64 * 4096, true, true, true)?;
        }
    }

    let pid = crate::task::scheduler::spawn_user_process(
        elf.header.pt2.entry_point(),
        stack_top,
        new_pml4,
        name,
        privilege
    );

    Ok(pid)
}