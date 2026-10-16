#![no_std]
#![no_main]
#![allow(unsafe_code)]
extern crate alloc;
use sys_libc::allocator::UserAllocator;
use sys_libc::syscalls::sys_exit;
#[global_allocator]
static ALLOCATOR: UserAllocator = UserAllocator::new();
#[no_mangle]
pub extern "C" fn _start() -> ! {
    lix_edit::run_editor("/home/user/notas.txt");
    sys_exit(0);
}
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    lix_edit::restore_terminal();
    sys_libc::println!("[edit] {}", info);
    sys_exit(1);
}
