use alloc::collections::VecDeque;
use alloc::string::String;
use crate::sync::IrqSafeSpinlock;
use super::process::{Process, ProcessId, ProcessState, InterruptContext};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub const MAX_CPUS: usize = 32;
const KERNEL_STACK_SIZE: usize = 256 * 1024;
const EMPTY_QUEUE: VecDeque<Process> = VecDeque::new();

static NEXT_PID: AtomicU64 = AtomicU64::new(1);
pub static REAPER_PID: AtomicU64 = AtomicU64::new(0);
pub static SHELL_PID: AtomicU64 = AtomicU64::new(0);
static SCHEDULER_STARTED: AtomicBool = AtomicBool::new(false);

pub struct PerCpuRunqueue {
    pub cpu_id: u32,
    pub queues: [VecDeque<Process>; 32],
    pub active_bitmap: u32,
    pub current: Option<Process>,
    pub total_ticks: u64,
    pub quantum_remaining: u8,
    pub idle_ticks: u64,
}

impl PerCpuRunqueue {
    pub const fn new(cpu_id: u32) -> Self {
        Self {
            cpu_id,
            queues: [
                EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE,
                EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE,
                EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE,
                EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE,
                EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE,
                EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE,
                EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE,
                EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE, EMPTY_QUEUE,
            ],
            active_bitmap: 0,
            current: None,
            total_ticks: 0,
            quantum_remaining: 4,
            idle_ticks: 0,
        }
    }

    pub fn enqueue(&mut self, mut proc: Process) {
        let prio = (proc.priority as usize).min(31);
        proc.state = ProcessState::Ready;
        self.queues[prio].push_back(proc);
        self.active_bitmap |= 1 << prio;
    }

    pub fn load(&self) -> usize {
        let mut count = if self.current.is_some() { 1 } else { 0 };
        for q in &self.queues {
            count += q.len();
        }
        count
    }
}

pub static RUNQUEUES: [IrqSafeSpinlock<PerCpuRunqueue>; MAX_CPUS] = [
    IrqSafeSpinlock::new(PerCpuRunqueue::new(0)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(1)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(2)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(3)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(4)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(5)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(6)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(7)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(8)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(9)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(10)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(11)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(12)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(13)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(14)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(15)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(16)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(17)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(18)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(19)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(20)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(21)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(22)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(23)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(24)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(25)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(26)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(27)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(28)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(29)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(30)),
    IrqSafeSpinlock::new(PerCpuRunqueue::new(31)),
];

pub static SLEEPING: IrqSafeSpinlock<VecDeque<Process>> = IrqSafeSpinlock::new(VecDeque::new());
pub static ZOMBIES: IrqSafeSpinlock<VecDeque<Process>> = IrqSafeSpinlock::new(VecDeque::new());

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

