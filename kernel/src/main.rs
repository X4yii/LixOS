//! # LixOS Kernel (Ring 0)
//!
//! Punto de entrada del kernel monolítico asíncrono para x86_64.

#![no_std]
#![no_main]

pub mod arch;
pub mod boot;
pub mod drivers;
#[macro_use]
pub mod klog;
pub mod panic;

// Marcadores de sección exigidos por el protocolo Limine
#[used]
#[link_section = ".requests_start_marker"]
static _REQUESTS_START: () = ();

#[used]
#[link_section = ".requests"]
static BASE_REVISION: boot::limine::BaseRevision = boot::limine::BaseRevision::new(2);

#[used]
#[link_section = ".requests"]
static HHDM_REQUEST: boot::limine::HhdmRequest = boot::limine::HhdmRequest::new();

#[used]
#[link_section = ".requests"]
static FRAMEBUFFER_REQUEST: boot::limine::FramebufferRequest = boot::limine::FramebufferRequest::new();

#[used]
#[link_section = ".requests_end_marker"]
static _REQUESTS_END: () = ();

/// Punto de entrada inicial del kernel transferido desde el bootloader.
#[no_mangle]
pub extern "C" fn _start() -> ! {
    // 1. Inicializar salida serial inmediata (UART 16550 COM1 en 0x3F8)
    drivers::serial::init_serial();

    kprintln!("========================================");
    kprintln!("        LixOS v0.0.1 (x86_64)           ");
    kprintln!("========================================");
    kprintln!("[lix] Arranque de kernel en Ring 0 exitoso.");
    kprintln!("[lix] Linker Higher-Half (-2 GB): 0xffffffff80000000");

    // 2. Verificar HHDM (Higher-Half Direct Map)
    if let Some(hhdm) = HHDM_REQUEST.get_response() {
        kprintln!("[lix] HHDM offset físico: {:#x}", hhdm.offset);
    } else {
        kprintln!("[lix] AVISO: HHDM no provisto por el bootloader");
    }

    // 3. Inspeccionar Framebuffer
    if let Some(fb_resp) = FRAMEBUFFER_REQUEST.get_response() {
        kprintln!("[lix] Framebuffers detectados: {}", fb_resp.framebuffer_count);
        if fb_resp.framebuffer_count > 0 && !fb_resp.framebuffers.is_null() {
            let fb = unsafe { &**fb_resp.framebuffers };
            kprintln!(
                "[lix] Resolución principal: {}x{} @ {} bpp (pitch: {} bytes)",
                fb.width,
                fb.height,
                fb.bpp,
                fb.pitch
            );
        }
    }

    kprintln!("[lix] Inicialización temprana de M0 completada.");
    kprintln!("[lix] Entrando en reposo seguro (HLT loop)...");

    arch::x86_64::halt_loop()
}
