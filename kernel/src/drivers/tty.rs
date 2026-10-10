use alloc::collections::VecDeque;
use crate::sync::IrqSafeSpinlock;
use lazy_static::lazy_static;
use core::sync::atomic::Ordering;

lazy_static! {
    pub static ref STDIN: IrqSafeSpinlock<VecDeque<char>> = IrqSafeSpinlock::new(VecDeque::new());
}

/// Disciplina de línea y buffer de entrada TTY del sistema operativo.
/// Gestiona la recepción de caracteres desde los controladores de hardware (PS/2, Serial),
/// procesa señales de control estándar (ISIG - VINTR Ctrl+C) sobre el proceso en primer plano,
/// y despierta tareas en espera de entrada.
pub fn push_char(c: char) -> bool {
    if c == '\x03' {
        let fg = crate::task::FOREGROUND_TASK.load(Ordering::SeqCst);
        if fg != 0 {
            crate::task::scheduler::send_signal(crate::task::process::ProcessId(fg), 2);
            crate::task::CANCEL_REQUEST.store(true, Ordering::SeqCst);
            return true;
        }
    }

    STDIN.lock().push_back(c);
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
