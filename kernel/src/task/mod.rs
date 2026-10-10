pub mod process;
pub mod scheduler;
pub mod elf;
pub mod syscall;
pub mod syscall_table;
pub mod handle;

pub static CANCEL_REQUEST: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
pub static FOREGROUND_TASK: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub fn init() {
    scheduler::init();
}
