#[unsafe(naked)]
pub extern "C" fn syscall_handler_asm() {
    core::arch::naked_asm!(
        "push rax", "push rcx", "push rdx", "push rbx",
        "push rbp", "push rsi", "push rdi",
        "push r8", "push r9", "push r10", "push r11",
        "push r12", "push r13", "push r14", "push r15",
        
        "mov rdi, rsp",
        "and rsp, -16",
        "call syscall_handler_rust",
        "mov rsp, rax",
        
        "pop r15", "pop r14", "pop r13", "pop r12",
        "pop r11", "pop r10", "pop r9", "pop r8",
        "pop rdi", "pop rsi", "pop rbp",
        "pop rbx", "pop rdx", "pop rcx", "pop rax",
        "iretq",
    );
}

#[no_mangle]
pub extern "C" fn syscall_handler_rust(stack_pointer: u64) -> u64 {
    let context = unsafe { &mut *(stack_pointer as *mut crate::task::process::InterruptContext) };
    
    let sys_num = context.rax;
    
    match sys_num {
        1 | 0x302 => { // sys_write
            let buffer = context.rsi as *const u8;
            let len = context.rdx as usize;
            
            if (buffer as u64) < 0x0000800000000000 {
                let slice = unsafe { core::slice::from_raw_parts(buffer, len) };
                if let Ok(s) = core::str::from_utf8(slice) {
                    crate::kprint!("{}", s);
                    context.rax = len as u64;
                } else {
                    context.rax = 0;
                }
            } else {
                context.rax = 0;
            }
        }
        0x301 => { // sys_read
            let fd = context.rdi;
            let buffer = context.rsi as *mut u8;
            let len = context.rdx as usize;
            
            if fd == 0 && (buffer as u64) < 0x0000800000000000 && len > 0 {
                let mut bytes_read = 0;
                let slice = unsafe { core::slice::from_raw_parts_mut(buffer, len) };
                let mut queue = crate::drivers::tty::STDIN.lock();
                
                while bytes_read < len {
                    if let Some(c) = queue.pop_front() {
                        let mut b = [0; 4];
                        let s = c.encode_utf8(&mut b);
                        if bytes_read + s.len() <= len {
                            slice[bytes_read..bytes_read+s.len()].copy_from_slice(s.as_bytes());
                            bytes_read += s.len();
                        } else {
                            queue.push_front(c);
                            break;
                        }
                    } else {
                        break;
                    }
                }
                
                context.rax = bytes_read as u64;
            } else {
                context.rax = 0;
            }
        }
        35 | 0x107 => { // sys_sleep (35 legacy, 0x107 LixOS)
            let ms = context.rdi;
            let mut sched = crate::task::scheduler::SCHEDULER.lock();
            let target_tick = sched.total_ticks + ms;
            sched.current.as_mut().unwrap().state = crate::task::process::ProcessState::Sleeping(target_tick);
            drop(sched);
            crate::task::scheduler::yield_current();
            context.rax = 0;
        }
        60 | 0x101 => { // sys_exit
            crate::kprintln!("[syscall] Proceso {} terminó con código {}", 
                             crate::task::scheduler::SCHEDULER.lock().current.as_ref().unwrap().id.0,
                             context.rdi);
            crate::task::scheduler::SCHEDULER.lock().current.as_mut().unwrap().state = crate::task::process::ProcessState::Dead;
            crate::task::scheduler::yield_current();
        }
        0x102 => { // sys_kill
            let target_pid = context.rdi;
            let signal = context.rsi as u8;
            
            let mut sched = crate::task::scheduler::SCHEDULER.lock();
            let current_priv = sched.current.as_ref().map_or(0, |p| p.privilege_level);
            
            let mut allowed = false;
            let mut found = false;
            
            for proc in sched.processes.iter() {
                if proc.id.0 == target_pid {
                    found = true;
                    if current_priv <= proc.privilege_level {
                        allowed = true;
                    }
                    break;
                }
            }
            if !found {
                if let Some(curr) = &sched.current {
                    if curr.id.0 == target_pid {
                        found = true;
                        if current_priv <= curr.privilege_level {
                            allowed = true;
                        }
                    }
                }
            }
            drop(sched);
            
            if !found {
                context.rax = 2; // ENOENT
            } else if !allowed {
                context.rax = 1; // EPERM
            } else {
                crate::task::scheduler::send_signal(crate::task::process::ProcessId(target_pid), signal);
                context.rax = 0; // Success
            }
        }
        0x10C => { // sys_process_list
            let buffer = context.rdi as *mut crate::task::process::ProcessInfo;
            let max_count = context.rsi as usize;
            
            if (buffer as u64) < 0x0000800000000000 && max_count > 0 {
                let mut count = 0;
                let sched = crate::task::scheduler::SCHEDULER.lock();
                
                let slice = unsafe { core::slice::from_raw_parts_mut(buffer, max_count) };
                
                let mut write_info = |proc: &crate::task::process::Process| {
                    if count < max_count {
                        let state_val = match proc.state {
                            crate::task::process::ProcessState::Ready => 0,
                            crate::task::process::ProcessState::Running => 1,
                            crate::task::process::ProcessState::Blocked => 2,
                            crate::task::process::ProcessState::Sleeping(_) => 3,
                            crate::task::process::ProcessState::Zombie => 4,
                            crate::task::process::ProcessState::Dead => 5,
                        };
                        let mut name_buf = [0u8; 16];
                        let name_bytes = proc.name.as_bytes();
                        let copy_len = core::cmp::min(16, name_bytes.len());
                        name_buf[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
                        
                        slice[count] = crate::task::process::ProcessInfo {
                            pid: proc.id.0,
                            name: name_buf,
                            state: state_val,
                            privilege_level: proc.privilege_level,
                            cpu_ticks: proc.cpu_ticks,
                            ram_bytes: proc.ram_bytes_used,
                        };
                        count += 1;
                    }
                };
                
                for proc in sched.processes.iter() {
                    write_info(proc);
                }
                if let Some(curr) = &sched.current {
                    write_info(curr);
                }
                
                context.rax = count as u64;
            } else {
                context.rax = 0;
            }
        }
        _ => {
            crate::kprintln!("[syscall] Syscall no implementada: {}", sys_num);
            context.rax = 0xFFFFFFFFFFFFFFFF;
        }
    }
    
    stack_pointer
}