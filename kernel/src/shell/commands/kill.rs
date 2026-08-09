use super::super::Command;

pub struct KillCommand;

impl Command for KillCommand {
    fn name(&self) -> &str { "kill" }

    fn execute(&self, args: &[&str]) {
        if args.is_empty() {
            crate::kprintln!("Uso: kill [-9] <pid>");
            return;
        }

        let mut signal = 15;
        let pid_str = if args[0] == "-9" {
            if args.len() < 2 {
                crate::kprintln!("Falta el PID");
                return;
            }
            signal = 9;
            args[1]
        } else {
            args[0]
        };

        if let Ok(pid) = pid_str.parse::<u64>() {
            crate::task::scheduler::send_signal(crate::task::process::ProcessId(pid), signal);
            crate::kprintln!("Señal {} enviada al PID {}", signal, pid);
        } else {
            crate::kprintln!("PID inválido: {}", pid_str);
        }
    }
}
