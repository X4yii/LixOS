use crate::fs::permissions;
use crate::task::syscall::{
    copy_from_user_slice, copy_to_user_slice, copy_user_string, validate_user_buffer,
    MAX_PATH_BYTES, MAX_USER_IO_BYTES,
};
use lix_sys::ErrorCode;
fn errno(code: ErrorCode) -> u64 {
    -(code as i32) as u64
}
fn current_rights() -> lix_sys::Rights {
    crate::task::scheduler::with_current_process(|current| current.rights)
        .unwrap_or(lix_sys::Rights::NONE)
}
fn owns_console() -> bool {
    let fg = crate::shell::FOREGROUND_TASK.load(core::sync::atomic::Ordering::SeqCst);
    if fg == 0 {
        return true;
    }
    crate::task::scheduler::current_pid() == fg
}
fn user_path(path_ptr: u64, path_len: usize) -> Result<alloc::string::String, u64> {
    if path_len == 0 || path_len > MAX_PATH_BYTES {
        return Err(errno(ErrorCode::EINVAL));
    }
    copy_user_string(path_ptr as *const u8, path_len).map_err(|_| errno(ErrorCode::EFAULT))
}
fn user_bytes(ptr: u64, len: usize) -> Result<alloc::vec::Vec<u8>, u64> {
    if len > MAX_USER_IO_BYTES {
        return Err(errno(ErrorCode::EINVAL));
    }
    let mut buf = alloc::vec![0u8; len];
    copy_from_user_slice(ptr as *const u8, &mut buf).map_err(|_| errno(ErrorCode::EFAULT))?;
    Ok(buf)
}
pub fn sys_handle_close(handle_val: u64) -> u64 {
    let handle = lix_sys::Handle(handle_val);
    crate::task::scheduler::with_current_process(|curr| match curr.handles.close(handle) {
        Ok(()) => 0,
        Err(e) => errno(e),
    })
    .unwrap_or(errno(ErrorCode::ESRCH))
}
pub fn sys_handle_dup(handle_val: u64, rights_val: u64) -> u64 {
    let handle = lix_sys::Handle(handle_val);
    let requested_rights = lix_sys::Rights(rights_val as u32);
    crate::task::scheduler::with_current_process(|curr| {
        match curr.handles.dup(handle, requested_rights) {
            Ok(new_h) => new_h.0,
            Err(e) => errno(e),
        }
    })
    .unwrap_or(errno(ErrorCode::ESRCH))
}
pub fn sys_handle_dup2(src_handle_val: u64, dst_idx: u64) -> u64 {
    let src = lix_sys::Handle(src_handle_val);
    crate::task::scheduler::with_current_process(|curr| {
        match curr.handles.dup2(src, dst_idx as usize) {
            Ok(new_h) => new_h.0,
            Err(e) => errno(e),
        }
    })
    .unwrap_or(errno(ErrorCode::ESRCH))
}
pub fn sys_open(path_ptr: u64, path_len: u64, flags: u64) -> u64 {
    let path_str = match user_path(path_ptr, path_len as usize) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let is_write = (flags & 0x02) != 0 || (flags & 0x01) != 0;
    let is_create = (flags & 0x40) != 0;
    let auth_res = if is_write {
        permissions::authorize_write(&path_str, current_rights())
    } else {
        permissions::authorize_read(&path_str, current_rights())
    };
    if auth_res.is_err() {
        return errno(ErrorCode::EACCES);
    }
    if crate::fs::vfs::read_file(&path_str).is_none() {
        if is_create {
            if crate::fs::vfs::write_file(&path_str, &[]).is_err() {
                return errno(ErrorCode::EIO);
            }
        } else {
            return errno(ErrorCode::ENOENT);
        }
    }
    let mut rights = lix_sys::Rights::DUP.union(lix_sys::Rights::READ);
    if is_write {
        rights = rights.union(lix_sys::Rights::WRITE);
    }
    let obj = crate::task::handle::KernelObject::File {
        path: path_str,
        cursor: 0,
        is_write,
    };
    crate::task::scheduler::with_current_process(|curr| {
        match curr.handles.insert(obj, rights) {
            Ok(handle) => handle.0,
            Err(e) => errno(e),
        }
    })
    .unwrap_or(errno(ErrorCode::ESRCH))
}
pub fn sys_lseek(handle_val: u64, offset: i64, whence: u64) -> u64 {
    let handle = lix_sys::Handle(handle_val);
    crate::task::scheduler::with_current_process(|curr| {
        match curr.handles.get_mut(handle, lix_sys::Rights::NONE) {
            Ok(crate::task::handle::KernelObject::File { path, cursor, .. }) => {
                let file_len = crate::fs::vfs::read_file(path).map(|d| d.len() as i64).unwrap_or(0);
                let new_offset = match whence {
                    0 => offset,
                    1 => (*cursor as i64).checked_add(offset).ok_or(ErrorCode::EINVAL)?,
                    2 => file_len.checked_add(offset).ok_or(ErrorCode::EINVAL)?,
                    _ => return Err(ErrorCode::EINVAL),
                };
                if new_offset < 0 {
                    return Err(ErrorCode::EINVAL);
                }
                *cursor = new_offset as u64;
                Ok(new_offset as u64)
            }
            _ => Err(ErrorCode::EBADF),
        }
    })
    .unwrap_or(Err(ErrorCode::ESRCH))
    .unwrap_or_else(errno)
}
pub fn sys_write(handle_val: u64, buf_ptr: u64, len: u64) -> u64 {
    let len = len as usize;
    if len == 0 {
        return 0;
    }
    if len > MAX_USER_IO_BYTES {
        return errno(ErrorCode::EINVAL);
    }
    if validate_user_buffer(buf_ptr, len, false).is_err() {
        return errno(ErrorCode::EFAULT);
    }
    let handle = lix_sys::Handle(handle_val);
    let mut kbuf = alloc::vec![0u8; len];
    if copy_from_user_slice(buf_ptr as *const u8, &mut kbuf).is_err() {
        return errno(ErrorCode::EFAULT);
    }
    let res = crate::task::scheduler::with_current_process(|curr| {
        match curr.handles.get_mut(handle, lix_sys::Rights::WRITE) {
            Ok(crate::task::handle::KernelObject::File { path, cursor, is_write }) => {
                if !*is_write {
                    return Err(ErrorCode::EACCES);
                }
                let mut data = crate::fs::vfs::read_file(path).unwrap_or_default();
                let pos = *cursor as usize;
                if pos.checked_add(len).is_none() {
                    return Err(ErrorCode::EINVAL);
                }
                if pos + len > data.len() {
                    data.resize(pos + len, 0);
                }
                data[pos..pos + len].copy_from_slice(&kbuf);
                if crate::fs::vfs::write_file(path, &data).is_err() {
                    return Err(ErrorCode::EIO);
                }
                *cursor += len as u64;
                Ok(len as u64)
            }
            Ok(crate::task::handle::KernelObject::StdOut) | Ok(crate::task::handle::KernelObject::StdErr) => {
                match core::str::from_utf8(&kbuf) {
                    Ok(text) => {
                        crate::kprint!("{}", text);
                        Ok(len as u64)
                    }
                    Err(_) => Err(ErrorCode::EINVAL),
                }
            }
            _ => Err(ErrorCode::EBADF),
        }
    });
    match res {
        Some(Ok(written)) => written,
        Some(Err(e)) => errno(e),
        None => errno(ErrorCode::ESRCH),
    }
}
pub fn sys_read(fd: u64, buffer_ptr: u64, len: usize) -> u64 {
    let buffer = buffer_ptr as *mut u8;
    if len > MAX_USER_IO_BYTES || validate_user_buffer(buffer as u64, len, true).is_err() {
        return errno(ErrorCode::EFAULT);
    }
    let has_read_rights = if fd == 0 {
        true
    } else {
        let handle = lix_sys::Handle(fd);
        crate::task::scheduler::with_current_process(|curr| {
            curr.handles.get(handle, lix_sys::Rights::READ).is_ok()
        })
        .unwrap_or(false)
    };
    if !has_read_rights {
        return errno(ErrorCode::EACCES);
    }
    if len == 0 {
        return 0;
    }
    if fd == 0 && !owns_console() {
        return errno(ErrorCode::EAGAIN);
    }
    if fd != 0 {
        let file_read_res = crate::task::scheduler::with_current_process(|curr| {
            let handle = lix_sys::Handle(fd);
            match curr.handles.get_mut(handle, lix_sys::Rights::READ) {
                Ok(crate::task::handle::KernelObject::File { path, cursor, .. }) => {
                    let data = crate::fs::vfs::read_file(path).ok_or(ErrorCode::ENOENT)?;
                    let pos = *cursor as usize;
                    if pos >= data.len() {
                        return Ok(alloc::vec::Vec::new());
                    }
                    let to_read = len.min(data.len() - pos);
                    let chunk = data[pos..pos + to_read].to_vec();
                    *cursor += to_read as u64;
                    Ok(chunk)
                }
                _ => Err(ErrorCode::EBADF),
            }
        });
        if let Some(res) = file_read_res {
            match res {
                Ok(chunk) => {
                    if chunk.is_empty() {
                        return 0;
                    }
                    if copy_to_user_slice(buffer, &chunk).is_err() {
                        return errno(ErrorCode::EFAULT);
                    }
                    return chunk.len() as u64;
                }
                Err(ErrorCode::EBADF) => {}
                Err(e) => return errno(e),
            }
        }
        let input_class = crate::task::scheduler::with_current_process(|current| {
            match current
                .handles
                .get(lix_sys::Handle(fd), lix_sys::Rights::READ)
            {
                Ok(crate::task::handle::KernelObject::InputDevice { class }) => Some(*class),
                _ => None,
            }
        })
        .flatten();
        if let Some(class) = input_class {
            let event_size = core::mem::size_of::<crate::drivers::input::InputEvent>();
            if len < event_size
                || buffer_ptr as usize % core::mem::align_of::<crate::drivers::input::InputEvent>()
                    != 0
            {
                return errno(ErrorCode::EINVAL);
            }
            let capacity = core::cmp::min(len / event_size, 64);
            let out = unsafe {
                core::slice::from_raw_parts_mut(
                    buffer as *mut crate::drivers::input::InputEvent,
                    capacity,
                )
            };
            let mut count = 0;
            while count < capacity {
                let Some(event) = crate::drivers::input::pop_for_class(class) else {
                    break;
                };
                out[count] = event;
                count += 1;
            }
            return (count * event_size) as u64;
        }
    }
    let mut bytes_read = 0;
    while bytes_read < len {
        crate::drivers::serial::drain_to_stdin();
        crate::drivers::xhci::poll();
        let is_dead = crate::task::scheduler::with_current_process(|curr| {
            curr.state == crate::task::process::ProcessState::Dead
        })
        .unwrap_or(false);
        if is_dead {
            crate::task::scheduler::yield_current();
            return 0;
        }
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
            if copy_to_user_slice(destination, s.as_bytes()).is_err() {
                return errno(ErrorCode::EFAULT);
            }
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
    if arg1 != 0
        || len > MAX_USER_IO_BYTES
        || validate_user_buffer(buffer as u64, len, true).is_err()
    {
        return errno(ErrorCode::EINVAL);
    }
    if len == 0 {
        return 0;
    }
    if !owns_console() {
        return errno(ErrorCode::EAGAIN);
    }
    crate::drivers::serial::drain_to_stdin();
    let is_dead = crate::task::scheduler::with_current_process(|curr| {
        curr.state == crate::task::process::ProcessState::Dead
    })
    .unwrap_or(false);
    if is_dead {
        crate::task::scheduler::yield_current();
        return 0;
    }
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
        if copy_to_user_slice(destination, bytes).is_err() {
            return errno(ErrorCode::EFAULT);
        }
        bytes_read += bytes.len();
    }
    bytes_read as u64
}
pub fn sys_list_dir(
    path_ptr: u64,
    path_len: usize,
    out_ptr: u64,
    out_len: usize,
) -> Result<u64, u64> {
    let path_str = user_path(path_ptr, path_len)?;
    if out_len == 0 || out_len > MAX_USER_IO_BYTES {
        return Err(errno(ErrorCode::EINVAL));
    }
    if validate_user_buffer(out_ptr, out_len, true).is_err() {
        return Err(errno(ErrorCode::EFAULT));
    }
    permissions::authorize_read(&path_str, current_rights())?;
    let files = crate::fs::vfs::list_files(&path_str);
    let mut kbuf = alloc::vec::Vec::new();
    for f in files {
        let f_bytes = f.as_bytes();
        if kbuf.len() + f_bytes.len() + 1 <= out_len {
            kbuf.extend_from_slice(f_bytes);
            kbuf.push(b'\n');
        }
    }
    copy_to_user_slice(out_ptr as *mut u8, &kbuf).map_err(|_| errno(ErrorCode::EFAULT))?;
    Ok(kbuf.len() as u64)
}
pub fn sys_read_file(
    path_ptr: u64,
    path_len: usize,
    out_ptr: u64,
    max_len: usize,
) -> Result<u64, u64> {
    let path_str = user_path(path_ptr, path_len)?;
    if max_len == 0 || max_len > MAX_USER_IO_BYTES {
        return Err(errno(ErrorCode::EINVAL));
    }
    if validate_user_buffer(out_ptr, max_len, true).is_err() {
        return Err(errno(ErrorCode::EFAULT));
    }
    permissions::authorize_read(&path_str, current_rights())?;
    let Some(data) = crate::fs::vfs::read_file(&path_str) else {
        return Err(errno(ErrorCode::ENOENT));
    };
    let copy_len = core::cmp::min(data.len(), max_len);
    copy_to_user_slice(out_ptr as *mut u8, &data[..copy_len])
        .map_err(|_| errno(ErrorCode::EFAULT))?;
    Ok(copy_len as u64)
}
pub fn sys_write_file(
    path_ptr: u64,
    path_len: usize,
    data_ptr: u64,
    data_len: usize,
) -> Result<u64, u64> {
    let path_str = user_path(path_ptr, path_len)?;
    let data = user_bytes(data_ptr, data_len)?;
    permissions::authorize_write(&path_str, current_rights())?;
    match crate::fs::vfs::write_file(&path_str, &data) {
        Ok(()) => Ok(data_len as u64),
        Err(error) => Err(errno(error.code())),
    }
}
pub fn sys_remove_file(path_ptr: u64, path_len: usize, recursive: u64) -> Result<u64, u64> {
    let path_str = user_path(path_ptr, path_len)?;
    permissions::authorize_remove(&path_str, current_rights())?;
    match crate::fs::vfs::remove_file(&path_str, recursive != 0) {
        Ok(()) => Ok(0),
        Err(error) => Err(errno(error.code())),
    }
}
pub fn sys_mkdir(path_ptr: u64, path_len: usize) -> Result<u64, u64> {
    let path_str = user_path(path_ptr, path_len)?;
    permissions::authorize_write(&path_str, current_rights())?;
    match crate::fs::vfs::mkdir(&path_str) {
        Ok(()) => Ok(0),
        Err(error) => Err(errno(error.code())),
    }
}
pub fn sys_log_write(buf_ptr: u64, len: u64) -> Result<u64, u64> {
    let len = len as usize;
    if len == 0 {
        return Ok(0);
    }
    let data = user_bytes(buf_ptr, len)?;
    match core::str::from_utf8(&data) {
        Ok(text) => {
            crate::klog::write_str(text);
            Ok(len as u64)
        }
        Err(_) => Err(errno(ErrorCode::EINVAL)),
    }
}
