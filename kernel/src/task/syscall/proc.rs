use crate::task::syscall::{validate_user_buffer, user_slice, copy_to_user, MAX_PATH_BYTES, MAX_PROCESS_INFO_ENTRIES};

pub fn sys_process_spawn(path_ptr: *const u8, path_len: usize) -> u64 {
    if path_len == 0 || path_len > MAX_PATH_BYTES || validate_user_buffer(path_ptr as u64, path_len, false).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    let slice = unsafe { user_slice(path_ptr, path_len).unwrap_or(&[]) };
    let path_str = match core::str::from_utf8(slice) {
        Ok(s) => s,
        Err(_) => return -(lix_sys::ErrorCode::EINVAL as i32) as u64,
    };

    let file_data = crate::fs::vfs::read_file(path_str);
    if let Some(data) = file_data {
        match crate::task::elf::load_elf(&data, path_str, 3) {
            Ok(pid) => {
                crate::shell::FOREGROUND_TASK.store(pid, core::sync::atomic::Ordering::SeqCst);
                pid
            }
            Err(_) => -(lix_sys::ErrorCode::ENOEXEC as i32) as u64,
        }
    } else {
        -(lix_sys::ErrorCode::ENOENT as i32) as u64
    }
}

pub fn sys_process_exit(code: i32) -> u64 {
    let pid = crate::task::scheduler::current_pid();
    crate::kprintln!("[syscall] Proceso {} terminó con código {}", pid, code);
    let fg = crate::shell::FOREGROUND_TASK.load(core::sync::atomic::Ordering::SeqCst);
    if fg == pid {
        crate::shell::FOREGROUND_TASK.store(0, core::sync::atomic::Ordering::SeqCst);
        crate::drivers::framebuffer::leave_alt_screen_if_active();
    }
    crate::task::scheduler::with_current_process(|curr| {
        curr.state = crate::task::process::ProcessState::Dead;
    });
    let r_pid = crate::task::scheduler::REAPER_PID.load(core::sync::atomic::Ordering::Relaxed);
    if r_pid != 0 {
        crate::task::scheduler::wake_process(r_pid);
    }
    crate::task::scheduler::yield_current();
    0
}

pub fn sys_process_kill(target_pid: u64, signal: u8) -> u64 {
    if signal >= u64::BITS as u8 {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }

    let current_priv = crate::task::scheduler::with_current_process(|p| p.privilege_level).unwrap_or(0);

    let mut allowed = false;
    let mut found = false;

    crate::task::scheduler::for_each_process(|proc| {
        if proc.id.0 == target_pid {
            found = true;
            if current_priv <= proc.privilege_level {
                allowed = true;
            }
        }
    });

    if !found {
        2
    } else if !allowed {
        1
    } else {
        crate::task::scheduler::send_signal(crate::task::process::ProcessId(target_pid), signal);
        0
    }
}

pub fn sys_process_list(buffer: *mut crate::task::process::ProcessInfo, max_count: usize) -> u64 {
    let required_bytes = max_count.saturating_mul(core::mem::size_of::<crate::task::process::ProcessInfo>());
    if max_count == 0 || max_count > MAX_PROCESS_INFO_ENTRIES || validate_user_buffer(buffer as u64, required_bytes, true).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }

    let mut count = 0;
    let output = buffer as *mut u8;

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

            let info = crate::task::process::ProcessInfo {
                pid: proc.id.0,
                name: name_buf,
                state: state_val,
                privilege_level: proc.privilege_level,
                cpu_ticks: proc.cpu_ticks,
                ram_bytes: proc.ram_bytes_used,
                disk_read_bytes: proc.disk_read_bytes,
            };
            unsafe {
                core::ptr::write_unaligned(
                    output.add(count * core::mem::size_of::<crate::task::process::ProcessInfo>()) as *mut crate::task::process::ProcessInfo,
                    info,
                );
            }
            count += 1;
        }
    };

    crate::task::scheduler::for_each_process(|proc| {
        write_info(proc);
    });

    count as u64
}

pub fn sys_waitpid(target_pid: u64, status_ptr: *mut i32) -> u64 {
    let current_pid = crate::task::scheduler::current_pid();
    if target_pid == current_pid {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    if status_ptr as u64 != 0 && validate_user_buffer(status_ptr as u64, 4, true).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }

    loop {
        let should_wait = {
            let mut target_alive = false;
            crate::task::scheduler::for_each_process(|process| {
                if process.id.0 == target_pid
                    && process.state != crate::task::process::ProcessState::Dead
                    && process.state != crate::task::process::ProcessState::Zombie
                {
                    target_alive = true;
                }
            });
            if target_alive {
                crate::task::scheduler::with_current_process(|current| {
                    current.waiting_for = Some(crate::task::process::ProcessId(target_pid));
                    current.state = crate::task::process::ProcessState::Blocked;
                });
            }
            target_alive
        };

        if !should_wait {
            crate::shell::FOREGROUND_TASK.store(0, core::sync::atomic::Ordering::SeqCst);
            crate::drivers::framebuffer::leave_alt_screen_if_active();
            if status_ptr as u64 != 0 {
                if unsafe { copy_to_user(status_ptr, &0i32) }.is_err() {
                    return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
                }
            }
            return target_pid;
        }
        crate::task::scheduler::yield_current();
    }
}

pub fn sys_sleep(ms: u64) -> u64 {
    let ticks = ms.saturating_add(9).checked_div(10).unwrap_or(1).max(1);
    let target_tick = crate::task::scheduler::total_ticks() + ticks;
    crate::task::scheduler::with_current_process(|curr| {
        curr.state = crate::task::process::ProcessState::Sleeping(target_tick);
    });
    crate::task::scheduler::yield_current();
    0
}

