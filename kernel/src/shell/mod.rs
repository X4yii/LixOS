pub mod commands;

use alloc::string::{String, ToString};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use crate::sync::IrqSafeSpinlock;
use lazy_static::lazy_static;

pub trait Command {
    fn name(&self) -> &str;
    fn execute(&self, args: &[&str]);
}

pub static CANCEL_REQUEST: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
pub static FOREGROUND_TASK: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub struct VanishEnv {
    pub paths: Vec<String>,
    pub aliases: BTreeMap<String, String>,
}

lazy_static! {
    pub static ref VANISH_ENV: IrqSafeSpinlock<VanishEnv> = IrqSafeSpinlock::new(VanishEnv {
        paths: alloc::vec!["/bin".to_string(), "/usr/bin".to_string(), "./".to_string()],
        aliases: BTreeMap::new(),
    });
}

pub fn execute_line(cmd_line: &str) {
    CANCEL_REQUEST.store(false, core::sync::atomic::Ordering::SeqCst);
    FOREGROUND_TASK.store(0, core::sync::atomic::Ordering::SeqCst);

    let cmd_line = cmd_line.trim();
    if cmd_line.is_empty() { return; }

    let mut parts = cmd_line.split_whitespace();
    let mut command_name = parts.next().unwrap();
    let mut args: Vec<&str> = parts.collect();
    
    let mut privilege = 3;
    if command_name == "sudo" {
        privilege = 1;
        if args.is_empty() {
            crate::kprintln!("sudo: se requiere un comando");
            return;
        }
        command_name = args[0];
        args = args[1..].to_vec();
    } else if command_name == "su0" {
        privilege = 0;
        if args.is_empty() {
            crate::kprintln!("su0: se requiere un comando");
            return;
        }
        command_name = args[0];
        args = args[1..].to_vec();
    }

    // Builtins implementation mapping
    let builtins: [&dyn Command; 7] = [
        &commands::help::HelpCommand,
        &commands::clear::ClearCommand,
        &commands::ping::PingCommand,
        &commands::ls::LsCommand,
        &commands::cat::CatCommand,
        &commands::kill::KillCommand,
        &commands::pkill::PkillCommand,
    ];

    for builtin in builtins.iter() {
        if builtin.name() == command_name {
            builtin.execute(&args);
            return;
        }
    }

    // Try to resolve as an ELF executable
    let env = VANISH_ENV.lock();
    let mut target_path = None;

    if command_name.starts_with("./") || command_name.starts_with("/") {
        target_path = Some(command_name.to_string());
    } else {
        let vfs = crate::fs::vfs::VFS.lock();
        if let Some(fs) = &vfs.root_fs {
            for p in env.paths.iter() {
                if let Some(files) = fs.list_files(p) {
                    let cmd_elf = alloc::format!("{}.elf", command_name);
                    let found_name = if files.contains(&command_name.to_string()) {
                        Some(command_name)
                    } else if files.contains(&cmd_elf) {
                        Some(cmd_elf.as_str())
                    } else {
                        None
                    };

                    if let Some(name) = found_name {
                        let base_p = if p == "/" { "" } else { p.as_str() };
                        target_path = Some(alloc::format!("{}/{}", base_p, name));
                        break;
                    }
                }
            }
        }
    }

    if let Some(path) = target_path {
        let vfs = crate::fs::vfs::VFS.lock();
        if let Some(file_data) = vfs.read_file(&path) {
            match crate::task::elf::load_elf(&file_data, &path, privilege) {
                Ok(pid) => {
                    FOREGROUND_TASK.store(pid, core::sync::atomic::Ordering::SeqCst);
                    loop {
                        if FOREGROUND_TASK.load(core::sync::atomic::Ordering::SeqCst) != pid { break; }
                        let is_alive = {
                            let sched = crate::task::scheduler::SCHEDULER.lock();
                            let mut found = false;
                            for p in sched.processes.iter() {
                                if p.id.0 == pid { found = true; break; }
                            }
                            if let Some(ref p) = sched.current {
                                if p.id.0 == pid { found = true; }
                            }
                            found
                        };
                        if !is_alive {
                            FOREGROUND_TASK.store(0, core::sync::atomic::Ordering::SeqCst);
                            break;
                        } else {
                            crate::task::scheduler::yield_current();
                        }
                    }
                },
                Err(e) => crate::kprintln!("Error cargando ELF: {}", e),
            }
        } else {
            crate::kprintln!("vanish: {}: Archivo no encontrado", path);
        }
    } else {
        crate::kprintln!("vanish: {}: comando no encontrado", command_name);
    }
}
