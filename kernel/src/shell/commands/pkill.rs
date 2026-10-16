use super::super::Command;
pub struct PkillCommand;
impl Command for PkillCommand {
    fn name(&self) -> &str {
        "pkill"
    }
    fn execute(&self, args: &[&str]) {
        if args.is_empty() {
            crate::kprintln!("Uso: pkill <name>");
            return;
        }
        let name = args[0];
        let mut count = 0;
        let pids: alloc::vec::Vec<_> = {
            let sched = crate::task::scheduler::SCHEDULER.lock();
            let mut matched = alloc::vec::Vec::new();
            sched.for_each_process(|proc| {
                if proc.name.contains(name) {
                    matched.push(proc.id);
                }
            });
            matched
        };
        for pid in pids {
            crate::task::scheduler::send_signal(pid, 15);
            count += 1;
        }
        crate::kprintln!("Señal 15 enviada a {} procesos", count);
    }
}
