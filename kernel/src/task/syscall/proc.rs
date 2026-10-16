use crate::task::syscall::{
    copy_to_user, copy_to_user_slice, copy_user_string, validate_user_buffer, MAX_PATH_BYTES,
    MAX_PROCESS_INFO_ENTRIES,
};
pub fn sys_process_spawn(path_ptr: u64, path_len: usize) -> u64 {
    if path_len == 0 || path_len > MAX_PATH_BYTES {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    let path_str = match copy_user_string(path_ptr as *const u8, path_len) {
        Ok(s) => s,
        Err(_) => return -(lix_sys::ErrorCode::EFAULT as i32) as u64,
    };
    let file_data = crate::fs::vfs::read_file(&path_str);
    let launcher_rights = crate::task::scheduler::with_current_process(|current| current.rights)
        .unwrap_or(lix_sys::Rights::NONE);
    if let Some(data) = file_data {
        match crate::task::elf::load_elf(&data, &path_str, 3, launcher_rights) {
            Ok(pid) => {
                let current_pid = crate::task::scheduler::current_pid();
                let fg = crate::shell::FOREGROUND_TASK.load(core::sync::atomic::Ordering::SeqCst);
                if fg == current_pid || fg == 0 {
                    crate::shell::FOREGROUND_TASK.store(pid, core::sync::atomic::Ordering::SeqCst);
                }
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
        curr.exit_code = code;
    });
    crate::task::scheduler::record_exit_status(pid, code);
    crate::task::scheduler::wake_waiters_for(crate::task::process::ProcessId(pid));
    let r_pid = crate::task::scheduler::REAPER_PID.load(core::sync::atomic::Ordering::Relaxed);
    if r_pid != 0 {
        crate::task::scheduler::wake_process(r_pid);
    }
    crate::task::scheduler::yield_current();
    loop {
        core::hint::spin_loop();
    }
}
pub fn sys_process_kill(target_pid: u64, signal: u8) -> u64 {
    if signal >= u64::BITS as u8 {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    let (current_priv, current_rights, current_pid) =
        crate::task::scheduler::with_current_process(|p| (p.privilege_level, p.rights, p.id.0))
            .unwrap_or((0, lix_sys::Rights::NONE, 0));
    let mut allowed = false;
    let mut found = false;
    crate::task::scheduler::for_each_process(|proc| {
        if proc.id.0 == target_pid {
            found = true;
            let is_self = target_pid == current_pid;
            let is_child = proc.parent_pid != 0 && proc.parent_pid == current_pid;
            let can_manage = current_rights.contains(lix_sys::Rights::MANAGE);
            if (is_self || is_child || can_manage) && current_priv <= proc.privilege_level {
                allowed = true;
            }
        }
    });
    if !found {
        -(lix_sys::ErrorCode::ENOENT as i32) as u64
    } else if !allowed {
        -(lix_sys::ErrorCode::EPERM as i32) as u64
    } else {
        crate::task::scheduler::send_signal(crate::task::process::ProcessId(target_pid), signal);
        if target_pid == crate::task::scheduler::current_pid() {
            crate::task::scheduler::yield_current();
        }
        0
    }
}
pub fn sys_process_list(buffer: u64, max_count: usize) -> u64 {
    let required_bytes =
        max_count.saturating_mul(core::mem::size_of::<crate::task::process::ProcessInfo>());
    if max_count == 0
        || max_count > MAX_PROCESS_INFO_ENTRIES
        || validate_user_buffer(buffer, required_bytes, true).is_err()
    {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    let mut collected: alloc::vec::Vec<crate::task::process::ProcessInfo> =
        alloc::vec::Vec::with_capacity(max_count.min(32));
    crate::task::scheduler::for_each_process(|proc| {
        if collected.len() >= max_count {
            return;
        }
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
        collected.push(crate::task::process::ProcessInfo {
            pid: proc.id.0,
            name: name_buf,
            state: state_val,
            privilege_level: proc.privilege_level,
            cpu_ticks: proc.cpu_ticks,
            ram_bytes: proc.ram_bytes_used,
            disk_read_bytes: proc.disk_read_bytes,
        });
    });
    if collected.is_empty() {
        return 0;
    }
    let byte_len = collected.len() * core::mem::size_of::<crate::task::process::ProcessInfo>();
    let bytes = unsafe { core::slice::from_raw_parts(collected.as_ptr().cast::<u8>(), byte_len) };
    if copy_to_user_slice(buffer as *mut u8, bytes).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    collected.len() as u64
}
pub fn sys_waitpid(target_pid: u64, status_ptr: u64) -> u64 {
    let current_pid = crate::task::scheduler::current_pid();
    if target_pid == current_pid {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    if status_ptr != 0 && validate_user_buffer(status_ptr, 4, true).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    let current_rights =
        crate::task::scheduler::with_current_process(|p| p.rights).unwrap_or(lix_sys::Rights::NONE);
    let mut target_parent = None;
    crate::task::scheduler::for_each_process(|proc| {
        if proc.id.0 == target_pid {
            target_parent = Some(proc.parent_pid);
        }
    });
    if let Some(parent) = target_parent {
        let is_child = parent != 0 && parent == current_pid;
        if !is_child && !current_rights.contains(lix_sys::Rights::MANAGE) {
            return -(lix_sys::ErrorCode::EPERM as i32) as u64;
        }
    }
    loop {
        if let Some(exit_code) = crate::task::scheduler::take_exit_status(target_pid) {
            crate::shell::FOREGROUND_TASK.store(0, core::sync::atomic::Ordering::SeqCst);
            crate::drivers::framebuffer::leave_alt_screen_if_active();
            if status_ptr != 0 {
                if unsafe { copy_to_user(status_ptr as *mut i32, &exit_code) }.is_err() {
                    return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
                }
            }
            return target_pid;
        }
        let is_alive = crate::task::scheduler::is_process_alive(target_pid);
        if !is_alive {
            crate::shell::FOREGROUND_TASK.store(0, core::sync::atomic::Ordering::SeqCst);
            crate::drivers::framebuffer::leave_alt_screen_if_active();
            if status_ptr != 0 {
                let code = 0i32;
                if unsafe { copy_to_user(status_ptr as *mut i32, &code) }.is_err() {
                    return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
                }
            }
            return target_pid;
        }
        crate::task::scheduler::with_current_process(|current| {
            current.waiting_for = Some(crate::task::process::ProcessId(target_pid));
            current.state = crate::task::process::ProcessState::Blocked;
        });
        crate::task::scheduler::yield_current();
    }
}
pub fn sys_sleep(ms: u64) -> u64 {
    let ticks = ms.saturating_add(9).checked_div(10).unwrap_or(1).max(1);
    let target_tick = crate::task::scheduler::total_ticks() + ticks;
    crate::task::scheduler::with_current_process(|curr| {
        if curr.state != crate::task::process::ProcessState::Dead {
            curr.state = crate::task::process::ProcessState::Sleeping(target_tick);
        }
    });
    crate::task::scheduler::yield_current();
    0
}
