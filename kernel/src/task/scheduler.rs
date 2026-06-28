use alloc::collections::VecDeque;
use alloc::string::String;
use crate::sync::IrqSafeSpinlock;
use lazy_static::lazy_static;
use super::process::{Process, ProcessId, ProcessState, InterruptContext};
use core::sync::atomic::{AtomicU64, Ordering};

lazy_static! {
    pub static ref SCHEDULER: IrqSafeSpinlock<Scheduler> = IrqSafeSpinlock::new(Scheduler::new());
}

static NEXT_PID: AtomicU64 = AtomicU64::new(1);
/// Tamaño de la pila de kernel por hilo (los frames en debug de ELF/VFS desbordaban 16 KiB).
const KERNEL_STACK_SIZE: usize = 256 * 1024;

pub struct Scheduler {
    pub processes: VecDeque<Process>,
    pub zombies: VecDeque<Process>,
    pub current: Option<Process>,
    pub total_ticks: u64,
    pub started: bool,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            processes: VecDeque::new(),
            zombies: VecDeque::new(),
            current: None,
            total_ticks: 0,
            started: false,
        }
    }

    pub fn add_process(&mut self, process: Process) {
        self.processes.push_back(process);
    }

    pub fn schedule(&mut self, current_sp: u64) -> u64 {
        if !self.started {
            return current_sp;
        }
        self.total_ticks += 1;

        while let Some(zombie) = self.zombies.pop_front() {
            destroy_process(&zombie);
        }

        if let Some(mut current_proc) = self.current.take() {
            current_proc.stack_pointer = current_sp;

            // Handle pending signals (default action for 2 and 15 is terminate)
            if (current_proc.pending_signals & ((1 << 15) | (1 << 2))) != 0 {
                current_proc.state = ProcessState::Dead;
            }

            if current_proc.state == ProcessState::Running {
                current_proc.state = ProcessState::Ready;
            }
            if current_proc.id.0 != 1 {
                current_proc.cpu_ticks += 1;
            }
            
            if current_proc.state == ProcessState::Dead {
                self.zombies.push_back(current_proc);
            } else {
                self.processes.push_back(current_proc);
            }
        }

        // Garbage collection loop for queued processes that died
        let mut next_proc = None;
        while let Some(mut proc) = self.processes.pop_front() {
            if proc.state == ProcessState::Dead || (proc.pending_signals & ((1 << 15) | (1 << 2))) != 0 {
                self.zombies.push_back(proc);
                continue;
            }
            if let ProcessState::Sleeping(wakeup) = proc.state {
                if self.total_ticks >= wakeup {
                    proc.state = ProcessState::Ready;
                }
            }
            if proc.state == ProcessState::Ready || proc.state == ProcessState::Running {
                next_proc = Some(proc);
                break;
            } else {
                self.processes.push_back(proc);
            }
        }

        if let Some(mut proc) = next_proc {
            proc.state = ProcessState::Running;
            let next_sp = proc.stack_pointer;
            
            unsafe {
                let current_cr3: u64;
                core::arch::asm!("mov {}, cr3", out(reg) current_cr3);
                if current_cr3 != proc.cr3 && proc.cr3 != 0 {
                    core::arch::asm!("mov cr3, {}", in(reg) proc.cr3);
                }
                if proc.kernel_stack_ptr != 0 {
                    crate::arch::x86_64::gdt::set_tss_rsp0(proc.kernel_stack_ptr + KERNEL_STACK_SIZE as u64);
                }
            }
            
            self.current = Some(proc);
            return next_sp;
        }

        current_sp
    }
}

pub fn destroy_process(proc: &Process) {
    if proc.id.0 == 1 || proc.id.0 == 2 {
        crate::kprintln!("AVISO: Intento de destruir proceso critico de Ring 0 (PID {})", proc.id.0);
        return;
    }
    crate::kprintln!("destroy_process: Liberando recursos PID {} ({})", proc.id.0, proc.name);
    
    // 1. Recover kernel stack
    if proc.kernel_stack_ptr != 0 {
        unsafe {
            let layout = core::alloc::Layout::from_size_align(KERNEL_STACK_SIZE, 16).unwrap();
            alloc::alloc::dealloc(proc.kernel_stack_ptr as *mut u8, layout);
        }
    }
    
    // 2. Free PML4 recursively (User process only, cr3 != 0 and != kernel pml4)
    if proc.pml4_frame != 0 {
        unsafe {
            crate::memory::vmm::destroy_userspace_pml4(proc.pml4_frame);
        }
    }
}

