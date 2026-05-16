use alloc::string::{String, ToString};
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use crate::sync::IrqSafeSpinlock;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref STDIN: IrqSafeSpinlock<VecDeque<char>> = IrqSafeSpinlock::new(VecDeque::new());
}

pub fn run_kernel_shell() -> ! {
    unsafe { crate::drivers::framebuffer::clear_screen(); }
    crate::kprintln!("\x1b[32mLixOS v0.0.1 - Terminal TTY1 (Vanish Ring 0)\x1b[0m");
    crate::kprint!("\x1b[36mvanish>\x1b[0m ");

    let mut input_buffer = String::new();
    let mut history: Vec<String> = Vec::new();
    let mut history_index = 0;

    crate::shell::execute_line("ltop");
    
    loop {
        let c = {
            let mut queue = STDIN.lock();
            queue.pop_front()
        };

        if let Some(c) = c {
            match c {
                '\n' => {
                    crate::kprint!("\n");
                    let cmd_trim = input_buffer.trim().to_string();
                    if !cmd_trim.is_empty() {
                        if history.is_empty() || history.last().unwrap() != &cmd_trim {
                            history.push(cmd_trim.clone());
                        }
                        history_index = history.len();
                        crate::shell::execute_line(&cmd_trim);
                    }
                    input_buffer.clear();
                    crate::kprint!("\x1b[36mvanish>\x1b[0m ");
                }
                '\x08' => {
                    if !input_buffer.is_empty() {
                        input_buffer.pop();
                        crate::kprint!("\x08");
                    }
                }
                '\x03' => { // Ctrl+C
                    let fg = crate::shell::FOREGROUND_TASK.load(core::sync::atomic::Ordering::SeqCst);
                    if fg != 0 {
                        crate::task::scheduler::send_signal(crate::task::process::ProcessId(fg), 2);
                    }
                    crate::shell::CANCEL_REQUEST.store(true, core::sync::atomic::Ordering::SeqCst);
                    crate::kprint!("^C");
                    
                    input_buffer.clear();
                    crate::kprint!("\n\x1b[36mvanish>\x1b[0m ");
                }
                '\x17' => { // Ctrl+W (Shift+Backspace mapped here)
                    while input_buffer.ends_with(' ') {
                        input_buffer.pop();
                        crate::kprint!("\x08");
                    }
                    while let Some(ch) = input_buffer.chars().last() {
                        if ch == ' ' { break; }
                        input_buffer.pop();
                        crate::kprint!("\x08");
                    }
                }
                '\x11' => { // Arrow Up
                    if history_index > 0 {
                        while !input_buffer.is_empty() {
                            input_buffer.pop();
                            crate::kprint!("\x08");
                        }
                        history_index -= 1;
                        input_buffer = history[history_index].clone();
                        crate::kprint!("{}", input_buffer);
                    }
                }
                '\x12' => { // Arrow Down
                    if history_index < history.len() {
                        while !input_buffer.is_empty() {
                            input_buffer.pop();
                            crate::kprint!("\x08");
                        }
                        history_index += 1;
                        if history_index < history.len() {
                            input_buffer = history[history_index].clone();
                            crate::kprint!("{}", input_buffer);
                        }
                    }
                }
                '\t' => { // Tab
                    let env = crate::shell::VANISH_ENV.lock();
                    let vfs = crate::fs::vfs::VFS.lock();
                    if let Some(fs) = &vfs.root_fs {
                        let mut matches = Vec::new();
                        for path in env.paths.iter() {
                            if let Some(files) = fs.list_files(path) {
                                for f in files {
                                    let base_name = if f.ends_with(".elf") { &f[..f.len()-4] } else { f.as_str() };
                                    if base_name.starts_with(&input_buffer) && !matches.contains(&base_name.to_string()) {
                                        matches.push(base_name.to_string());
                                    }
                                }
                            }
                        }
                        if matches.len() == 1 {
                            let completion = &matches[0][input_buffer.len()..];
                            input_buffer.push_str(completion);
                            crate::kprint!("{}", completion);
                        } else if matches.len() > 1 {
                            crate::kprintln!("");
                            for m in matches {
                                crate::kprint!("\x1b[32m{}\x1b[0m  ", m);
                            }
                            crate::kprintln!("");
                            crate::kprint!("\x1b[36mvanish>\x1b[0m {}", input_buffer);
                        }
                    }
                }
                _ => {
                    // Ignoramos caracteres no imprimibles raros
                    if c >= ' ' && c <= '~' {
                        input_buffer.push(c);
                        crate::kprint!("{}", c);
                    }
                }
            }
        } else {
            crate::task::scheduler::block_current_and_yield();
        }
    }
}