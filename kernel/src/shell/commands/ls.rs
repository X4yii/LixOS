use super::super::Command;

pub struct LsCommand;

impl Command for LsCommand {
    fn name(&self) -> &str { "ls" }

    fn execute(&self, args: &[&str]) {
        let path = if args.is_empty() { "/" } else { args[0] };
        let vfs = crate::fs::vfs::VFS.lock();
        
        if let Some(fs) = &vfs.root_fs {
            if let Some(files) = fs.list_files(path) {
                crate::kprintln!("Archivos en disco ({}):", path);
                for f in files {
                    crate::kprintln!("  - {}", f);
                }
            } else {
                crate::kprintln!("ls: {}: No existe", path);
            }
        } else {
            crate::kprintln!("Archivos en disco RAM:");
            for file in vfs.get_files() {
                crate::kprintln!("  - {}", file.path);
            }
        }
    }
}