pub fn init() {
    crate::kprintln!("[scheduler] Inicializando Kernel Scheduler...");
    spawn_kernel_thread(crate::arch::x86_64::halt_loop, "idle");
}

pub fn block_current_and_yield() {
    {
        let mut sched = SCHEDULER.lock();
        if let Some(ref mut current) = sched.current {
            current.state = ProcessState::Blocked;
        }
    }
    unsafe { core::arch::asm!("int 32"); }
}

pub fn yield_current() {
    unsafe { core::arch::asm!("int 32"); }
}

pub fn wake_process(pid: u64) {
    let mut sched = SCHEDULER.lock();
    for proc in sched.processes.iter_mut() {
        if proc.id.0 == pid && proc.state == ProcessState::Blocked {
            proc.state = ProcessState::Ready;
        }
    }
    if let Some(ref mut current) = sched.current {
        if current.id.0 == pid && current.state == ProcessState::Blocked {
            current.state = ProcessState::Ready;
        }
    }
}

pub fn spawn_kernel_thread(entry_point: fn() -> !, name: &str) {
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
        // ABI requires RSP % 16 == 8 upon function entry
        context.rsp = stack_end - 8;
    }
    
    let pid = ProcessId(NEXT_PID.fetch_add(1, Ordering::SeqCst));
    let cr3: u64;
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3); }
    let proc = Process::new(pid, String::from(name), context_ptr as u64, cr3, stack_ptr, (cr3 / 4096) as usize, 0);
    
    SCHEDULER.lock().add_process(proc);
}

pub fn spawn_user_process(entry_point: u64, stack_top: u64, pml4_frame: usize, name: &str, privilege: u8) -> u64 {
    let layout = core::alloc::Layout::from_size_align(KERNEL_STACK_SIZE, 16).unwrap();
    let stack_ptr = unsafe { alloc::alloc::alloc(layout) as u64 };
    let stack_end = stack_ptr + KERNEL_STACK_SIZE as u64;
    
    let context_ptr = (stack_end - core::mem::size_of::<InterruptContext>() as u64) as *mut InterruptContext;
    
    unsafe {
        core::ptr::write_bytes(context_ptr as *mut u8, 0, core::mem::size_of::<InterruptContext>());
        let context = &mut *context_ptr;
        context.rip = entry_point;
        
        if privilege == 0 {
            context.cs = 0x08;
            context.ss = 0x10;
        } else {
            context.cs = 0x1B;
            context.ss = 0x23;
        }
        
        let mut rflags = 0x202; // IF set
        if privilege <= 1 {
            rflags |= (3 << 12); // IOPL = 3 for ring 1 and 0
        }
        context.rflags = rflags;
        // ABI requires RSP % 16 == 8 upon function entry
        context.rsp = stack_top - 8;
    }
    
    let pid_val = NEXT_PID.fetch_add(1, Ordering::SeqCst);
    let pid = ProcessId(pid_val);
    let proc = Process::new(pid, String::from(name), context_ptr as u64, (pml4_frame as u64) * 4096, stack_ptr, pml4_frame, privilege);
    
    {
        let mut s = SCHEDULER.lock();
        s.add_process(proc);
    }
    
    pid_val
}

pub fn send_signal(pid: ProcessId, signal: u8) {
    let mut sched = SCHEDULER.lock();
    for proc in sched.processes.iter_mut() {
        if proc.id == pid {
            proc.pending_signals |= 1 << signal;
            if signal == 9 {
                proc.state = ProcessState::Dead;
            } else if proc.state == ProcessState::Blocked {
                proc.state = ProcessState::Ready;
            }
        }
    }
    if let Some(ref mut current) = sched.current {
        if current.id == pid {
            current.pending_signals |= 1 << signal;
            if signal == 9 {
                current.state = ProcessState::Dead;
            }
        }
    }
}

#[unsafe(naked)]
pub extern "C" fn timer_interrupt_handler_asm() {
    unsafe {
        core::arch::naked_asm!(
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
            "iretq",
        );
    }
}

#[no_mangle]
pub extern "C" fn timer_interrupt_handler_rust(stack_pointer: u64) -> u64 {
    unsafe { crate::arch::x86_64::apic::end_of_interrupt(); }
    
    let mut sched = SCHEDULER.lock();
    sched.schedule(stack_pointer)
}