use crate::autocomplete::resolve_path;
use crate::builtins::try_execute_builtin;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use sys_libc::strerror;
use sys_libc::syscalls::{
    sys_handle_close, sys_handle_dup, sys_handle_dup2, sys_lseek, sys_open, sys_spawn,
    sys_waitpid, sys_write,
};
use sys_libc::Handle;

fn parse_tokens(cmd_line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = ' ';
    for c in cmd_line.chars() {
        if in_quotes {
            if c == quote_char {
                in_quotes = false;
            } else {
                current.push(c);
            }
        } else if c == '"' || c == '\'' {
            in_quotes = true;
            quote_char = c;
        } else if c.is_whitespace() {
            if !current.is_empty() {
                tokens.push(current);
                current = String::new();
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

pub fn execute_command(cmd_line: &str, current_dir: &mut String) {
    let tokens = parse_tokens(cmd_line);
    if tokens.is_empty() {
        return;
    }
    let mut redirect_out: Option<(String, bool)> = None;
    let mut clean_tokens = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == ">>" {
            if i + 1 < tokens.len() {
                redirect_out = Some((tokens[i + 1].clone(), true));
                i += 2;
                continue;
            }
        } else if tokens[i] == ">" {
            if i + 1 < tokens.len() {
                redirect_out = Some((tokens[i + 1].clone(), false));
                i += 2;
                continue;
            }
        } else if let Some(target) = tokens[i].strip_prefix(">>") {
            if !target.is_empty() {
                redirect_out = Some((target.to_string(), true));
                i += 1;
                continue;
            }
        } else if let Some(target) = tokens[i].strip_prefix('>') {
            if !target.is_empty() {
                redirect_out = Some((target.to_string(), false));
                i += 1;
                continue;
            }
        }
        clean_tokens.push(tokens[i].clone());
        i += 1;
    }
    if clean_tokens.is_empty() {
        return;
    }
    let cmd = &clean_tokens[0];
    let args: Vec<&str> = clean_tokens[1..]
        .iter()
        .map(alloc::string::String::as_str)
        .collect();

    let redirection = if let Some((target_file, is_append)) = redirect_out {
        let resolved = resolve_path(current_dir, &target_file);
        let flags = 0x02 | 0x40;
        match sys_open(&resolved, flags) {
            Ok(file_handle) => {
                if is_append {
                    let _ = sys_lseek(file_handle, 0, 2);
                }
                match sys_handle_dup(Handle(1), 0) {
                    Ok(saved_stdout) => {
                        if sys_handle_dup2(file_handle, 1).is_ok() {
                            let _ = sys_handle_close(file_handle);
                            Some(saved_stdout)
                        } else {
                            let _ = sys_handle_close(saved_stdout);
                            let _ = sys_handle_close(file_handle);
                            sys_write("vanish: fallo al redirigir salida estándar\n");
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = sys_handle_close(file_handle);
                        sys_write(&alloc::format!(
                            "vanish: fallo al duplicar stdout: {} (código {})\n",
                            strerror(e),
                            e
                        ));
                        return;
                    }
                }
            }
            Err(e) => {
                sys_write(&alloc::format!(
                    "vanish: no se pudo abrir {}: {} (código {})\n",
                    resolved,
                    strerror(e),
                    e
                ));
                return;
            }
        }
    } else {
        None
    };

    let ran_builtin = try_execute_builtin(cmd, &args, current_dir);
    if !ran_builtin {
        let mut candidates = Vec::new();
        candidates.push(resolve_path(current_dir, cmd));
        candidates.push(resolve_path(current_dir, &alloc::format!("{cmd}.elf")));
        candidates.push(alloc::format!("/bin/apps/{cmd}.elf"));
        candidates.push(alloc::format!("/bin/system/{cmd}.elf"));
        candidates.push(alloc::format!("/bin/userspace/{cmd}.elf"));
        candidates.push(alloc::format!("/bin/{cmd}.elf"));
        candidates.push(alloc::format!("/{cmd}.elf"));
        candidates.push(alloc::format!("/bin/apps/{cmd}"));
        candidates.push(alloc::format!("/bin/system/{cmd}"));
        candidates.push(alloc::format!("/bin/userspace/{cmd}"));
        candidates.push(alloc::format!("/bin/{cmd}"));
        candidates.push(alloc::format!("/{cmd}"));
        candidates.push(cmd.clone());
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
            sys_write(&alloc::format!("vanish: comando no encontrado: {cmd}\n"));
        }
    }

    if let Some(saved_stdout) = redirection {
        let _ = sys_handle_dup2(saved_stdout, 1);
        let _ = sys_handle_close(saved_stdout);
    }
}
