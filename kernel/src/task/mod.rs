pub mod channel;
pub mod elf;
pub mod handle;
pub mod process;
pub mod rights;
pub mod scheduler;
pub mod syscall;
pub mod syscall_table;
pub static CANCEL_REQUEST: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
pub static FOREGROUND_TASK: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub fn init() {
    scheduler::init();
}
