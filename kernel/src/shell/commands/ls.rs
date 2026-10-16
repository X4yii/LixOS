use super::super::Command;
pub struct LsCommand;
impl Command for LsCommand {
    fn name(&self) -> &str {
        "ls"
    }
    fn execute(&self, args: &[&str]) {
        let path = if args.is_empty() { "/" } else { args[0] };
        let files = crate::fs::vfs::list_files(path);
        if files.is_empty() {
            crate::kprintln!("ls: {}: vacío o no existe", path);
        } else {
            crate::kprintln!("Archivos en {}:", path);
            for file in files {
                crate::kprintln!("  - {}", file);
            }
        }
    }
}
