pub mod commands;
use crate::sync::IrqSafeSpinlock;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use lazy_static::lazy_static;
pub trait Command {
    fn name(&self) -> &str;
    fn execute(&self, args: &[&str]);
}
pub use crate::task::{CANCEL_REQUEST, FOREGROUND_TASK};
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
    if cmd_line.is_empty() {
        return;
    }
    let mut parts = cmd_line.split_whitespace();
    let command_name = parts.next().unwrap();
    let args: Vec<&str> = parts.collect();
    if command_name == "sudo" || command_name == "vdo" || command_name == "su0" {
        crate::kprintln!(
            "{}: elevación deshabilitada; authd y los permisos temporales aún no están integrados",
            command_name
        );
        return;
    }
    let privilege = 3;
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
    let search_paths = VANISH_ENV.lock().paths.clone();
    let mut target_path = None;
    if command_name.starts_with("./") || command_name.starts_with("/") {
        target_path = Some(command_name.to_string());
    } else {
        for p in search_paths.iter() {
            let files = crate::fs::vfs::list_files(p);
            let cmd_elf = alloc::format!("{}.elf", command_name);
            let found_name = if files.iter().any(|file| file == command_name) {
                Some(command_name)
            } else if files.iter().any(|file| file == &cmd_elf) {
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
    if let Some(path) = target_path {
        if let Some(file_data) = crate::fs::vfs::read_file(&path) {
            match crate::task::elf::load_elf(
                &file_data,
                &path,
                privilege,
                crate::task::rights::SESSION,
            ) {
                Ok(pid) => {
                    FOREGROUND_TASK.store(pid, core::sync::atomic::Ordering::SeqCst);
                    loop {
                        if FOREGROUND_TASK.load(core::sync::atomic::Ordering::SeqCst) != pid {
                            break;
                        }
                        let is_alive = {
                            let sched = crate::task::scheduler::SCHEDULER.lock();
                            sched.is_process_alive(pid)
                        };
                        if !is_alive {
                            FOREGROUND_TASK.store(0, core::sync::atomic::Ordering::SeqCst);
                            break;
                        } else {
                            crate::task::scheduler::yield_current();
                        }
                    }
                }
                Err(e) => crate::kprintln!("Error cargando ELF: {}", e),
            }
        } else {
            crate::kprintln!("vanish: {}: Archivo no encontrado", path);
        }
    } else {
        crate::kprintln!("vanish: {}: comando no encontrado", command_name);
    }
}
