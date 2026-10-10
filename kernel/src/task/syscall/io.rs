use crate::task::syscall::{validate_user_buffer, user_slice, user_slice_mut, MAX_PATH_BYTES, MAX_USER_IO_BYTES};

pub fn sys_handle_close(handle_val: u64) -> u64 {
    let handle = lix_sys::Handle(handle_val);
    crate::task::scheduler::with_current_process(|curr| {
        match curr.handles.close(handle) {
            Ok(()) => 0,
            Err(e) => -(e as i32) as u64,
        }
    }).unwrap_or(-(lix_sys::ErrorCode::ESRCH as i32) as u64)
}

pub fn sys_handle_dup(handle_val: u64, rights_val: u64) -> u64 {
    let handle = lix_sys::Handle(handle_val);
    let requested_rights = lix_sys::Rights(rights_val as u32);
    crate::task::scheduler::with_current_process(|curr| {
        match curr.handles.dup(handle, requested_rights) {
            Ok(new_h) => new_h.0,
            Err(e) => -(e as i32) as u64,
        }
    }).unwrap_or(-(lix_sys::ErrorCode::ESRCH as i32) as u64)
}

pub fn sys_write(arg1: u64, arg2: u64, arg3: u64) -> u64 {
    let (handle_val, buf_ptr, len) = if arg1 >= 0x1000 && arg1 < 0x0000800000000000 {
        (1u64, arg1, arg2 as usize)
    } else {
        (arg1, arg2, arg3 as usize)
    };

    if len == 0 {
        return 0;
    }

    if len > MAX_USER_IO_BYTES {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    if validate_user_buffer(buf_ptr, len, false).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }

    let handle = lix_sys::Handle(handle_val);
    let has_write_rights = crate::task::scheduler::with_current_process(|curr| {
        curr.handles.get(handle, lix_sys::Rights::WRITE).is_ok()
    }).unwrap_or(true);

    if !has_write_rights {
        return -(lix_sys::ErrorCode::EACCES as i32) as u64;
    }

    let slice = unsafe { user_slice(buf_ptr as *const u8, len).unwrap_or(&[]) };
    if let Ok(s) = core::str::from_utf8(slice) {
        crate::kprint!("{}", s);
        len as u64
    } else {
        0
    }
}

pub fn sys_read(fd: u64, buffer_ptr: u64, len: usize) -> u64 {
    let buffer = buffer_ptr as *mut u8;
    if len > MAX_USER_IO_BYTES || validate_user_buffer(buffer as u64, len, true).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }

    let has_read_rights = if fd == 0 {
        true
    } else {
        let handle = lix_sys::Handle(fd);
        crate::task::scheduler::with_current_process(|curr| {
            curr.handles.get(handle, lix_sys::Rights::READ).is_ok()
        }).unwrap_or(true)
    };

    if !has_read_rights {
        return -(lix_sys::ErrorCode::EACCES as i32) as u64;
    }

    if len == 0 {
        return 0;
    }

    let mut bytes_read = 0;
    while bytes_read < len {
        crate::drivers::serial::drain_to_stdin();
        let mut queue = crate::drivers::tty::STDIN.lock();
        if let Some(c) = queue.pop_front() {
            let mut b = [0; 4];
            let s = c.encode_utf8(&mut b);
            if bytes_read.checked_add(s.len()).is_none_or(|end| end > len) {
                queue.push_front(c);
                break;
            }
            drop(queue);
            let destination = unsafe { buffer.add(bytes_read) };
            let output = match unsafe { user_slice_mut(destination, s.len()) } {
                Ok(output) => output,
                Err(_) => return -(lix_sys::ErrorCode::EFAULT as i32) as u64,
            };
            output.copy_from_slice(s.as_bytes());
            bytes_read += s.len();
        } else {
            if bytes_read > 0 {
                break;
            }
            crate::task::scheduler::with_current_process(|current| {
                current.state = crate::task::process::ProcessState::Blocked;
            });
            drop(queue);
            crate::task::scheduler::yield_current();
        }
    }
    bytes_read as u64
}


pub fn sys_read_nonblocking(arg1: u64, buffer_ptr: u64, len: usize) -> u64 {
    let buffer = buffer_ptr as *mut u8;
    if arg1 != 0 || len > MAX_USER_IO_BYTES || validate_user_buffer(buffer as u64, len, true).is_err() {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    if len == 0 {
        return 0;
    }

    crate::drivers::serial::drain_to_stdin();
    let mut bytes_read = 0;
    let mut queue = crate::drivers::tty::STDIN.lock();
    while let Some(c) = queue.pop_front() {
        let mut encoded = [0; 4];
        let bytes = c.encode_utf8(&mut encoded).as_bytes();
        if bytes_read + bytes.len() > len {
            queue.push_front(c);
            break;
        }
        let destination = unsafe { buffer.add(bytes_read) };
        let output = match unsafe { user_slice_mut(destination, bytes.len()) } {
            Ok(output) => output,
            Err(_) => return -(lix_sys::ErrorCode::EFAULT as i32) as u64,
        };
        output.copy_from_slice(bytes);
        bytes_read += bytes.len();
    }
    bytes_read as u64
}

pub fn sys_list_dir(path_ptr: *const u8, path_len: usize, out_buf: *mut u8) -> u64 {
    let max_len = 1024usize;
    if path_len == 0 || path_len > MAX_PATH_BYTES || validate_user_buffer(path_ptr as u64, path_len, false).is_err() || validate_user_buffer(out_buf as u64, max_len, true).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    let slice = unsafe { user_slice(path_ptr, path_len).unwrap_or(&[]) };
    let path_str = match core::str::from_utf8(slice) {
        Ok(s) => s,
        Err(_) => return -(lix_sys::ErrorCode::EINVAL as i32) as u64,
    };
    let files = crate::fs::vfs::list_files(path_str);
    let mut written = 0;
    let out_slice = unsafe { user_slice_mut(out_buf, max_len).unwrap_or(&mut []) };
    for f in files {
        let f_bytes = f.as_bytes();
        if written + f_bytes.len() + 1 <= max_len {
            out_slice[written..written + f_bytes.len()].copy_from_slice(f_bytes);
            written += f_bytes.len();
            out_slice[written] = b'\n';
            written += 1;
        }
    }
    written as u64
}

pub fn sys_read_file(path_ptr: *const u8, path_len: usize, out_buf: *mut u8, max_len: usize) -> u64 {
    if path_len == 0 || path_len > MAX_PATH_BYTES || max_len > MAX_USER_IO_BYTES || validate_user_buffer(path_ptr as u64, path_len, false).is_err() || validate_user_buffer(out_buf as u64, max_len, true).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    let slice = unsafe { user_slice(path_ptr, path_len).unwrap_or(&[]) };
    let path_str = match core::str::from_utf8(slice) {
        Ok(s) => s,
        Err(_) => return -(lix_sys::ErrorCode::EINVAL as i32) as u64,
    };
    let file_data = crate::fs::vfs::read_file(path_str);
    if let Some(data) = file_data {
        let copy_len = core::cmp::min(data.len(), max_len);
        let out_slice = unsafe { user_slice_mut(out_buf, max_len).unwrap_or(&mut []) };
        out_slice[..copy_len].copy_from_slice(&data[..copy_len]);
        copy_len as u64
    } else {
        -(lix_sys::ErrorCode::ENOENT as i32) as u64
    }
}
