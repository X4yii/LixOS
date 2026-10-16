mod metrics;
mod runqueue;
mod spawn;
mod wait;
use super::process::{Process, ProcessState};
use core::sync::atomic::{AtomicBool, Ordering};
pub use metrics::{for_each_process, is_process_alive, total_ticks};
pub use runqueue::{enqueue_process, try_steal_task, PerCpuRunqueue, RUNQUEUES};
pub use spawn::{send_signal, spawn_idle_for_cpu, spawn_kernel_thread, spawn_user_process};
use wait::wake_expired_sleeping_processes;
use wait::wake_reaper_into;
pub use wait::{
    destroy_process, reaper_thread, record_exit_status, take_exit_status, terminate_user_processes,
    wake_process, wake_stdin_waiters, wake_waiters_for, REAPER_PID, SHELL_PID, SLEEPING, ZOMBIES,
};
pub const MAX_CPUS: usize = 32;
const KERNEL_STACK_SIZE: usize = 256 * 1024;
static SCHEDULER_STARTED: AtomicBool = AtomicBool::new(false);
pub fn is_started() -> bool {
    SCHEDULER_STARTED.load(Ordering::Acquire)
}
pub fn start_scheduler() {
    SCHEDULER_STARTED.store(true, Ordering::Release);
}
pub fn current_pid() -> u64 {
    crate::arch::x86_64::percpu::current_percpu().current_pid
}
pub fn with_current_process<R, F: FnOnce(&mut Process) -> R>(f: F) -> Option<R> {
    let cpu_id = crate::arch::x86_64::percpu::current_cpu_id() as usize;
    let idx = cpu_id.min(MAX_CPUS - 1);
    let mut rq = RUNQUEUES[idx].lock();
    rq.current.as_mut().map(f)
}
pub fn add_process(proc: Process) {
    match proc.state {
        ProcessState::Blocked | ProcessState::Sleeping(_) => {
            SLEEPING.lock().push_back(proc);
        }
        ProcessState::Dead => {
            ZOMBIES.lock().push_back(proc);
        }
        _ => {
            enqueue_process(proc);
        }
    }
}
pub fn schedule(current_sp: u64) -> u64 {
    let cpu_id = crate::arch::x86_64::percpu::current_cpu_id() as usize;
    schedule_on_cpu(cpu_id, current_sp, true)
}
pub fn schedule_on_cpu(cpu_id: usize, current_sp: u64, timer_tick: bool) -> u64 {
    if !is_started() {
        return current_sp;
    }
    let idx = cpu_id.min(MAX_CPUS - 1);
    if timer_tick && idx == 0 {
        let tick = {
            let mut rq = RUNQUEUES[idx].lock();
            rq.total_ticks = rq.total_ticks.wrapping_add(1);
            rq.total_ticks
        };
        wake_expired_sleeping_processes(tick);
    }
    let mut rq = RUNQUEUES[idx].lock();
    runqueue::drain_overflow_into(&mut rq);
    if timer_tick && idx != 0 {
        rq.total_ticks = rq.total_ticks.wrapping_add(1);
    }
    if timer_tick {
        if idx == 0 {
            if rq.total_ticks % 1000 == 0 {
                for prio in 0..16 {
                    while let Some(mut proc) = rq.queues[prio].pop_front() {
                        let boosted = (prio + 4).min(23);
                        proc.priority = boosted as u8;
                        rq.queues[boosted].push_back(proc);
                        rq.active_bitmap |= 1 << boosted;
                    }
                    rq.active_bitmap &= !(1 << prio);
                }
            }
        }
    }
    if let Some(mut current_proc) = rq.current.take() {
        unsafe {
            current_proc.fpu_state.save();
        }
        current_proc.stack_pointer = current_sp;
        if (current_proc.pending_signals & ((1 << 15) | (1 << 2) | (1 << 9))) != 0 {
            current_proc.state = ProcessState::Dead;
        }
        if current_proc.state == ProcessState::Running {
            current_proc.state = ProcessState::Ready;
        }
        if current_proc.id.0 != 1 {
            current_proc.cpu_ticks += 1;
        }
        match current_proc.state {
            ProcessState::Dead => {
                ZOMBIES.lock().push_back(current_proc);
                wake_reaper_into(&mut rq);
            }
            ProcessState::Blocked | ProcessState::Sleeping(_) => {
                SLEEPING.lock().push_back(current_proc);
            }
            _ => {
                if timer_tick {
                    if current_proc.quantum_remaining == 0 {
                        if current_proc.priority > 0 {
                            current_proc.priority -= 1;
                        }
                        current_proc.quantum_remaining = if current_proc.priority >= 24 {
                            2
                        } else if current_proc.priority >= 16 {
                            4
                        } else if current_proc.priority >= 8 {
                            8
                        } else {
                            16
                        };
                    } else {
                        current_proc.quantum_remaining -= 1;
                    }
                }
                let p = (current_proc.priority as usize).min(31);
                rq.queues[p].push_back(current_proc);
                rq.active_bitmap |= 1 << p;
            }
        }
    }
    let mut next_proc = None;
    while rq.active_bitmap != 0 {
        let highest_prio = 31 - rq.active_bitmap.leading_zeros() as usize;
        if let Some(proc) = rq.queues[highest_prio].pop_front() {
            if rq.queues[highest_prio].is_empty() {
                rq.active_bitmap &= !(1 << highest_prio);
            }
            if proc.state == ProcessState::Dead
                || (proc.pending_signals & ((1 << 15) | (1 << 2) | (1 << 9))) != 0
            {
                ZOMBIES.lock().push_back(proc);
                wake_reaper_into(&mut rq);
                continue;
            }
            next_proc = Some(proc);
            break;
        } else {
            rq.active_bitmap &= !(1 << highest_prio);
        }
    }
    if let Some(proc) = next_proc {
        return switch_to_process(&mut rq, idx, proc);
    }
    drop(rq);
    if let Some(stolen) = try_steal_task(idx) {
        let mut rq_reacquire = RUNQUEUES[idx].lock();
        return switch_to_process(&mut rq_reacquire, idx, stolen);
    }
    let rq_final = RUNQUEUES[idx].lock();
    if rq_final.current.is_none() {
        drop(rq_final);
        spawn_idle_for_cpu(idx);
        let mut rq_final = RUNQUEUES[idx].lock();
        if let Some(proc) = rq_final.queues[0].pop_front() {
            if rq_final.queues[0].is_empty() {
                rq_final.active_bitmap &= !1;
            }
            return switch_to_process(&mut rq_final, idx, proc);
        }
    }
    current_sp
}
fn switch_to_process(rq: &mut PerCpuRunqueue, cpu_id: usize, mut proc: Process) -> u64 {
    proc.state = ProcessState::Running;
    let next_sp = proc.stack_pointer;
    let pid = proc.id.0;
    unsafe {
        let current_cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) current_cr3);
        if current_cr3 != proc.cr3 && proc.cr3 != 0 {
            core::arch::asm!("mov cr3, {}", in(reg) proc.cr3);
        }
        proc.fpu_state.restore();
        if proc.kernel_stack_ptr != 0 {
            let kstack_top = proc.kernel_stack_ptr + KERNEL_STACK_SIZE as u64;
            crate::arch::x86_64::gdt::set_cpu_tss_rsp0(cpu_id, kstack_top);
            crate::task::syscall::set_cpu_kernel_rsp(cpu_id, kstack_top);
        }
    }
    let area = crate::arch::x86_64::percpu::percpu_mut(cpu_id);
    area.current_pid = pid;
    if proc.kernel_stack_ptr != 0 {
        area.kernel_rsp = proc.kernel_stack_ptr + KERNEL_STACK_SIZE as u64;
    }
    rq.current = Some(proc);
    next_sp
}
pub fn block_current_and_yield() {
    with_current_process(|current| {
        current.state = ProcessState::Blocked;
    });
    unsafe {
        core::arch::asm!("int 0xF0");
    }
}
pub fn yield_current() {
    unsafe {
        core::arch::asm!("int 0xF0");
    }
}
pub fn init() {
    crate::kprintln!("[scheduler] Inicializando Kernel Scheduler SMP...");
    spawn_idle_for_cpu(0);
    let r_pid = spawn_kernel_thread(reaper_thread, "reaper");
    REAPER_PID.store(r_pid, Ordering::Release);
}
extern "C" {
    pub fn timer_interrupt_handler_asm();
    pub fn reschedule_interrupt_handler_asm();
}
#[no_mangle]
pub extern "C" fn timer_interrupt_handler_rust(stack_pointer: u64) -> u64 {
    unsafe {
        crate::arch::x86_64::apic::end_of_interrupt();
    }
    static BLINK_TICK: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
    if BLINK_TICK.fetch_add(1, core::sync::atomic::Ordering::Relaxed) % 50 == 0 {
        crate::drivers::framebuffer::cursor_blink_tick();
    }
    let cpu_id = crate::arch::x86_64::percpu::current_cpu_id() as usize;
    if cpu_id == 0 {
        crate::drivers::xhci::poll();
    }
    schedule_on_cpu(cpu_id, stack_pointer, true)
}
#[no_mangle]
pub extern "C" fn reschedule_interrupt_handler_rust(stack_pointer: u64) -> u64 {
    unsafe {
        crate::arch::x86_64::apic::end_of_interrupt();
    }
    let cpu_id = crate::arch::x86_64::percpu::current_cpu_id() as usize;
    schedule_on_cpu(cpu_id, stack_pointer, false)
}
pub struct SchedulerFacade {
    pub started: bool,
    pub total_ticks: u64,
}
static SCHEDULER_MUTEX: crate::sync::IrqSafeSpinlock<SchedulerFacade> =
    crate::sync::IrqSafeSpinlock::new(SchedulerFacade {
        started: false,
        total_ticks: 0,
    });
