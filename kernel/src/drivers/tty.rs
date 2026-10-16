use crate::sync::IrqSafeSpinlock;
use alloc::collections::VecDeque;
use core::sync::atomic::Ordering;
use lazy_static::lazy_static;
const STDIN_LIMIT: usize = 1024;
lazy_static! {
    pub static ref STDIN: IrqSafeSpinlock<VecDeque<char>> = IrqSafeSpinlock::new(VecDeque::new());
}
pub fn push_char(c: char) -> bool {
    if c == '\x03' {
        let fg = crate::task::FOREGROUND_TASK.load(Ordering::SeqCst);
        if fg != 0 {
            crate::task::scheduler::send_signal(crate::task::process::ProcessId(fg), 2);
            crate::task::CANCEL_REQUEST.store(true, Ordering::SeqCst);
            return true;
        }
    }
    let mut stdin = STDIN.lock();
    if stdin.len() == STDIN_LIMIT {
        stdin.pop_front();
    }
    stdin.push_back(c);
    drop(stdin);
    crate::task::scheduler::wake_stdin_waiters()
}
pub fn push_str(s: &str) -> bool {
    let mut woke_waiter = false;
    for c in s.chars() {
        woke_waiter |= push_char(c);
    }
    woke_waiter
}
pub fn pop_char() -> Option<char> {
    STDIN.lock().pop_front()
}
