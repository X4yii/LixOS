use alloc::collections::BTreeMap;
use crate::sync::IrqSafeSpinlock;
#[derive(Clone, Copy)]
pub struct IoWaiter {
    pub cid: u16,
    pub waiter_pid: u64,
    pub completed: bool,
    pub success: bool,
}
static WAITERS: IrqSafeSpinlock<BTreeMap<u16, IoWaiter>> = IrqSafeSpinlock::new(BTreeMap::new());
pub fn register_waiter(cid: u16, waiter_pid: u64) {
    let mut lock = WAITERS.lock();
    lock.insert(
        cid,
        IoWaiter {
            cid,
            waiter_pid,
            completed: false,
            success: false,
        },
    );
}
pub fn notify_completion(cid: u16, success: bool) {
    let mut pid_to_wake = 0u64;
    {
        let mut lock = WAITERS.lock();
        if let Some(waiter) = lock.get_mut(&cid) {
            waiter.completed = true;
            waiter.success = success;
            pid_to_wake = waiter.waiter_pid;
        } else {
            lock.insert(
                cid,
                IoWaiter {
                    cid,
                    waiter_pid: 0,
                    completed: true,
                    success,
                },
            );
        }
    }
    if pid_to_wake != 0 {
        crate::task::scheduler::wake_process(pid_to_wake);
    }
}
pub fn check_and_clear(cid: u16) -> Option<bool> {
    let mut lock = WAITERS.lock();
    if let Some(waiter) = lock.get(&cid) {
        if waiter.completed {
            let success = waiter.success;
            lock.remove(&cid);
            return Some(success);
        }
    }
    None
}
pub fn remove_waiter(cid: u16) {
    let mut lock = WAITERS.lock();
    lock.remove(&cid);
}
