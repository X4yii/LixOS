use crate::autocomplete::resolve_path;
use alloc::string::String;
use sys_libc::strerror;
use sys_libc::syscalls::{sys_list_dir, sys_read_file_all, sys_write, sys_write_file};
pub fn cd(args: &[&str], current_dir: &mut String) -> bool {
    let target = match args.first().copied() {
        None | Some("~") => {
            let mut test_buf = [0u8; 1024];
            if sys_list_dir("/home/user", &mut test_buf).is_ok() {
                "/home/user"
            } else if sys_list_dir("/home", &mut test_buf).is_ok() {
                "/home"
            } else {
                "/"
            }
        }
        Some(p) => p,
    };
    let resolved = resolve_path(current_dir, target);
    let mut test_buf = [0u8; 1024];
    if resolved == "/" || sys_list_dir(&resolved, &mut test_buf).is_ok() {
        *current_dir = resolved;
    } else {
        sys_write(&alloc::format!("cd: no existe el directorio: {target}\n"));
    }
    true
}
pub fn pwd(_args: &[&str], current_dir: &mut String) -> bool {
    sys_write(current_dir);
    sys_write("\n");
    true
}
pub fn mkdir(args: &[&str], current_dir: &mut String) -> bool {
    if args.is_empty() {
        sys_write("Uso: mkdir [-p] <directorio>...\n");
    } else {
        let mut parents = false;
        let mut dirs = alloc::vec::Vec::new();
        for arg in args {
            if *arg == "-p" {
                parents = true;
            } else if !arg.starts_with('-') {
                dirs.push(*arg);
            }
        }
        if dirs.is_empty() {
            sys_write("mkdir: falta el operando de directorio\n");
        }
        for dir in dirs {
            let resolved = resolve_path(current_dir, dir);
            if parents {
                let mut current_sub = alloc::string::String::new();
                for part in resolved.split('/') {
                    if part.is_empty() {
                        continue;
                    }
                    current_sub.push('/');
                    current_sub.push_str(part);
                    let _ = sys_libc::syscalls::sys_mkdir(&current_sub);
                }
            } else if let Err(e) = sys_libc::syscalls::sys_mkdir(&resolved) {
                sys_write(&alloc::format!(
                    "mkdir: no se pudo crear el directorio '{}': {} (código {})\n",
                    dir,
                    strerror(e),
                    e
                ));
            }
        }
    }
    true
}
pub fn rm(args: &[&str], current_dir: &mut String) -> bool {
    if args.is_empty() {
        sys_write("Uso: rm [-r|-R|-f|-rf] <archivo|directorio>...\n");
    } else {
        let mut recursive = false;
        let mut force = false;
        let mut targets = alloc::vec::Vec::new();
        for arg in args {
            if *arg == "-r" || *arg == "-R" {
                recursive = true;
            } else if *arg == "-f" {
                force = true;
            } else if *arg == "-rf" || *arg == "-fr" || *arg == "-Rf" || *arg == "-fR" {
                recursive = true;
                force = true;
            } else if !arg.starts_with('-') {
                targets.push(*arg);
            }
        }
        if targets.is_empty() {
            sys_write("rm: falta el operando\n");
        }
        for target in targets {
            let resolved = resolve_path(current_dir, target);
            if let Err(e) = sys_libc::syscalls::sys_remove_file(&resolved, recursive) {
                if !force {
                    sys_write(&alloc::format!(
                        "rm: no se pudo borrar '{}': {} (código {})\n",
                        target,
                        strerror(e),
                        e
                    ));
                }
            }
        }
    }
    true
}
pub fn edit(args: &[&str], current_dir: &mut String) -> bool {
    let target = args.first().copied().unwrap_or("/home/user/nuevo.txt");
    let resolved = resolve_path(current_dir, target);
    lix_edit::run_editor(&resolved);
    true
}
pub fn write(args: &[&str], current_dir: &mut String) -> bool {
    if args.is_empty() {
        sys_write("Uso: write <archivo> [texto]\n");
    } else {
        let target = args[0];
        let resolved = resolve_path(current_dir, target);
        let text = if args.len() > 1 {
            let mut s = alloc::string::String::new();
            for (i, a) in args[1..].iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                s.push_str(a);
            }
            s.push('\n');
            s
        } else {
            alloc::string::String::from("\n")
        };
        match sys_write_file(&resolved, text.as_bytes()) {
            Ok(bytes) => {
                sys_write(&alloc::format!("Escritos {bytes} bytes en {resolved}\n"));
            }
            Err(e) => {
                sys_write(&alloc::format!(
                    "write: no se pudo escribir en {}: {} (código {})\n",
                    resolved,
                    strerror(e),
                    e
                ));
            }
        }
    }
    true
}
pub fn ls(args: &[&str], current_dir: &mut String) -> bool {
    let raw_path = args.first().copied().unwrap_or(".");
    let target_path = resolve_path(current_dir, raw_path);
    let mut buf = [0u8; 1024];
    match sys_list_dir(&target_path, &mut buf) {
        Ok(bytes) => {
            if bytes == 0 {
                sys_write("(directorio vacío)\n");
            } else if let Ok(listing) = core::str::from_utf8(&buf[..bytes]) {
                for line in listing.lines() {
                    if line.is_empty() {
                        continue;
                    }
                    if line.ends_with(".elf") {
                        sys_write("\x1b[32;1m");
                        sys_write(line);
                        sys_write("\x1b[0m\n");
                    } else if line.ends_with('/') || !line.contains('.') {
                        sys_write("\x1b[34;1m");
                        sys_write(line);
                        sys_write("\x1b[0m\n");
                    } else {
                        sys_write(line);
                        sys_write("\n");
                    }
                }
            }
        }
        Err(e) => {
            sys_write(&alloc::format!(
                "ls: no se pudo listar '{}': {} (código {})\n",
                target_path,
                strerror(e),
                e
            ));
        }
    }
    true
}
pub fn cat(args: &[&str], current_dir: &mut String) -> bool {
    if args.is_empty() {
        sys_write("Uso: cat <archivo>\n");
    } else {
        let file_path = resolve_path(current_dir, args[0]);
        match sys_read_file_all(&file_path) {
            Ok(bytes) => {
                if let Ok(content) = core::str::from_utf8(&bytes) {
                    sys_write(content);
                    if !content.ends_with('\n') {
                        sys_write("\n");
                    }
                } else {
                    sys_write(&alloc::format!(
                        "cat: archivo binario ({} bytes)\n",
                        bytes.len()
                    ));
                }
            }
            Err(e) => {
                sys_write(&alloc::format!(
                    "cat: no se pudo leer '{}': {} (código {})\n",
                    file_path,
                    strerror(e),
                    e
                ));
            }
        }
    }
    true
}