pub fn enqueue_process(proc: Process) {
    let online = crate::arch::x86_64::smp::ONLINE_CPUS.load(Ordering::Acquire) as usize;
    let mut best_cpu = 0usize;
    let mut min_load = usize::MAX;

    for id in 0..online.min(MAX_CPUS) {
        if (proc.affinity_mask & (1 << id)) != 0 {
            if let Some(guard) = RUNQUEUES[id].try_lock() {
                let load = guard.load();
                if load < min_load {
                    min_load = load;
                    best_cpu = id;
                }
            }
        }
    }

    let mut target_rq = RUNQUEUES[best_cpu].lock();
    target_rq.enqueue(proc);
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

pub fn reschedule(current_sp: u64) -> u64 {
    let cpu_id = crate::arch::x86_64::percpu::current_cpu_id() as usize;
    schedule_on_cpu(cpu_id, current_sp, false)
}

pub fn schedule_on_cpu(cpu_id: usize, current_sp: u64, timer_tick: bool) -> u64 {
    if !is_started() {
        return current_sp;
    }

    let idx = cpu_id.min(MAX_CPUS - 1);
    let mut rq = RUNQUEUES[idx].lock();

    if timer_tick {
        rq.total_ticks += 1;
        if idx == 0 {
            wake_expired_sleeping_processes(rq.total_ticks);

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

    wake_reaper_if_needed();

    if let Some(mut current_proc) = rq.current.take() {
        current_proc.stack_pointer = current_sp;

        if (current_proc.pending_signals & ((1 << 15) | (1 << 2))) != 0 {
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
            if proc.state == ProcessState::Dead || (proc.pending_signals & ((1 << 15) | (1 << 2))) != 0 {
                ZOMBIES.lock().push_back(proc);
                continue;
            }
            next_proc = Some(proc);
            break;
        } else {
            rq.active_bitmap &= !(1 << highest_prio);
        }
    }

    if next_proc.is_none() {
        drop(rq);
        if let Some(stolen) = try_steal_task(idx) {
            let mut rq_reacquire = RUNQUEUES[idx].lock();
            return switch_to_process(&mut rq_reacquire, idx, stolen);
        }
        return current_sp;
    }

    switch_to_process(&mut rq, idx, next_proc.unwrap())
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

pub fn try_steal_task(my_cpu: usize) -> Option<Process> {
    let online = crate::arch::x86_64::smp::ONLINE_CPUS.load(Ordering::Acquire) as usize;
    if online <= 1 {
        return None;
    }

    let mut victim_cpu = None;
    let mut max_load = 0;

    for id in 0..online.min(MAX_CPUS) {
        if id == my_cpu {
            continue;
        }
        if let Some(guard) = RUNQUEUES[id].try_lock() {
            let load = guard.load();
            if load > max_load && load > 1 {
                max_load = load;
                victim_cpu = Some(id);
            }
        }
    }

    let victim_id = victim_cpu?;
    let mut victim_rq = RUNQUEUES[victim_id].lock();

    for prio in (1..24).rev() {
        if let Some(pos) = victim_rq.queues[prio].iter().position(|p| {
            (p.affinity_mask & (1 << my_cpu)) != 0
        }) {
            let proc = victim_rq.queues[prio].remove(pos).unwrap();
            if victim_rq.queues[prio].is_empty() {
                victim_rq.active_bitmap &= !(1 << prio);
            }
            return Some(proc);
        }
    }

    None
}

fn wake_expired_sleeping_processes(ticks: u64) {
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
            enqueue_process(proc);
            continue;
        }
        i += 1;
    }
}

fn wake_reaper_if_needed() {
    if !ZOMBIES.lock().is_empty() {
        let r_pid = REAPER_PID.load(Ordering::Relaxed);
        if r_pid != 0 {
            wake_process(r_pid);
        }
    }
}

pub fn destroy_process(proc: &Process) {
    let r_pid = REAPER_PID.load(Ordering::Relaxed);
    if proc.id.0 == 1 || (r_pid != 0 && proc.id.0 == r_pid) {
        crate::kprintln!("AVISO: Intento de destruir proceso critico de Ring 0 (PID {})", proc.id.0);
        return;
    }
    crate::kprintln!("destroy_process: Liberando recursos PID {} ({})", proc.id.0, proc.name);

    if proc.kernel_stack_ptr != 0 {
        unsafe {
            let layout = core::alloc::Layout::from_size_align(KERNEL_STACK_SIZE, 16).unwrap();
            alloc::alloc::dealloc(proc.kernel_stack_ptr as *mut u8, layout);
        }
    }

    if proc.pml4_frame != 0 {
        unsafe {
            crate::memory::vmm::destroy_userspace_pml4(proc.pml4_frame);
        }
    }
}

pub fn reaper_thread() -> ! {
    loop {
        let dead_proc = ZOMBIES.lock().pop_front();
        if let Some(proc) = dead_proc {
            let pid = proc.id;
            destroy_process(&proc);
            wake_waiters_for(pid);
        } else {
            block_current_and_yield();
        }
    }
}

pub fn spawn_idle_for_cpu(cpu_id: usize) -> u64 {
    let layout = core::alloc::Layout::from_size_align(KERNEL_STACK_SIZE, 16).unwrap();
    let stack_ptr = unsafe { alloc::alloc::alloc(layout) as u64 };
    let stack_end = stack_ptr + KERNEL_STACK_SIZE as u64;

    let context_ptr = (stack_end - core::mem::size_of::<InterruptContext>() as u64) as *mut InterruptContext;

    unsafe {
        core::ptr::write_bytes(context_ptr as *mut u8, 0, core::mem::size_of::<InterruptContext>());
        let context = &mut *context_ptr;
        context.rip = crate::arch::x86_64::halt_loop as *const () as usize as u64;
        context.cs = 0x8;
        context.ss = 0x0;
        context.rflags = 0x202;
        context.rsp = stack_end - 8;
    }

    let pid_val = NEXT_PID.fetch_add(1, Ordering::SeqCst);
    let pid = ProcessId(pid_val);
    let cr3: u64;
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3); }
    let name = alloc::format!("idle/{}", cpu_id);
    let mut proc = Process::new(pid, name, context_ptr as u64, cr3, stack_ptr, (cr3 / 4096) as usize, 0);
    proc.ram_bytes_used = KERNEL_STACK_SIZE as u64;
    proc.priority = 0;
    proc.quantum_remaining = 16;
    proc.affinity_mask = 1 << cpu_id;

    let idx = cpu_id.min(MAX_CPUS - 1);
    let mut rq = RUNQUEUES[idx].lock();
    rq.enqueue(proc);

    pid_val
}

