use super::runqueue::{enqueue_process, PerCpuRunqueue, RUNQUEUES};
use super::MAX_CPUS;
use crate::sync::IrqSafeSpinlock;
use crate::task::process::{Process, ProcessId, ProcessState};
use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicU64, Ordering};
pub static SLEEPING: IrqSafeSpinlock<VecDeque<Process>> = IrqSafeSpinlock::new(VecDeque::new());
pub static ZOMBIES: IrqSafeSpinlock<VecDeque<Process>> = IrqSafeSpinlock::new(VecDeque::new());
pub static REAPER_PID: AtomicU64 = AtomicU64::new(0);
pub static SHELL_PID: AtomicU64 = AtomicU64::new(0);
static EXIT_STATUSES: IrqSafeSpinlock<VecDeque<(u64, i32)>> = IrqSafeSpinlock::new(VecDeque::new());
const MAX_RETAINED_EXIT_STATUSES: usize = 256;
pub fn record_exit_status(pid: u64, code: i32) {
    let mut lock = EXIT_STATUSES.lock();
    if let Some(entry) = lock.iter_mut().find(|entry| entry.0 == pid) {
        entry.1 = code;
        return;
    }
    if lock.len() >= MAX_RETAINED_EXIT_STATUSES {
        lock.pop_front();
    }
    lock.push_back((pid, code));
}
pub fn take_exit_status(pid: u64) -> Option<i32> {
    let mut lock = EXIT_STATUSES.lock();
    if let Some(pos) = lock.iter().position(|&(p, _)| p == pid) {
        Some(lock.remove(pos).unwrap().1)
    } else {
        None
    }
}
pub fn wake_process(pid: u64) {
    let proc = {
        let mut sleeping = SLEEPING.lock();
        if let Some(pos) = sleeping.iter().position(|p| p.id.0 == pid) {
            let mut proc = sleeping.remove(pos).unwrap();
            proc.state = ProcessState::Ready;
            Some(proc)
        } else {
            None
        }
    };
    if let Some(proc) = proc {
        enqueue_process(proc);
    }
    super::with_current_process(|current| {
        if current.id.0 == pid && current.state == ProcessState::Blocked {
            current.state = ProcessState::Ready;
        }
    });
}
pub fn wake_waiters_for(exited_pid: ProcessId) {
    let mut waiters = alloc::vec::Vec::new();
    {
        let mut sleeping = SLEEPING.lock();
        let mut i = 0;
        while i < sleeping.len() {
            if sleeping[i].waiting_for == Some(exited_pid) {
                let mut proc = sleeping.remove(i).unwrap();
                proc.waiting_for = None;
                proc.state = ProcessState::Ready;
                waiters.push(proc);
                continue;
            }
            i += 1;
        }
    }
    for proc in waiters {
        enqueue_process(proc);
    }
    super::with_current_process(|current| {
        if current.waiting_for == Some(exited_pid) {
            current.waiting_for = None;
            current.state = ProcessState::Ready;
        }
    });
}
pub fn wake_stdin_waiters() -> bool {
    let fg = crate::shell::FOREGROUND_TASK.load(Ordering::SeqCst);
    let s_pid = SHELL_PID.load(Ordering::Relaxed);
    let mut to_wake = alloc::vec::Vec::new();
    let mut woke_waiter = false;
    {
        let mut sleeping = SLEEPING.lock();
        let mut i = 0;
        while i < sleeping.len() {
            let target_pid = sleeping[i].id.0;
            if target_pid == fg || (fg == 0 && target_pid == s_pid) {
                let mut proc = sleeping.remove(i).unwrap();
                proc.state = ProcessState::Ready;
                to_wake.push(proc);
                woke_waiter = true;
                continue;
            }
            i += 1;
        }
    }
    for proc in to_wake {
        enqueue_process(proc);
    }
    super::with_current_process(|current| {
        if current.id.0 == fg || (fg == 0 && current.id.0 == s_pid) {
            if matches!(
                current.state,
                ProcessState::Blocked | ProcessState::Sleeping(_)
            ) {
                current.state = ProcessState::Ready;
                woke_waiter = true;
            }
        }
    });
    woke_waiter
}
pub(super) fn wake_expired_sleeping_processes(ticks: u64) {
    let mut woken = alloc::vec::Vec::new();
    {
        let mut sleeping = SLEEPING.lock();
        let mut i = 0;
        while i < sleeping.len() {
            let should_wake = match sleeping[i].state {
                ProcessState::Sleeping(w) => ticks >= w,
                _ => false,
            };
            if should_wake {
                let mut proc = sleeping.remove(i).unwrap();
                proc.state = ProcessState::Ready;
                woken.push(proc);
                continue;
            }
            i += 1;
        }
    }
    for proc in woken {
        enqueue_process(proc);
    }
}
pub(super) fn wake_reaper_if_needed() {
    if !ZOMBIES.lock().is_empty() {
        let r_pid = REAPER_PID.load(Ordering::Relaxed);
        if r_pid != 0 {
            wake_process(r_pid);
        }
    }
}
pub(super) fn wake_reaper_into(rq: &mut PerCpuRunqueue) {
    let r_pid = REAPER_PID.load(Ordering::Relaxed);
    if r_pid != 0 {
        let mut sleeping = SLEEPING.lock();
        if let Some(pos) = sleeping.iter().position(|p| p.id.0 == r_pid) {
            let reaper = sleeping.remove(pos).unwrap();
            rq.enqueue(reaper);
        }
    }
}
pub fn destroy_process(proc: &Process) {
    let r_pid = REAPER_PID.load(Ordering::Relaxed);
    if proc.id.0 == 1 || (r_pid != 0 && proc.id.0 == r_pid) {
        crate::kprintln!(
            "AVISO: Intento de destruir proceso critico de Ring 0 (PID {})",
            proc.id.0
        );
        return;
    }
    crate::kprintln!(
        "destroy_process: Liberando recursos PID {} ({})",
        proc.id.0,
        proc.name
    );
    if proc.kernel_stack_ptr != 0 {
        unsafe {
            let layout =
                core::alloc::Layout::from_size_align(super::KERNEL_STACK_SIZE, 16).unwrap();
            alloc::alloc::dealloc(proc.kernel_stack_ptr as *mut u8, layout);
        }
    }
    if proc.pml4_frame != 0 {
        unsafe {
            crate::memory::vmm::destroy_userspace_pml4(proc.pml4_frame);
        }
    }
    crate::kprintln!(
        "destroy_process: Proceso PID {} destruido completamente",
        proc.id.0
    );
}
pub fn reaper_thread() -> ! {
    loop {
        let dead_proc = ZOMBIES.lock().pop_front();
        if let Some(proc) = dead_proc {
            let pid = proc.id;
            record_exit_status(pid.0, proc.exit_code);
            destroy_process(&proc);
            wake_waiters_for(pid);
        } else {
            super::block_current_and_yield();
        }
    }
}
pub fn terminate_user_processes() {
    let online = crate::arch::x86_64::smp::ONLINE_CPUS.load(Ordering::Acquire) as usize;
    for id in 0..online.min(MAX_CPUS) {
        let mut rq = RUNQUEUES[id].lock();
        for q in 0..32 {
            let mut i = 0;
            while i < rq.queues[q].len() {
                if rq.queues[q][i].id.0 > 2 {
                    let proc = rq.queues[q].remove(i).unwrap();
                    destroy_process(&proc);
                } else {
                    i += 1;
                }
            }
            if rq.queues[q].is_empty() {
                rq.active_bitmap &= !(1 << q);
            }
        }
    }
    let mut sleeping = SLEEPING.lock();
    let mut i = 0;
    while i < sleeping.len() {
        if sleeping[i].id.0 > 2 {
            let proc = sleeping.remove(i).unwrap();
            destroy_process(&proc);
        } else {
            i += 1;
        }
    }
    let mut zombies = ZOMBIES.lock();
    let mut i = 0;
    while i < zombies.len() {
        if zombies[i].id.0 > 2 {
            let proc = zombies.remove(i).unwrap();
            destroy_process(&proc);
        } else {
            i += 1;
        }
    }
}
