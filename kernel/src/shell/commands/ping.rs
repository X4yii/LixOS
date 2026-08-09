use super::super::Command;

pub struct PingCommand;

impl Command for PingCommand {
    fn name(&self) -> &str { "ping" }

    fn execute(&self, _args: &[&str]) {
        crate::kprintln!("pong!");
    }
}