pub fn init() {
    crate::kprintln!("[scheduler] Inicializando Kernel Scheduler SMP...");
    spawn_idle_for_cpu(0);
    let r_pid = spawn_kernel_thread(reaper_thread, "reaper");
    REAPER_PID.store(r_pid, Ordering::Release);
}

pub fn block_current_and_yield() {
    with_current_process(|current| {
        current.state = ProcessState::Blocked;
    });
    unsafe { core::arch::asm!("int 0xF0"); }
}

pub fn yield_current() {
    unsafe { core::arch::asm!("int 0xF0"); }
}

pub fn reschedule_from_interrupt() {
    unsafe { core::arch::asm!("int 0xF0"); }
}

pub fn wake_process(pid: u64) {
    let mut sleeping = SLEEPING.lock();
    if let Some(pos) = sleeping.iter().position(|p| p.id.0 == pid) {
        let mut proc = sleeping.remove(pos).unwrap();
        proc.state = ProcessState::Ready;
        enqueue_process(proc);
    }
    with_current_process(|current| {
        if current.id.0 == pid && current.state == ProcessState::Blocked {
            current.state = ProcessState::Ready;
        }
    });
}

pub fn wake_waiters_for(exited_pid: ProcessId) {
    let mut sleeping = SLEEPING.lock();
    let mut i = 0;
    while i < sleeping.len() {
        if sleeping[i].waiting_for == Some(exited_pid) {
            let mut proc = sleeping.remove(i).unwrap();
            proc.waiting_for = None;
            proc.state = ProcessState::Ready;
            enqueue_process(proc);
            continue;
        }
        i += 1;
    }
    with_current_process(|current| {
        if current.waiting_for == Some(exited_pid) {
            current.waiting_for = None;
            current.state = ProcessState::Ready;
        }
    });
}

pub fn wake_stdin_waiters() -> bool {
    let fg = crate::shell::FOREGROUND_TASK.load(Ordering::SeqCst);
    let s_pid = SHELL_PID.load(Ordering::Relaxed);

    let mut sleeping = SLEEPING.lock();
    let mut woke_waiter = false;
    let mut i = 0;
    while i < sleeping.len() {
        let target_pid = sleeping[i].id.0;
        if target_pid == fg || (fg == 0 && target_pid == s_pid) {
            let mut proc = sleeping.remove(i).unwrap();
            proc.state = ProcessState::Ready;
            enqueue_process(proc);
            woke_waiter = true;
            continue;
        }
        i += 1;
    }
    with_current_process(|current| {
        if current.id.0 == fg || (fg == 0 && current.id.0 == s_pid) {
            if matches!(current.state, ProcessState::Blocked | ProcessState::Sleeping(_)) {
                current.state = ProcessState::Ready;
                woke_waiter = true;
            }
        }
    });
    woke_waiter
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

pub fn spawn_kernel_thread(entry_point: fn() -> !, name: &str) -> u64 {
    let layout = core::alloc::Layout::from_size_align(KERNEL_STACK_SIZE, 16).unwrap();
    let stack_ptr = unsafe { alloc::alloc::alloc(layout) as u64 };
    let stack_end = stack_ptr + KERNEL_STACK_SIZE as u64;

    let context_ptr = (stack_end - core::mem::size_of::<InterruptContext>() as u64) as *mut InterruptContext;

    unsafe {
        core::ptr::write_bytes(context_ptr as *mut u8, 0, core::mem::size_of::<InterruptContext>());
        let context = &mut *context_ptr;
        context.rip = entry_point as usize as u64;
        context.cs = 0x8;
        context.ss = 0x0;
        context.rflags = 0x202;
        context.rsp = stack_end - 8;
    }

    let pid_val = NEXT_PID.fetch_add(1, Ordering::SeqCst);
    let pid = ProcessId(pid_val);
    let cr3: u64;
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3); }
    let mut proc = Process::new(pid, String::from(name), context_ptr as u64, cr3, stack_ptr, (cr3 / 4096) as usize, 0);
    proc.ram_bytes_used = KERNEL_STACK_SIZE as u64;

    if name == "idle" {
        proc.priority = 0;
        proc.quantum_remaining = 16;
    } else if name == "reaper" {
        proc.priority = 28;
        proc.quantum_remaining = 2;
    } else {
        proc.priority = 24;
        proc.quantum_remaining = 2;
    }

    add_process(proc);
    pid_val
}

