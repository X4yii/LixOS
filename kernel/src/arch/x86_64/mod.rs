pub mod io;

/// Detiene la CPU indefinidamente en un bucle seguro.
#[inline(always)]
pub fn halt_loop() -> ! {
    loop {
        unsafe {
            core::arch::asm!("cli; hlt", options(nomem, nostack));
        }
    }
}
