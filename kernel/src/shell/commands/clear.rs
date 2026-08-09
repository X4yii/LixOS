use super::super::Command;

pub struct ClearCommand;

impl Command for ClearCommand {
    fn name(&self) -> &str { "clear" }

    fn execute(&self, _args: &[&str]) {
        unsafe { crate::drivers::framebuffer::clear_screen(); }
    }
}