pub fn spawn_user_process(
    entry_point: u64,
    stack_top: u64,
    pml4_frame: usize,
    name: &str,
    privilege: u8,
    user_memory_bytes: u64,
    vma_mgr: crate::memory::vma::VmaManager,
) -> Result<u64, &'static str> {
    let layout = match core::alloc::Layout::from_size_align(KERNEL_STACK_SIZE, 16) {
        Ok(layout) => layout,
        Err(_) => {
            unsafe { crate::memory::vmm::destroy_userspace_pml4(pml4_frame); }
            return Err("Layout de stack de kernel inválido");
        }
    };
    let stack_ptr = unsafe { alloc::alloc::alloc(layout) };
    if stack_ptr.is_null() {
        unsafe { crate::memory::vmm::destroy_userspace_pml4(pml4_frame); }
        return Err("Sin memoria para stack de kernel");
    }
    let stack_ptr = stack_ptr as u64;
    let stack_end = stack_ptr + KERNEL_STACK_SIZE as u64;

    let context_ptr = (stack_end - core::mem::size_of::<InterruptContext>() as u64) as *mut InterruptContext;

    unsafe {
        core::ptr::write_bytes(context_ptr as *mut u8, 0, core::mem::size_of::<InterruptContext>());
        let context = &mut *context_ptr;
        context.rip = entry_point;

        if privilege == 0 {
            context.cs = crate::arch::x86_64::gdt::GDT.1.code_selector.0 as u64;
            context.ss = crate::arch::x86_64::gdt::GDT.1.data_selector.0 as u64;
        } else {
            context.cs = crate::arch::x86_64::gdt::GDT.1.user_code_selector.0 as u64;
            context.ss = crate::arch::x86_64::gdt::GDT.1.user_data_selector.0 as u64;
        }

        let mut rflags = 0x202;
        if privilege <= 1 {
            rflags |= 3 << 12;
        }
        context.rflags = rflags;
        context.rsp = stack_top - 8;
    }

    let pid_val = NEXT_PID.fetch_add(1, Ordering::SeqCst);
    let pid = ProcessId(pid_val);
    let mut proc = Process::new(pid, String::from(name), context_ptr as u64, (pml4_frame as u64) * 4096, stack_ptr, pml4_frame, privilege);
    proc.ram_bytes_used = user_memory_bytes.saturating_add(KERNEL_STACK_SIZE as u64);
    proc.vma_mgr = vma_mgr;

    if name.contains("vanish") {
        proc.priority = 22;
        proc.quantum_remaining = 4;
    } else {
        proc.priority = 16;
        proc.quantum_remaining = 4;
    }

    add_process(proc);
    Ok(pid_val)
}

