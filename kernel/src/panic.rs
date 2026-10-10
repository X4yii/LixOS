#[cfg(not(test))]
use core::panic::PanicInfo;
#[cfg(not(test))]
use crate::arch::x86_64::halt_loop;

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    unsafe {
        crate::klog::force_unlock();
    }

    crate::drivers::framebuffer::emergency_panic_screen();

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
