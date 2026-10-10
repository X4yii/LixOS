use alloc::string::{String, ToString};
use alloc::vec::Vec;
use sys_libc::syscalls::{sys_spawn, sys_waitpid, sys_write};
use crate::autocomplete::resolve_path;
use crate::builtins::try_execute_builtin;

pub fn execute_command(cmd_line: &str, current_dir: &mut String) {
    let mut parts = cmd_line.split_whitespace();
    let cmd = match parts.next() {
        Some(c) => c,
        None => return,
    };
    let args: Vec<&str> = parts.collect();

    if try_execute_builtin(cmd, &args, current_dir) {
        return;
    }

    let mut candidates = Vec::new();
    candidates.push(resolve_path(current_dir, cmd));
    candidates.push(resolve_path(current_dir, &alloc::format!("{}.elf", cmd)));
    candidates.push(alloc::format!("/bin/{}.elf", cmd));
    candidates.push(alloc::format!("/{}.elf", cmd));
    candidates.push(alloc::format!("/bin/{}", cmd));
    candidates.push(alloc::format!("/{}", cmd));
    candidates.push(cmd.to_string());

    let mut executed = false;
    for path in candidates {
        if let Ok(pid) = sys_spawn(&path) {
            executed = true;
            let mut status = 0i32;
            let _ = sys_waitpid(pid, &mut status);
            break;
        }
    }

    if !executed {
        sys_write(&alloc::format!("vanish: comando no encontrado: {}\n", cmd));
    }
}
