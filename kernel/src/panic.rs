use core::panic::PanicInfo;
use crate::arch::x86_64::halt_loop;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    unsafe {
        crate::klog::force_unlock();
    }

    unsafe {
        crate::drivers::framebuffer::emergency_panic_screen();
    }

    kprintln!("\n========================================");
    kprintln!("          LIXOS KERNEL PANIC            ");
    kprintln!("========================================");

    if let Some(location) = info.location() {
        kprintln!(
            "Origen: {}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        );
    } else {
        kprintln!("Origen: Ubicación desconocida");
    }

    kprintln!("Detalle: {}", info.message());
    kprintln!("========================================");
    kprintln!("El sistema se ha detenido.");

    halt_loop()
}
