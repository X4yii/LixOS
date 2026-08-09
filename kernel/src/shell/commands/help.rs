use super::super::Command;

pub struct HelpCommand;

impl Command for HelpCommand {
    fn name(&self) -> &str { "help" }

    fn execute(&self, _args: &[&str]) {
        crate::kprintln!("Comandos internos de Vanish (Ring 0):");
        crate::kprintln!("  help, clear, ls, cat, ping, ltop, kill, pkill");
        
        crate::kprintln!("\nComandos disponibles en el PATH:");
        let env = crate::shell::VANISH_ENV.lock();
        let vfs = crate::fs::vfs::VFS.lock();
        if let Some(fs) = &vfs.root_fs {
            for path in env.paths.iter() {
                if let Some(files) = fs.list_files(path) {
                    for f in files {
                        if f != "." && f != ".." {
                            crate::kprintln!("  {} ({})", f, path);
                        }
                    }
                }
            }
        }
    }
}
