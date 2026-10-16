use super::{Process, ProcessState, MAX_CPUS};
use crate::sync::{IrqSafeSpinlock, IrqSafeSpinlockGuard};
use alloc::collections::VecDeque;
use core::sync::atomic::Ordering;
const EMPTY_QUEUE: VecDeque<Process> = VecDeque::new();
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
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
                EMPTY_QUEUE,
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
/// Cola de respaldo para `enqueue_process`.
///
/// Solo se llena si, tras varios intentos, ninguna runqueue concedió su
/// `try_lock` (todas retenidas por otro CPU o por este mismo). `schedule_on_cpu`
/// la drena en su propia runqueue, ya retenida, así que es una hoja de lock:
/// se puede tomar con `RUNQUEUES[*]` tomada sin crear ciclos.
pub static ENQUEUE_OVERFLOW: IrqSafeSpinlock<VecDeque<Process>> =
    IrqSafeSpinlock::new(VecDeque::new());
/// Intentos de `try_lock` antes de caer en `ENQUEUE_OVERFLOW`.
const ENQUEUE_MAX_ATTEMPTS: usize = 256;
static OVERFLOW_WARNED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
/// Selecciona la runqueue menos cargada que conceda su `try_lock`.
///
/// Devuelve el guard ya en la mano: el antiguo código sondeaba con `try_lock` y
/// volvía a adquirir con `.lock()`, lo que dejaba una ventana de carrera y, peor
/// aún, transformaba una operación no bloqueante en una bloqueante.
fn try_take(id: usize, best: &mut Option<(usize, IrqSafeSpinlockGuard<'static, PerCpuRunqueue>)>) {
    if let Some(guard) = RUNQUEUES[id].try_lock() {
        let load = guard.load();
        if best.as_ref().is_none_or(|(best_load, _)| load < *best_load) {
            *best = Some((load, guard));
        }
    }
}
fn least_loaded_available(
    affinity_mask: u64,
    online: usize,
) -> Option<IrqSafeSpinlockGuard<'static, PerCpuRunqueue>> {
    let mut best: Option<(usize, IrqSafeSpinlockGuard<'static, PerCpuRunqueue>)> = None;
    let mut matched = false;
    for id in 0..online.min(MAX_CPUS) {
        if affinity_mask & (1u64 << id) != 0 {
            matched = true;
            try_take(id, &mut best);
        }
    }
    if !matched {
        for id in 0..online.min(MAX_CPUS) {
            try_take(id, &mut best);
        }
    }
    best.map(|(_, guard)| guard)
}
pub fn enqueue_process(proc: Process) {
    let online = crate::arch::x86_64::smp::ONLINE_CPUS.load(Ordering::Acquire) as usize;
    let online = if online == 0 { 1 } else { online };
    for attempt in 0..ENQUEUE_MAX_ATTEMPTS {
        if let Some(mut guard) = least_loaded_available(proc.affinity_mask, online) {
            guard.enqueue(proc);
            return;
        }
        for _ in 0..(attempt + 1) * 8 {
            core::hint::spin_loop();
        }
    }
    let mut overflow = ENQUEUE_OVERFLOW.lock();
    if !OVERFLOW_WARNED.swap(true, Ordering::Relaxed) {
        crate::kprintln!(
            "[scheduler] enqueue_process: sin runqueue libre, encolando en la cola de respaldo"
        );
    }
    overflow.push_back(proc);
}
/// Drena `ENQUEUE_OVERFLOW` en la runqueue `rq`, que el llamador ya retiene.
pub fn drain_overflow_into(rq: &mut PerCpuRunqueue) {
    let mut overflow = ENQUEUE_OVERFLOW.lock();
    while let Some(proc) = overflow.pop_front() {
        rq.enqueue(proc);
    }
}
pub fn try_steal_task(my_cpu: usize) -> Option<Process> {
    let online = crate::arch::x86_64::smp::ONLINE_CPUS.load(Ordering::Acquire) as usize;
    if online <= 1 {
        return None;
    }
    let mut victim_rq: Option<IrqSafeSpinlockGuard<'static, PerCpuRunqueue>> = None;
    let mut max_load = 0;
    for id in 0..online.min(MAX_CPUS) {
        if id == my_cpu {
            continue;
        }
        if let Some(guard) = RUNQUEUES[id].try_lock() {
            let load = guard.load();
            if load > max_load && load > 1 {
                max_load = load;
                victim_rq = Some(guard);
            }
        }
    }
    let mut victim_rq = victim_rq?;
    for prio in (1..24).rev() {
        if let Some(pos) = victim_rq.queues[prio]
            .iter()
            .position(|p| (p.affinity_mask & (1 << my_cpu)) != 0)
        {
            let proc = victim_rq.queues[prio].remove(pos).unwrap();
            if victim_rq.queues[prio].is_empty() {
                victim_rq.active_bitmap &= !(1 << prio);
            }
            return Some(proc);
        }
    }
    None
}
