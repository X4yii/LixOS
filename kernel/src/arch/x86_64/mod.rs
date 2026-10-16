pub mod acpi;
pub mod apic;
pub mod gdt;
pub mod idt;
pub mod io;
pub mod percpu;
pub mod pic;
pub mod smp;
pub mod tlb;
pub fn init() {
    unsafe {
        let mut cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0);
        cr0 &= !(1 << 2);
        cr0 |= (1 << 1) | (1 << 5);
        core::arch::asm!("mov cr0, {}", in(reg) cr0);
        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4);
        cr4 |= (1 << 9) | (1 << 10);
        core::arch::asm!("mov cr4, {}", in(reg) cr4);
    }
    gdt::init();
    idt::init();
    pic::init();
    unsafe {
        apic::init();
    }
    if !crate::drivers::ps2_keyboard::init() {
        crate::kprintln!("[ps2] Entrada PS/2 no disponible; se depende de USB HID u otro teclado");
    }
    unsafe {
        crate::task::syscall::init_syscall_msrs();
    }
    x86_64::instructions::interrupts::enable();
}
pub fn halt_loop() -> ! {
    loop {
        crate::drivers::virtio_net::poll();
        crate::drivers::net::poll_protocols();
        crate::drivers::xhci::poll();
        crate::drivers::serial::drain_to_stdin();
        x86_64::instructions::hlt();
    }
}
