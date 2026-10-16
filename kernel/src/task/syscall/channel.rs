use crate::task::channel::{ChannelEndpoint, ChannelMessage};
use crate::task::handle::KernelObject;
use crate::task::syscall::{copy_from_user_slice, copy_to_user_slice, validate_user_buffer};
use alloc::vec::Vec;
use lix_sys::{ErrorCode, Handle, Rights};
fn errno(code: ErrorCode) -> u64 {
    -(code as i32) as u64
}
pub fn sys_channel_create(out_handles_ptr: u64) -> u64 {
    if validate_user_buffer(out_handles_ptr, 16, true).is_err() {
        return errno(ErrorCode::EFAULT);
    }
    let (end_a, end_b) = ChannelEndpoint::new_pair();
    let rights = Rights(Rights::READ.0 | Rights::WRITE.0 | Rights::TRANSFER.0 | Rights::DUP.0 | Rights::WAIT.0);
    let res = crate::task::scheduler::with_current_process(|curr| {
        let h_a = curr.handles.insert(KernelObject::Channel(end_a), rights)?;
        let h_b = curr.handles.insert(KernelObject::Channel(end_b), rights)?;
        Ok([h_a.0, h_b.0])
    });
    match res {
        Some(Ok(handles)) => {
            let bytes: [u8; 16] = unsafe { core::mem::transmute(handles) };
            if copy_to_user_slice(out_handles_ptr as *mut u8, &bytes).is_err() {
                return errno(ErrorCode::EFAULT);
            }
            0
        }
        Some(Err(e)) => errno(e),
        None => errno(ErrorCode::ESRCH),
    }
}
pub fn sys_channel_send(
    handle_val: u64,
    data_ptr: u64,
    data_len: u64,
    handles_ptr: u64,
    handles_len: u64,
) -> u64 {
    let dlen = data_len as usize;
    let hlen = handles_len as usize;
    if dlen > 64 * 1024 || hlen > 16 {
        return errno(ErrorCode::EINVAL);
    }
    let mut data = alloc::vec![0u8; dlen];
    if dlen > 0 {
        if copy_from_user_slice(data_ptr as *const u8, &mut data).is_err() {
            return errno(ErrorCode::EFAULT);
        }
    }
    let mut transferred_raw_handles = alloc::vec![0u64; hlen];
    if hlen > 0 {
        let byte_len = hlen * 8;
        let mut handle_bytes = alloc::vec![0u8; byte_len];
        if copy_from_user_slice(handles_ptr as *const u8, &mut handle_bytes).is_err() {
            return errno(ErrorCode::EFAULT);
        }
        for (i, chunk) in handle_bytes.chunks_exact(8).enumerate() {
            transferred_raw_handles[i] = u64::from_le_bytes(chunk.try_into().unwrap());
        }
    }
    let channel_handle = Handle(handle_val);
    let res = crate::task::scheduler::with_current_process(|curr| {
        let endpoint = match curr.handles.get(channel_handle, Rights::WRITE) {
            Ok(KernelObject::Channel(ep)) => ep.clone(),
            _ => return Err(ErrorCode::EBADF),
        };
        let mut bundled_handles = Vec::with_capacity(hlen);
        for &raw_h in &transferred_raw_handles {
            let h = Handle(raw_h);
            let extracted = curr.handles.take(h, Rights::TRANSFER)?;
            bundled_handles.push(extracted);
        }
        let msg = ChannelMessage {
            data,
            handles: bundled_handles,
        };
        endpoint.send(msg)
    });
    match res {
        Some(Ok(())) => 0,
        Some(Err(e)) => errno(e),
        None => errno(ErrorCode::ESRCH),
    }
}
pub fn sys_channel_recv(
    handle_val: u64,
    data_ptr: u64,
    max_data_len: u64,
    handles_ptr: u64,
    max_handles_len: u64,
) -> u64 {
    let max_dlen = max_data_len as usize;
    let max_hlen = max_handles_len as usize;
    let channel_handle = Handle(handle_val);
    let res = crate::task::scheduler::with_current_process(|curr| {
        let endpoint = match curr.handles.get(channel_handle, Rights::READ) {
            Ok(KernelObject::Channel(ep)) => ep.clone(),
            _ => return Err(ErrorCode::EBADF),
        };
        endpoint.recv()
    });
    let msg = match res {
        Some(Ok(m)) => m,
        Some(Err(e)) => return errno(e),
        None => return errno(ErrorCode::ESRCH),
    };
    if msg.data.len() > max_dlen || msg.handles.len() > max_hlen {
        return errno(ErrorCode::EINVAL);
    }
    if !msg.data.is_empty() {
        if copy_to_user_slice(data_ptr as *mut u8, &msg.data).is_err() {
            return errno(ErrorCode::EFAULT);
        }
    }
    if !msg.handles.is_empty() {
        let mut allocated_handles = Vec::with_capacity(msg.handles.len());
        let insert_res = crate::task::scheduler::with_current_process(|curr| {
            for (obj, rights) in msg.handles {
                let h = curr.handles.insert(obj, rights)?;
                allocated_handles.push(h.0);
            }
            Ok(())
        });
        if insert_res.is_none_or(|r: Result<(), ErrorCode>| r.is_err()) {
            return errno(ErrorCode::EMFILE);
        }
        let mut handle_bytes = Vec::with_capacity(allocated_handles.len() * 8);
        for h in allocated_handles {
            handle_bytes.extend_from_slice(&h.to_le_bytes());
        }
        if copy_to_user_slice(handles_ptr as *mut u8, &handle_bytes).is_err() {
            return errno(ErrorCode::EFAULT);
        }
    }
    msg.data.len() as u64
}
