use super::super::Command;

pub struct CatCommand;

impl Command for CatCommand {
    fn name(&self) -> &str { "cat" }

    fn execute(&self, args: &[&str]) {
        if args.is_empty() {
            crate::kprintln!("Uso: cat <archivo>");
            return;
        }
        if let Some(content) = crate::fs::vfs::read_file(args[0]) {
            if let Ok(text) = core::str::from_utf8(&content) {
                crate::kprintln!("{}", text);
            } else {
                crate::kprintln!("<datos binarios>");
            }
        } else {
            crate::kprintln!("cat: {}: No existe el archivo", args[0]);
        }
    }
}