pub struct SchedulerGuard {
    inner: crate::sync::IrqSafeSpinlockGuard<'static, SchedulerFacade>,
}
impl core::ops::Deref for SchedulerGuard {
    type Target = SchedulerFacade;
    fn deref(&self) -> &SchedulerFacade {
        &self.inner
    }
}
impl core::ops::DerefMut for SchedulerGuard {
    fn deref_mut(&mut self) -> &mut SchedulerFacade {
        &mut self.inner
    }
}
impl Drop for SchedulerGuard {
    fn drop(&mut self) {
        if self.inner.started {
            start_scheduler();
        }
    }
}
pub struct SchedulerLock;
impl SchedulerLock {
    pub fn lock(&self) -> SchedulerGuard {
        let mut guard = SCHEDULER_MUTEX.lock();
        guard.total_ticks = total_ticks();
        guard.started = is_started();
        SchedulerGuard { inner: guard }
    }
}
impl SchedulerFacade {
    pub fn for_each_process<F: FnMut(&Process)>(&self, f: F) {
        for_each_process(f);
    }
    pub fn is_process_alive(&self, pid: u64) -> bool {
        is_process_alive(pid)
    }
    pub fn terminate_user_processes(&mut self) {
        terminate_user_processes();
    }
    pub fn add_process(&mut self, proc: Process) {
        add_process(proc);
    }
}
pub static SCHEDULER: SchedulerLock = SchedulerLock;
