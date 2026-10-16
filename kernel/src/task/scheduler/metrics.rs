use super::runqueue::RUNQUEUES;
use super::wait::SLEEPING;
use super::MAX_CPUS;
use crate::task::process::Process;
use core::sync::atomic::Ordering;
pub fn total_ticks() -> u64 {
    RUNQUEUES[0].lock().total_ticks
}
pub fn for_each_process<F: FnMut(&Process)>(mut f: F) {
    let online = crate::arch::x86_64::smp::ONLINE_CPUS.load(Ordering::Acquire) as usize;
    for id in 0..online.min(MAX_CPUS) {
        let rq = RUNQUEUES[id].lock();
        if let Some(ref curr) = rq.current {
            f(curr);
        }
        for q in &rq.queues {
            for proc in q {
                f(proc);
            }
        }
    }
    for proc in &*SLEEPING.lock() {
        f(proc);
    }
}
pub fn is_process_alive(pid: u64) -> bool {
    let online = crate::arch::x86_64::smp::ONLINE_CPUS.load(Ordering::Acquire) as usize;
    for id in 0..online.min(MAX_CPUS) {
        let rq = RUNQUEUES[id].lock();
        if rq.current.as_ref().map_or(false, |c| c.id.0 == pid) {
            return true;
        }
        for q in &rq.queues {
            if q.iter().any(|p| p.id.0 == pid) {
                return true;
            }
        }
    }
    SLEEPING.lock().iter().any(|p| p.id.0 == pid)
}
