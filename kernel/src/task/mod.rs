pub mod process;
pub mod scheduler;
pub mod elf;
pub mod syscall;

pub fn init() {
    scheduler::init();
}
