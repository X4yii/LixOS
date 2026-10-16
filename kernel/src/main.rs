#![no_std]
#![cfg_attr(not(test), no_main)]
#![allow(unsafe_code)]
extern crate alloc;
pub mod arch;
pub mod boot;
pub mod drivers;
#[macro_use]
pub mod klog;
pub mod fs;
pub mod memory;
pub mod panic;
pub mod power;
pub mod shell;
pub mod io_uring;
pub mod sync;
pub mod task;
#[used]
#[link_section = ".requests_start_marker"]
static _REQUESTS_START: boot::limine::RequestsStartMarker =
    boot::limine::RequestsStartMarker::new();
#[used]
#[link_section = ".requests"]
static BASE_REVISION: boot::limine::BaseRevision = boot::limine::BaseRevision::new();
#[used]
#[link_section = ".requests"]
pub static MEMORY_MAP_REQUEST: boot::limine::MemoryMapRequest =
    boot::limine::MemoryMapRequest::new();
#[used]
#[link_section = ".requests"]
pub static HHDM_REQUEST: boot::limine::HhdmRequest = boot::limine::HhdmRequest::new();
#[used]
#[link_section = ".requests"]
pub static EXECUTABLE_CMDLINE_REQUEST: boot::limine::ExecutableCmdlineRequest =
    boot::limine::ExecutableCmdlineRequest::new();
#[used]
#[link_section = ".requests"]
pub static RSDP_REQUEST: boot::limine::RsdpRequest = boot::limine::RsdpRequest::new();
#[used]
#[link_section = ".requests"]
static MODULE_REQUEST: boot::limine::ModuleRequest = boot::limine::ModuleRequest::new();
#[used]
#[link_section = ".requests"]
static FRAMEBUFFER_REQUEST: boot::limine::FramebufferRequest =
    boot::limine::FramebufferRequest::new();
#[used]
#[link_section = ".requests"]
static STACK_SIZE_REQUEST: boot::limine::StackSizeRequest =
    boot::limine::StackSizeRequest::new().with_size(0x200000);
#[used]
#[link_section = ".requests_end_marker"]
static _REQUESTS_END: boot::limine::RequestsEndMarker = boot::limine::RequestsEndMarker::new();
#[no_mangle]
#[cfg(not(test))]
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
    kprintln!("        LixOS v0.0.2 (x86_64)           ");
    kprintln!("========================================");
    kprintln!("[lix] Arranque de kernel en Ring 0 exitoso.");
    kprintln!("[lix] Linker Higher-Half (-2 GB): 0xffffffff80000000");
    if let Some(hhdm) = HHDM_REQUEST.get_response() {
        kprintln!("[lix] HHDM offset físico: {:#x}", hhdm.offset());
    } else {
        kprintln!("[lix] AVISO: HHDM no provisto por el bootloader");
    }
    unsafe {
        crate::arch::x86_64::percpu::init_bsp();
    }
    memory::init();
    let rsdp_addr = RSDP_REQUEST
        .get_response()
        .map(|r| r.address() as u64)
        .unwrap_or(0);
    let topology = unsafe { crate::arch::x86_64::acpi::parse_madt(rsdp_addr) };
    crate::arch::x86_64::init();
    fs::init();
    drivers::pci::init();
    task::init();
    if let Some(topology) = topology {
        kprintln!(
            "[acpi] MADT detectada: {} CPUs, LAPIC base: {:#x}",
            topology.cpu_count,
            topology.lapic_phys_addr
        );
        unsafe {
            crate::arch::x86_64::smp::boot_secondary_cores(&topology);
        }
    } else {
        kprintln!("[acpi] AVISO: No se detecto tabla MADT, continuando en modo mononucleo");
    }
    if let Some(module_resp) = MODULE_REQUEST.get_response() {
        let modules = module_resp.modules();
        kprintln!("[lix] Módulos cargados por bootloader: {}", modules.len());
        for module in modules {
            let name = module.path().to_str().unwrap_or("unknown");
            kprintln!("[lix] Parseando módulo: {}", name);
            let data = unsafe {
                core::slice::from_raw_parts(module.addr() as *const u8, module.size() as usize)
            };
            let archive = fs::tar::TarArchive::new(data);
            let mut vfs = fs::vfs::VFS.lock();
            for file in archive.files() {
                if file.is_dir {
                    let _ = vfs.mkdir(file.name);
                } else {
                    kprintln!("  -> FS: {} ({} bytes)", file.name, file.size);
                    vfs.add_file(alloc::string::String::from(file.name), file.data);
                }
            }
        }
    } else {
        kprintln!("[lix] AVISO: No se encontraron módulos Initramfs.");
    }
    drivers::keyboard::load_config_from_vfs();
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
    let mut spawned_user_shell = false;
    let vanish_data = fs::vfs::read_file("/bin/apps/vanish.elf")
        .or_else(|| fs::vfs::read_file("/bin/userspace/vanish.elf"))
        .or_else(|| fs::vfs::read_file("/bin/vanish.elf"))
        .or_else(|| fs::vfs::read_file("/vanish.elf"));
    if let Some(data) = vanish_data {
        match crate::task::elf::load_elf(
            &data,
            "/bin/apps/vanish.elf",
            3,
            crate::task::rights::SESSION,
        ) {
            Ok(pid) => {
                kprintln!(
                    "[lix] Shell Vanish iniciada en Ring 3 (Userspace) con PID {}.",
                    pid
                );
                crate::task::scheduler::SHELL_PID.store(pid, core::sync::atomic::Ordering::Release);
                spawned_user_shell = true;
            }
            Err(e) => {
                kprintln!(
                    "[lix] AVISO: Error cargando vanish.elf en Ring 3: {}, usando fallback",
                    e
                );
            }
        }
    }
    if !spawned_user_shell {
        kprintln!("[lix] ERROR: vanish.elf no encontrado, iniciando bucle de espera.");
        let shell_pid = crate::task::scheduler::spawn_kernel_thread(
            crate::arch::x86_64::halt_loop,
            "idle_shell",
        );
        crate::task::scheduler::SHELL_PID.store(shell_pid, core::sync::atomic::Ordering::Release);
    }
    crate::task::scheduler::SCHEDULER.lock().started = true;
    crate::arch::x86_64::halt_loop();
}
