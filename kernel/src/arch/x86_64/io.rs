use core::arch::asm;
pub unsafe fn outb(port: u16, val: u8) {
    asm!(
        "out dx, al",
        in("dx") port,
        in("al") val,
        options(nomem, nostack, preserves_flags)
    );
}
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
pub unsafe fn outw(port: u16, val: u16) {
    asm!(
        "out dx, ax",
        in("dx") port,
        in("ax") val,
        options(nomem, nostack, preserves_flags)
    );
}
pub unsafe fn inw(port: u16) -> u16 {
    let val: u16;
    asm!(
        "in ax, dx",
        out("ax") val,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    val
}
pub unsafe fn io_wait() {
    outb(0x80, 0);
}
