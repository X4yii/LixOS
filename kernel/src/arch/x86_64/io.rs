//! # Primitivas de E/S de Puertos x86_64

use core::arch::asm;

/// Escribe un byte en un puerto de E/S.
#[inline(always)]
pub unsafe fn outb(port: u16, val: u8) {
    asm!(
        "out dx, al",
        in("dx") port,
        in("al") val,
        options(nomem, nostack, preserves_flags)
    );
}

/// Lee un byte desde un puerto de E/S.
#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    asm!(
        "in al, dx",
        out("al") val,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    val
}

/// Pequeña pausa de E/S escribiendo en el puerto POST 0x80.
#[inline(always)]
pub unsafe fn io_wait() {
    outb(0x80, 0);
}
