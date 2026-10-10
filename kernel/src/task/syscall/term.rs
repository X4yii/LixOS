use crate::task::syscall::{validate_user_buffer, copy_to_user};

pub fn sys_terminal_info(out_ptr_val: u64) -> u64 {
    let out_ptr = out_ptr_val as *mut lix_sys::TerminalInfo;
    if validate_user_buffer(out_ptr as u64, core::mem::size_of::<lix_sys::TerminalInfo>(), true).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    if let Some(info) = crate::drivers::framebuffer::get_terminal_info() {
        if unsafe { copy_to_user(out_ptr, &info) }.is_err() {
            return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
        }
        0
    } else {
        -(lix_sys::ErrorCode::ENODEV as i32) as u64
    }
}

pub fn sys_terminal_set_scale(cmd: i64) -> u64 {
    match cmd {
        0 => {
            crate::drivers::framebuffer::zoom_reset();
        }
        1..=4 => {
            crate::drivers::framebuffer::set_scale(cmd as usize);
        }
        10 => {
            crate::drivers::framebuffer::zoom_in();
        }
        11 => {
            crate::drivers::framebuffer::zoom_out();
        }
        _ => return -(lix_sys::ErrorCode::EINVAL as i32) as u64,
    }
    0
}

pub fn sys_set_keymap(id: u8) -> u64 {
    if let Some(kind) = crate::drivers::keyboard::KeyboardLayoutKind::from_u8(id) {
        crate::drivers::keyboard::set_layout(kind);
        0
    } else {
        -(lix_sys::ErrorCode::EINVAL as i32) as u64
    }
}

pub fn sys_get_keymap() -> u64 {
    crate::drivers::keyboard::get_layout() as u64
}

pub fn sys_monitor_metrics(out_ptr_val: u64) -> u64 {
    let out = out_ptr_val as *mut lix_sys::SystemMetrics;
    if validate_user_buffer(out as u64, core::mem::size_of::<lix_sys::SystemMetrics>(), true).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    let scheduler_ticks = crate::task::scheduler::SCHEDULER.lock().total_ticks;
    let (total_ram_bytes, free_ram_bytes) = crate::memory::pmm::memory_stats();
    let disk = crate::fs::vfs::disk_io_stats();
    let metrics = lix_sys::SystemMetrics {
        scheduler_ticks,
        total_ram_bytes,
        free_ram_bytes,
        disk_read_ops: disk.read_ops,
        disk_read_bytes: disk.read_bytes,
        disk_mounted: u64::from(disk.mounted),
    };
    if unsafe { copy_to_user(out, &metrics) }.is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    0
}

pub fn sys_power() {
    crate::power::system_shutdown();
}
