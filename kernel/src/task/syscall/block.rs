use crate::task::syscall::{validate_user_buffer, user_slice, user_slice_mut, copy_to_user};
use crate::drivers::block::{BlockDevice, DmaBuffer};

pub fn installer_privilege() -> bool {
    crate::task::scheduler::with_current_process(|process| process.privilege_level <= 1)
        .unwrap_or(false)
}

pub fn block_capability(
    handle: u64,
    rights: lix_sys::Rights,
) -> Result<alloc::sync::Arc<crate::task::handle::BlockDeviceCapability>, lix_sys::ErrorCode> {
    crate::task::scheduler::with_current_process(|process| {
        match process.handles.get(lix_sys::Handle(handle), rights)? {
            crate::task::handle::KernelObject::BlockDevice(ref capability) => {
                Ok(alloc::sync::Arc::clone(capability))
            }
            _ => Err(lix_sys::ErrorCode::EBADF),
        }
    }).unwrap_or(Err(lix_sys::ErrorCode::ESRCH))
}


pub fn block_transfer(start: u64, requested_count: u64, user_ptr: u64, write: bool) -> u64 {
    let count = match u16::try_from(requested_count) {
        Ok(count) if count != 0 => count,
        _ => return -(lix_sys::ErrorCode::EINVAL as i32) as u64,
    };
    let geometry = match crate::drivers::nvme::DISK.geometry() {
        Ok(geometry) => geometry,
        Err(_) => return -(lix_sys::ErrorCode::ENODEV as i32) as u64,
    };
    let Some(byte_len) = (count as usize).checked_mul(geometry.block_size as usize) else {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    };
    if geometry.block_size == 0
        || byte_len > crate::memory::pmm::PAGE_SIZE
        || start.checked_add(count as u64).is_none_or(|end| end > geometry.block_count)
    {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    if validate_user_buffer(user_ptr, byte_len, !write).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    let mut buffer = match DmaBuffer::allocate() {
        Ok(buffer) => buffer,
        Err(_) => return -(lix_sys::ErrorCode::ENOMEM as i32) as u64,
    };
    if write {
        let source = unsafe { user_slice(user_ptr as *const u8, byte_len) };
        let destination = buffer.as_mut_slice(byte_len);
        match (source, destination) {
            (Ok(source), Ok(destination)) => destination.copy_from_slice(source),
            _ => return -(lix_sys::ErrorCode::EFAULT as i32) as u64,
        }
        if crate::drivers::nvme::DISK.write_blocks(start, count, &buffer).is_err() {
            return -(lix_sys::ErrorCode::EIO as i32) as u64;
        }
    } else {
        if crate::drivers::nvme::DISK.read_blocks(start, count, &mut buffer).is_err() {
            return -(lix_sys::ErrorCode::EIO as i32) as u64;
        }
        let source = match buffer.as_slice(byte_len) {
            Ok(source) => source,
            Err(_) => return -(lix_sys::ErrorCode::EIO as i32) as u64,
        };
        let destination = unsafe { user_slice_mut(user_ptr as *mut u8, byte_len) };
        match destination {
            Ok(destination) => destination.copy_from_slice(source),
            Err(_) => return -(lix_sys::ErrorCode::EFAULT as i32) as u64,
        }
    }
    byte_len as u64
}

pub fn sys_block_open(arg1: u64, flags: u64) -> u64 {
    if !installer_privilege() {
        return -(lix_sys::ErrorCode::EPERM as i32) as u64;
    }
    if arg1 != 0
        || flags & !lix_sys::block_open_flags::VALID != 0
        || flags & lix_sys::block_open_flags::READ == 0
    {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    if crate::drivers::nvme::DISK.geometry().is_err() {
        return -(lix_sys::ErrorCode::ENODEV as i32) as u64;
    }
    let writable = flags & lix_sys::block_open_flags::WRITE != 0;
    if writable && crate::fs::ext4::is_mounted() {
        return -(lix_sys::ErrorCode::EBUSY as i32) as u64;
    }
    let capability = match crate::task::handle::BlockDeviceCapability::acquire(0, writable) {
        Ok(capability) => capability,
        Err(error) => return -(error as i32) as u64,
    };
    let mut rights = lix_sys::Rights::READ.union(lix_sys::Rights::INSPECT);
    if writable {
        rights = rights.union(lix_sys::Rights::WRITE);
    }
    let res = crate::task::scheduler::with_current_process(|process| {
        match process.handles.insert(
            crate::task::handle::KernelObject::BlockDevice(capability),
            rights,
        ) {
            Ok(handle) => handle.0,
            Err(error) => -(error as i32) as u64,
        }
    });
    res.unwrap_or(-(lix_sys::ErrorCode::ESRCH as i32) as u64)
}


pub fn sys_block_info(handle_val: u64, out_ptr: u64) -> u64 {
    let capability = match block_capability(handle_val, lix_sys::Rights::INSPECT) {
        Ok(capability) => capability,
        Err(error) => return -(error as i32) as u64,
    };
    if validate_user_buffer(out_ptr, core::mem::size_of::<lix_sys::BlockDeviceInfo>(), true).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    let geometry = match crate::drivers::nvme::DISK.geometry() {
        Ok(geometry) => geometry,
        Err(_) => return -(lix_sys::ErrorCode::ENODEV as i32) as u64,
    };
    let info = lix_sys::BlockDeviceInfo {
        block_count: geometry.block_count,
        block_size: geometry.block_size,
        flags: if capability.writable { 0 } else { lix_sys::BlockDeviceInfo::FLAG_READ_ONLY },
    };
    if unsafe { copy_to_user(out_ptr as *mut lix_sys::BlockDeviceInfo, &info) }.is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    0
}

pub fn sys_block_read(handle_val: u64, start: u64, count: u64, user_ptr: u64) -> u64 {
    let capability = match block_capability(handle_val, lix_sys::Rights::READ) {
        Ok(capability) => capability,
        Err(error) => return -(error as i32) as u64,
    };
    if capability.device_index != 0 {
        return -(lix_sys::ErrorCode::ENODEV as i32) as u64;
    }
    block_transfer(start, count, user_ptr, false)
}

pub fn sys_block_write(handle_val: u64, start: u64, count: u64, user_ptr: u64) -> u64 {
    let capability = match block_capability(handle_val, lix_sys::Rights::WRITE) {
        Ok(capability) => capability,
        Err(error) => return -(error as i32) as u64,
    };
    if !capability.writable || capability.device_index != 0 {
        return -(lix_sys::ErrorCode::EACCES as i32) as u64;
    }
    block_transfer(start, count, user_ptr, true)
}

pub fn sys_block_sync(handle_val: u64) -> u64 {
    if let Err(error) = block_capability(handle_val, lix_sys::Rights::WRITE) {
        return -(error as i32) as u64;
    }
    match crate::drivers::nvme::DISK.sync() {
        Ok(()) => 0,
        Err(_) => -(lix_sys::ErrorCode::EIO as i32) as u64,
    }
}
