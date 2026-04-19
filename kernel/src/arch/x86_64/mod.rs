pub mod io;
pub mod gdt;
pub mod idt;
pub mod pic;
pub mod apic;

pub fn init() {
    gdt::init();
    idt::init();
    pic::init();
    unsafe { apic::init(); }
    x86_64::instructions::interrupts::enable();
}

pub fn halt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}