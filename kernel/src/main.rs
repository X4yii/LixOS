#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

extern crate alloc;

pub mod arch;
pub mod boot;
pub mod drivers;
#[macro_use]
pub mod klog;
pub mod memory;
pub mod panic;
pub mod sync;
pub mod fs;
pub mod task;
pub mod shell;

#[used]
#[link_section = ".requests_start_marker"]
static _REQUESTS_START: boot::limine::RequestsStartMarker =
    boot::limine::RequestsStartMarker::new();

#[used]
#[link_section = ".requests"]
static BASE_REVISION: boot::limine::BaseRevision = boot::limine::BaseRevision::new();

#[used]
#[link_section = ".requests"]
pub static MEMORY_MAP_REQUEST: boot::limine::MemoryMapRequest = boot::limine::MemoryMapRequest::new();

#[used]
#[link_section = ".requests"]
pub static HHDM_REQUEST: boot::limine::HhdmRequest = boot::limine::HhdmRequest::new();

#[used]
#[link_section = ".requests"]
static MODULE_REQUEST: boot::limine::ModuleRequest = boot::limine::ModuleRequest::new();

#[used]
#[link_section = ".requests"]
static FRAMEBUFFER_REQUEST: boot::limine::FramebufferRequest =
    boot::limine::FramebufferRequest::new();

#[used]
#[link_section = ".requests_end_marker"]
static _REQUESTS_END: boot::limine::RequestsEndMarker =
    boot::limine::RequestsEndMarker::new();

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let serial = drivers::serial::init();
    klog::register_console(serial);

    if let Some(fb_resp) = FRAMEBUFFER_REQUEST.get_response() {
        if let Some(fb) = fb_resp.framebuffers().next() {
            if let Some(fb_console) = drivers::framebuffer::init(
                fb.addr(),
                fb.width() as usize,
                fb.height() as usize,
                fb.pitch() as usize,
                fb.bpp(),
            ) {
                klog::register_console(fb_console);
            }
        }
    }

    kprintln!("========================================");
    kprintln!("        LixOS v0.0.1 (x86_64)           ");
    kprintln!("========================================");
    kprintln!("[lix] Arranque de kernel en Ring 0 exitoso.");
    kprintln!("[lix] Linker Higher-Half (-2 GB): 0xffffffff80000000");

    if let Some(hhdm) = HHDM_REQUEST.get_response() {
        kprintln!("[lix] HHDM offset físico: {:#x}", hhdm.offset());
    } else {
        kprintln!("[lix] AVISO: HHDM no provisto por el bootloader");
    }

    memory::init();

    crate::arch::x86_64::init();

    fs::init();

    drivers::pci::init();

    task::init();
    if let Some(module_resp) = MODULE_REQUEST.get_response() {
        let modules = module_resp.modules();
        kprintln!("[lix] Módulos cargados por bootloader: {}", modules.len());
        for module in modules {
            let name = module.path().to_str().unwrap_or("unknown");
            kprintln!("[lix] Parseando módulo: {}", name);
            let data = unsafe { core::slice::from_raw_parts(module.addr() as *const u8, module.size() as usize) };
            let archive = fs::tar::TarArchive::new(data);
            let mut vfs = fs::vfs::VFS.lock();
            for file in archive.files() {
                kprintln!("  -> FS: {} ({} bytes)", file.name, file.size);
                vfs.add_file(alloc::string::String::from(file.name), file.data);
            }
        }
    } else {
        kprintln!("[lix] AVISO: No se encontraron módulos Initramfs.");
    }


    if let Some(fb_resp) = FRAMEBUFFER_REQUEST.get_response() {
        if let Some(fb) = fb_resp.framebuffers().next() {
            kprintln!(
                "[lix] Framebuffer activo: {}x{} @ {} bpp (pitch: {} bytes)",
                fb.width(),
                fb.height(),
                fb.bpp(),
                fb.pitch()
            );
        }
    }

    kprintln!("[lix] Consola unificada activa: Serial (COM1) + GOP Framebuffer + RAM Ring Buffer.");
    
    crate::task::scheduler::spawn_kernel_thread(crate::drivers::tty::run_kernel_shell, "vanish");
    crate::task::scheduler::SCHEDULER.lock().started = true;
    crate::arch::x86_64::halt_loop();
}
