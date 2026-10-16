#![no_std]
#![no_main]
#![allow(unsafe_code)]
extern crate alloc;
mod autocomplete;
mod builtins;
mod executor;
mod history;
mod line_editor;
use alloc::string::String;
use executor::execute_command;
use history::History;
use line_editor::{format_prompt, LineEditor};
use sys_libc::allocator::UserAllocator;
use sys_libc::syscalls::{sys_read, sys_write};
#[global_allocator]
static ALLOCATOR: UserAllocator = UserAllocator::new();
#[no_mangle]
pub extern "C" fn _start() -> ! {
    sys_write("\x1b[32;1mLixOS v0.0.2 - Vanish Shell (Ring 3 Userspace)\x1b[0m\n");
    sys_write("Escribe 'help' para ver la lista de comandos o usa [TAB] para autocompletar.\n\n");
    let mut current_dir = String::from("/");
    let mut history = History::new();
    let mut editor = LineEditor::new();
    sys_write(&format_prompt(&current_dir));
    loop {
        let mut char_buf = [0u8; 16];
        let n = sys_read(0, &mut char_buf).unwrap_or(0);
        if n == 0 {
            continue;
        }
        for i in 0..n {
            if let Some(cmd) = editor.handle_byte(char_buf[i], &current_dir, &mut history) {
                if !cmd.is_empty() {
                    history.push(&cmd);
                    execute_command(&cmd, &mut current_dir);
                }
                sys_write(&format_prompt(&current_dir));
            }
        }
    }
}
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    sys_write("\x1b[31m[vanish] Panic en Ring 3\x1b[0m\n");
    sys_libc::syscalls::sys_exit(1);
}