pub fn send_signal(pid: ProcessId, signal: u8) {
    if signal >= u64::BITS as u8 { return; }

    let mut found = false;
    let mut sleeping = SLEEPING.lock();
    if let Some(pos) = sleeping.iter().position(|p| p.id == pid) {
        found = true;
        if signal == 9 {
            let mut proc = sleeping.remove(pos).unwrap();
            proc.pending_signals |= 1 << signal;
            proc.state = ProcessState::Dead;
            ZOMBIES.lock().push_back(proc);
        } else {
            sleeping[pos].pending_signals |= 1 << signal;
            let mut proc = sleeping.remove(pos).unwrap();
            proc.state = ProcessState::Ready;
            enqueue_process(proc);
        }
    }
    drop(sleeping);

    if !found {
        let online = crate::arch::x86_64::smp::ONLINE_CPUS.load(Ordering::Acquire) as usize;
        for id in 0..online.min(MAX_CPUS) {
            let mut rq = RUNQUEUES[id].lock();
            for q in 0..32 {
                if let Some(pos) = rq.queues[q].iter().position(|p| p.id == pid) {
                    if signal == 9 {
                        let mut proc = rq.queues[q].remove(pos).unwrap();
                        if rq.queues[q].is_empty() {
                            rq.active_bitmap &= !(1 << q);
                        }
                        proc.pending_signals |= 1 << signal;
                        proc.state = ProcessState::Dead;
                        ZOMBIES.lock().push_back(proc);
                    } else {
                        rq.queues[q][pos].pending_signals |= 1 << signal;
                    }
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
    }

    with_current_process(|current| {
        if current.id == pid {
            current.pending_signals |= 1 << signal;
            if signal == 9 {
                current.state = ProcessState::Dead;
            }
        }
    });

    if signal == 9 {
        let fg = crate::shell::FOREGROUND_TASK.load(Ordering::SeqCst);
        if fg == pid.0 {
            crate::shell::FOREGROUND_TASK.store(0, Ordering::SeqCst);
            crate::drivers::framebuffer::leave_alt_screen_if_active();
        }
    }
}

pub fn total_ticks() -> u64 {
    RUNQUEUES[0].lock().total_ticks
}

#[unsafe(naked)]
pub extern "C" fn timer_interrupt_handler_asm() {
    core::arch::naked_asm!(
        "test byte ptr [rsp + 8], 3",
        "jz 1f",
        "swapgs",
    "1:",
        "push rax", "push rcx", "push rdx", "push rbx",
        "push rbp", "push rsi", "push rdi",
        "push r8", "push r9", "push r10", "push r11",
        "push r12", "push r13", "push r14", "push r15",

        "mov rdi, rsp",
        "and rsp, -16",
        "call timer_interrupt_handler_rust",
        "mov rsp, rax",

        "pop r15", "pop r14", "pop r13", "pop r12",
        "pop r11", "pop r10", "pop r9", "pop r8",
        "pop rdi", "pop rsi", "pop rbp",
        "pop rbx", "pop rdx", "pop rcx", "pop rax",

        "test byte ptr [rsp + 8], 3",
        "jz 2f",
        "swapgs",
    "2:",
        "iretq",
    );
}

#[unsafe(naked)]
pub extern "C" fn reschedule_interrupt_handler_asm() {
    core::arch::naked_asm!(
        "test byte ptr [rsp + 8], 3",
        "jz 1f",
        "swapgs",
    "1:",
        "push rax", "push rcx", "push rdx", "push rbx",
        "push rbp", "push rsi", "push rdi",
        "push r8", "push r9", "push r10", "push r11",
        "push r12", "push r13", "push r14", "push r15",
        "mov rdi, rsp",
        "and rsp, -16",
        "call reschedule_interrupt_handler_rust",
        "mov rsp, rax",
        "pop r15", "pop r14", "pop r13", "pop r12",
        "pop r11", "pop r10", "pop r9", "pop r8",
        "pop rdi", "pop rsi", "pop rbp",
        "pop rbx", "pop rdx", "pop rcx", "pop rax",
        "test byte ptr [rsp + 8], 3",
        "jz 2f",
        "swapgs",
    "2:",
        "iretq",
    );
}

#[no_mangle]
pub extern "C" fn timer_interrupt_handler_rust(stack_pointer: u64) -> u64 {
    unsafe { crate::arch::x86_64::apic::end_of_interrupt(); }

    static BLINK_TICK: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
    if BLINK_TICK.fetch_add(1, core::sync::atomic::Ordering::Relaxed) % 50 == 0 {
        crate::drivers::framebuffer::cursor_blink_tick();
    }

    let cpu_id = crate::arch::x86_64::percpu::current_cpu_id() as usize;
    schedule_on_cpu(cpu_id, stack_pointer, true)
}

#[no_mangle]
pub extern "C" fn reschedule_interrupt_handler_rust(stack_pointer: u64) -> u64 {
    let cpu_id = crate::arch::x86_64::percpu::current_cpu_id() as usize;
    schedule_on_cpu(cpu_id, stack_pointer, false)
}

pub struct SchedulerFacade {
    pub started: bool,
    pub total_ticks: u64,
}

pub struct SchedulerGuard;
impl core::ops::Deref for SchedulerGuard {
    type Target = SchedulerFacade;
    fn deref(&self) -> &SchedulerFacade {
        unsafe { &*core::ptr::addr_of!(SCHEDULER_FACADE) }
    }
}
impl core::ops::DerefMut for SchedulerGuard {
    fn deref_mut(&mut self) -> &mut SchedulerFacade {
        unsafe { &mut *core::ptr::addr_of_mut!(SCHEDULER_FACADE) }
    }
}

impl Drop for SchedulerGuard {
    fn drop(&mut self) {
        if unsafe { SCHEDULER_FACADE.started } {
            start_scheduler();
        }
    }
}

static mut SCHEDULER_FACADE: SchedulerFacade = SchedulerFacade {
    started: false,
    total_ticks: 0,
};

pub struct SchedulerLock;
impl SchedulerLock {
    pub fn lock(&self) -> SchedulerGuard {
        unsafe {
            SCHEDULER_FACADE.total_ticks = total_ticks();
            SCHEDULER_FACADE.started = is_started();
        }
        SchedulerGuard
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
