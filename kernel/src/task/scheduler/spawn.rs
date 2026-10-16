use super::runqueue::{enqueue_process, RUNQUEUES};
use super::wait::{wake_reaper_if_needed, wake_waiters_for, SLEEPING, ZOMBIES};
use super::{add_process, KERNEL_STACK_SIZE, MAX_CPUS};
use crate::task::process::{InterruptContext, Process, ProcessId, ProcessState};
use alloc::string::String;
use core::sync::atomic::{AtomicU64, Ordering};
static NEXT_PID: AtomicU64 = AtomicU64::new(1);
pub fn spawn_idle_for_cpu(cpu_id: usize) -> u64 {
    let layout = core::alloc::Layout::from_size_align(KERNEL_STACK_SIZE, 16).unwrap();
    let stack_ptr = unsafe { alloc::alloc::alloc(layout) as u64 };
    let stack_end = stack_ptr + KERNEL_STACK_SIZE as u64;
    let context_ptr =
        (stack_end - core::mem::size_of::<InterruptContext>() as u64) as *mut InterruptContext;
    unsafe {
        core::ptr::write_bytes(
            context_ptr as *mut u8,
            0,
            core::mem::size_of::<InterruptContext>(),
        );
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
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3);
    }
    let name = alloc::format!("idle/{}", cpu_id);
    let mut proc = Process::new(
        pid,
        name,
        context_ptr as u64,
        cr3,
        stack_ptr,
        (cr3 / 4096) as usize,
        0,
    );
    proc.ram_bytes_used = KERNEL_STACK_SIZE as u64;
    proc.priority = 0;
    proc.quantum_remaining = 16;
    proc.affinity_mask = 1 << cpu_id;
    let idx = cpu_id.min(MAX_CPUS - 1);
    let mut rq = RUNQUEUES[idx].lock();
    rq.enqueue(proc);
    pid_val
}
pub fn spawn_kernel_thread(entry_point: fn() -> !, name: &str) -> u64 {
    let layout = core::alloc::Layout::from_size_align(KERNEL_STACK_SIZE, 16).unwrap();
    let stack_ptr = unsafe { alloc::alloc::alloc(layout) as u64 };
    let stack_end = stack_ptr + KERNEL_STACK_SIZE as u64;
    let context_ptr =
        (stack_end - core::mem::size_of::<InterruptContext>() as u64) as *mut InterruptContext;
    unsafe {
        core::ptr::write_bytes(
            context_ptr as *mut u8,
            0,
            core::mem::size_of::<InterruptContext>(),
        );
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
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3);
    }
    let mut proc = Process::new(
        pid,
        String::from(name),
        context_ptr as u64,
        cr3,
        stack_ptr,
        (cr3 / 4096) as usize,
        0,
    );
    proc.ram_bytes_used = KERNEL_STACK_SIZE as u64;
    proc.rights = crate::task::rights::KERNEL;
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
    rights: lix_sys::Rights,
) -> Result<u64, &'static str> {
    if privilege != 3 {
        unsafe {
            crate::memory::vmm::destroy_userspace_pml4(pml4_frame);
        }
        return Err("ELF de usuario solo puede ejecutarse en Ring 3");
    }
    let layout = match core::alloc::Layout::from_size_align(KERNEL_STACK_SIZE, 16) {
        Ok(layout) => layout,
        Err(_) => {
            unsafe {
                crate::memory::vmm::destroy_userspace_pml4(pml4_frame);
            }
            return Err("Layout de stack de kernel inválido");
        }
    };
    let stack_ptr = unsafe { alloc::alloc::alloc(layout) };
    if stack_ptr.is_null() {
        unsafe {
            crate::memory::vmm::destroy_userspace_pml4(pml4_frame);
        }
        return Err("Sin memoria para stack de kernel");
    }
    let stack_ptr = stack_ptr as u64;
    let stack_end = stack_ptr + KERNEL_STACK_SIZE as u64;
    let context_ptr =
        (stack_end - core::mem::size_of::<InterruptContext>() as u64) as *mut InterruptContext;
    unsafe {
        core::ptr::write_bytes(
            context_ptr as *mut u8,
            0,
            core::mem::size_of::<InterruptContext>(),
        );
        let context = &mut *context_ptr;
        context.rip = entry_point;
        context.cs = crate::arch::x86_64::gdt::GDT.1.user_code_selector.0 as u64;
        context.ss = crate::arch::x86_64::gdt::GDT.1.user_data_selector.0 as u64;
        context.rflags = 0x202;
        context.rsp = stack_top - 8;
    }
    let pid_val = NEXT_PID.fetch_add(1, Ordering::SeqCst);
    let pid = ProcessId(pid_val);
    let mut proc = Process::new(
        pid,
        String::from(name),
        context_ptr as u64,
        (pml4_frame as u64) * 4096,
        stack_ptr,
        pml4_frame,
        3,
    );
    proc.ram_bytes_used = user_memory_bytes.saturating_add(KERNEL_STACK_SIZE as u64);
    proc.vma_mgr = vma_mgr;
    proc.rights = rights;
    proc.parent_pid = crate::task::scheduler::current_pid();
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
    if signal >= u64::BITS as u8 {
        return;
    }
    let is_fatal = signal == 9 || signal == 2 || signal == 15;
    let mut found = false;
    let mut killed = false;
    let mut ipi_cpu = None;
    let mut non_fatal_proc = None;
    {
        let mut sleeping = SLEEPING.lock();
        if let Some(pos) = sleeping.iter().position(|p| p.id == pid) {
            found = true;
            let mut proc = sleeping.remove(pos).unwrap();
            proc.pending_signals |= 1 << signal;
            if is_fatal {
                proc.state = ProcessState::Dead;
                proc.exit_code = 128 + signal as i32;
                super::wait::record_exit_status(proc.id.0, proc.exit_code);
                ZOMBIES.lock().push_back(proc);
                killed = true;
            } else {
                proc.state = ProcessState::Ready;
                non_fatal_proc = Some(proc);
            }
        }
    }
    if let Some(proc) = non_fatal_proc {
        enqueue_process(proc);
    }
    if !found {
        let online = crate::arch::x86_64::smp::ONLINE_CPUS.load(Ordering::Acquire) as usize;
        let curr_cpu = crate::arch::x86_64::percpu::current_cpu_id() as usize;
        for id in 0..online.min(MAX_CPUS) {
            let mut rq = RUNQUEUES[id].lock();
            for q in 0..32 {
                if let Some(pos) = rq.queues[q].iter().position(|p| p.id == pid) {
                    if is_fatal {
                        let mut proc = rq.queues[q].remove(pos).unwrap();
                        if rq.queues[q].is_empty() {
                            rq.active_bitmap &= !(1 << q);
                        }
                        proc.pending_signals |= 1 << signal;
                        proc.state = ProcessState::Dead;
                        proc.exit_code = 128 + signal as i32;
                        super::wait::record_exit_status(proc.id.0, proc.exit_code);
                        ZOMBIES.lock().push_back(proc);
                        killed = true;
                    } else {
                        rq.queues[q][pos].pending_signals |= 1 << signal;
                    }
                    found = true;
                    break;
                }
            }
            if !found {
                if let Some(ref mut curr) = rq.current {
                    if curr.id == pid {
                        curr.pending_signals |= 1 << signal;
                        if is_fatal {
                            curr.state = ProcessState::Dead;
                            curr.exit_code = 128 + signal as i32;
                            super::wait::record_exit_status(curr.id.0, curr.exit_code);
                            killed = true;
                            if id != curr_cpu {
                                ipi_cpu = Some(id);
                            }
                        }
                        found = true;
                    }
                }
            }
            drop(rq);
            if found {
                break;
            }
        }
    }
    if let Some(target_cpu) = ipi_cpu {
        unsafe {
            crate::arch::x86_64::apic::send_ipi(target_cpu as u8, 0xF0, 0);
        }
    }
    if is_fatal {
        let fg = crate::shell::FOREGROUND_TASK.load(Ordering::SeqCst);
        if fg == pid.0 {
            crate::shell::FOREGROUND_TASK.store(0, Ordering::SeqCst);
            crate::drivers::framebuffer::leave_alt_screen_if_active();
        }
    }
    if killed {
        wake_waiters_for(pid);
        wake_reaper_if_needed();
    }
}
