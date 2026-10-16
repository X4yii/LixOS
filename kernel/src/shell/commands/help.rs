use super::super::Command;
pub struct HelpCommand;
impl Command for HelpCommand {
    fn name(&self) -> &str {
        "help"
    }
    fn execute(&self, _args: &[&str]) {
        crate::kprintln!("Comandos internos de Vanish (Ring 0):");
        crate::kprintln!("  help, clear, ls, cat, ping, ltop, kill, pkill");
        crate::kprintln!("\nComandos disponibles en el PATH:");
        let paths = crate::shell::VANISH_ENV.lock().paths.clone();
        for path in paths {
            for file in crate::fs::vfs::list_files(&path) {
                if file != "." && file != ".." {
                    crate::kprintln!("  {} ({})", file, path);
                }
            }
        }
    }
}
