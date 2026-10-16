use crate::builtins::{COMMANDS, SYSTEM_BINARIES};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use sys_libc::syscalls::{sys_list_dir, sys_write};
pub fn resolve_path(cwd: &str, target: &str) -> String {
    let mut full = String::new();
    if target.starts_with('/') {
        full.push_str(target);
    } else if cwd == "/" {
        full.push('/');
        full.push_str(target);
    } else {
        full.push_str(cwd);
        full.push('/');
        full.push_str(target);
    }
    let mut components: Vec<&str> = Vec::new();
    for part in full.split('/') {
        if part.is_empty() || part == "." {
            continue;
        } else if part == ".." {
            components.pop();
        } else {
            components.push(part);
        }
    }
    if components.is_empty() {
        String::from("/")
    } else {
        let mut res = String::new();
        for c in components {
            res.push('/');
            res.push_str(c);
        }
        res
    }
}
pub fn handle_tab_completion(
    line_buffer: &mut String,
    cursor_idx: &mut usize,
    current_dir: &str,
    prompt: &str,
    refresh: impl Fn(&str, &str, usize),
) {
    let trimmed = line_buffer.trim_start();
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.is_empty() {
        sys_write("\n\x1b[33mComandos disponibles:\x1b[0m\n  ");
        for command in COMMANDS {
            sys_write(command.name);
            sys_write("  ");
        }
        for (name, _) in SYSTEM_BINARIES {
            sys_write(name);
            sys_write("  ");
        }
        sys_write("\n");
        refresh(prompt, line_buffer, *cursor_idx);
        return;
    }
    if parts.len() == 1 && !line_buffer.ends_with(' ') {
        let prefix = parts[0];
        let mut candidates: Vec<String> = Vec::new();
        for command in COMMANDS {
            if command.name.starts_with(prefix) {
                candidates.push(command.name.to_string());
            }
        }
        for (name, _) in SYSTEM_BINARIES {
            if name.starts_with(prefix) && !candidates.iter().any(|c| c.as_str() == *name) {
                candidates.push(name.to_string());
            }
        }
        for dir in &[
            current_dir,
            "/bin/apps",
            "/bin/system",
            "/bin/userspace",
            "/bin",
            "/",
        ] {
            let mut dir_buf = [0u8; 1024];
            if let Ok(bytes) = sys_list_dir(dir, &mut dir_buf) {
                if let Ok(listing) = core::str::from_utf8(&dir_buf[..bytes]) {
                    for file in listing.lines() {
                        let clean = file.strip_suffix(".elf").unwrap_or(file);
                        if clean.starts_with(prefix) && !candidates.iter().any(|c| c == clean) {
                            candidates.push(clean.to_string());
                        }
                    }
                }
            }
        }
        if candidates.len() == 1 {
            line_buffer.clear();
            line_buffer.push_str(&candidates[0]);
            line_buffer.push(' ');
            *cursor_idx = line_buffer.len();
            refresh(prompt, line_buffer, *cursor_idx);
        } else if candidates.len() > 1 {
            sys_write("\n\x1b[36mSugerencias:\x1b[0m\n  ");
            for c in &candidates {
                sys_write(c);
                sys_write("  ");
            }
            sys_write("\n");
            refresh(prompt, line_buffer, *cursor_idx);
        }
    } else {
        let last_arg = if line_buffer.ends_with(' ') {
            ""
        } else {
            parts.last().unwrap_or(&"")
        };
        let (dir_part, file_prefix) = if let Some(last_slash) = last_arg.rfind('/') {
            (&last_arg[..=last_slash], &last_arg[last_slash + 1..])
        } else {
            ("", last_arg)
        };
        let search_dir = resolve_path(
            current_dir,
            if dir_part.is_empty() { "." } else { dir_part },
        );
        let mut dir_buf = [0u8; 1024];
        let mut candidates: Vec<String> = Vec::new();
        if let Ok(bytes) = sys_list_dir(&search_dir, &mut dir_buf) {
            if let Ok(listing) = core::str::from_utf8(&dir_buf[..bytes]) {
                for file in listing.lines() {
                    if file.starts_with(file_prefix) && file != "." && file != ".." {
                        candidates.push(file.to_string());
                    }
                }
            }
        }
        if candidates.len() == 1 {
            let matched_name = &candidates[0];
            let completion = if dir_part.is_empty() {
                matched_name.clone()
            } else {
                alloc::format!("{dir_part}{matched_name}")
            };
            line_buffer.truncate(line_buffer.len().saturating_sub(last_arg.len()));
            line_buffer.push_str(&completion);
            *cursor_idx = line_buffer.len();
            refresh(prompt, line_buffer, *cursor_idx);
        } else if candidates.len() > 1 {
            sys_write("\n\x1b[36mArchivos:\x1b[0m\n  ");
            for c in &candidates {
                sys_write(c);
                sys_write("  ");
            }
            sys_write("\n");
            refresh(prompt, line_buffer, *cursor_idx);
        }
    }
}
