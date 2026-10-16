use crate::drivers::block::{BlockDevice, DmaBuffer};
use crate::task::syscall::{
    copy_from_user_slice, copy_to_user, copy_to_user_slice, validate_user_buffer,
};
pub fn installer_privilege() -> bool {
    crate::task::scheduler::with_current_process(|process| {
        process.rights.contains(lix_sys::Rights::DEVICES)
    })
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
    })
    .unwrap_or(Err(lix_sys::ErrorCode::ESRCH))
}
pub fn block_transfer(
    device: &'static dyn BlockDevice,
    start: u64,
    requested_count: u64,
    user_ptr: u64,
    write: bool,
) -> u64 {
    let count = match u16::try_from(requested_count) {
        Ok(count) if count != 0 => count,
        _ => return -(lix_sys::ErrorCode::EINVAL as i32) as u64,
    };
    let geometry = match device.geometry() {
        Ok(geometry) => geometry,
        Err(_) => return -(lix_sys::ErrorCode::ENODEV as i32) as u64,
    };
    if geometry.block_size == 0
        || start
            .checked_add(count as u64)
            .is_none_or(|end| end > geometry.block_count)
    {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    let Some(byte_len) = (count as usize).checked_mul(geometry.block_size as usize) else {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    };
    if byte_len > crate::task::syscall::MAX_USER_IO_BYTES {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    if validate_user_buffer(user_ptr, byte_len, !write).is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    let blocks_per_chunk =
        (crate::memory::pmm::PAGE_SIZE / geometry.block_size as usize).max(1) as u16;
    let dev_key = device as *const dyn crate::drivers::block::BlockDevice as *const () as usize;
    let mut buffer = match DmaBuffer::allocate() {
        Ok(buffer) => buffer,
        Err(_) => return -(lix_sys::ErrorCode::ENOMEM as i32) as u64,
    };
    let mut done_blocks = 0u16;
    while done_blocks < count {
        let chunk = (count - done_blocks).min(blocks_per_chunk);
        let chunk_bytes = chunk as usize * geometry.block_size as usize;
        let lba = start + done_blocks as u64;
        let cache_key = (dev_key, lba, chunk);
        let user_off = (done_blocks as usize * geometry.block_size as usize) as u64;
        if write {
            let destination = match buffer.as_mut_slice(chunk_bytes) {
                Ok(d) => d,
                Err(_) => return -(lix_sys::ErrorCode::EFAULT as i32) as u64,
            };
            if copy_from_user_slice(user_ptr.wrapping_add(user_off) as *const u8, destination)
                .is_err()
            {
                return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
            }
            if device.write_blocks(lba, chunk, &buffer).is_err() {
                return -(lix_sys::ErrorCode::EIO as i32) as u64;
            }
            let mut cache = BLOCK_TRANSFER_CACHE.lock();
            cache.map.remove(&cache_key);
        } else {
            let hit = {
                let cache = BLOCK_TRANSFER_CACHE.lock();
                cache.map.get(&cache_key).cloned()
            };
            match hit {
                Some(hit) => {
                    if copy_to_user_slice(
                        user_ptr.wrapping_add(user_off) as *mut u8,
                        hit.as_slice(),
                    )
                    .is_err()
                    {
                        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
                    }
                }
                None => {
                    if device.read_blocks(lba, chunk, &mut buffer).is_err() {
                        return -(lix_sys::ErrorCode::EIO as i32) as u64;
                    }
                    let source = match buffer.as_slice(chunk_bytes) {
                        Ok(source) => source,
                        Err(_) => return -(lix_sys::ErrorCode::EIO as i32) as u64,
                    };
                    if copy_to_user_slice(user_ptr.wrapping_add(user_off) as *mut u8, source)
                        .is_err()
                    {
                        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
                    }
                    let mut cache = BLOCK_TRANSFER_CACHE.lock();
                    cache.map.insert(cache_key, source.to_vec());
                }
            }
        }
        done_blocks += chunk;
    }
    byte_len as u64
}
pub fn sys_block_open(arg1: u64, flags: u64) -> u64 {
    if !installer_privilege() {
        return -(lix_sys::ErrorCode::EPERM as i32) as u64;
    }
    if flags & !lix_sys::block_open_flags::VALID != 0
        || flags & lix_sys::block_open_flags::READ == 0
    {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    let device_index = match u32::try_from(arg1) {
        Ok(index) => index,
        Err(_) => return -(lix_sys::ErrorCode::ENODEV as i32) as u64,
    };
    let Some(entry) = crate::drivers::block::block_device(device_index as usize) else {
        return -(lix_sys::ErrorCode::ENODEV as i32) as u64;
    };
    let writable = flags & lix_sys::block_open_flags::WRITE != 0;
    if writable && crate::fs::ext4::is_device_mounted(entry.device) {
        return -(lix_sys::ErrorCode::EBUSY as i32) as u64;
    }
    let capability =
        match crate::task::handle::BlockDeviceCapability::acquire(device_index, entry, writable) {
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
    if validate_user_buffer(
        out_ptr,
        core::mem::size_of::<lix_sys::BlockDeviceInfo>(),
        true,
    )
    .is_err()
    {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    let geometry = match capability.device.geometry() {
        Ok(geometry) => geometry,
        Err(_) => return -(lix_sys::ErrorCode::ENODEV as i32) as u64,
    };
    let mut info = lix_sys::BlockDeviceInfo {
        block_count: geometry.block_count,
        block_size: geometry.block_size,
        flags: if capability.read_only {
            lix_sys::BlockDeviceInfo::FLAG_READ_ONLY
        } else {
            0
        },
        name: [0u8; lix_sys::BlockDeviceInfo::NAME_LEN],
    };
    copy_device_name(&mut info.name, capability.name);
    if unsafe { copy_to_user(out_ptr as *mut lix_sys::BlockDeviceInfo, &info) }.is_err() {
        return -(lix_sys::ErrorCode::EFAULT as i32) as u64;
    }
    0
}
fn copy_device_name(dest: &mut [u8; lix_sys::BlockDeviceInfo::NAME_LEN], name: &str) {
    let bytes = name.as_bytes();
    let len = bytes.len().min(dest.len() - 1);
    dest[..len].copy_from_slice(&bytes[..len]);
    dest[len] = 0;
}
pub fn sys_block_read(handle_val: u64, start: u64, count: u64, user_ptr: u64) -> u64 {
    let capability = match block_capability(handle_val, lix_sys::Rights::READ) {
        Ok(capability) => capability,
        Err(error) => return -(error as i32) as u64,
    };
    block_transfer(capability.device, start, count, user_ptr, false)
}
pub fn sys_block_write(handle_val: u64, start: u64, count: u64, user_ptr: u64) -> u64 {
    let capability = match block_capability(handle_val, lix_sys::Rights::WRITE) {
        Ok(capability) => capability,
        Err(error) => return -(error as i32) as u64,
    };
    if !capability.writable {
        return -(lix_sys::ErrorCode::EACCES as i32) as u64;
    }
    block_transfer(capability.device, start, count, user_ptr, true)
}
pub fn sys_block_sync(handle_val: u64) -> u64 {
    let capability = match block_capability(handle_val, lix_sys::Rights::WRITE) {
        Ok(capability) => capability,
        Err(error) => return -(error as i32) as u64,
    };
    match capability.device.sync() {
        Ok(()) => 0,
        Err(_) => -(lix_sys::ErrorCode::EIO as i32) as u64,
    }
}
use alloc::collections::BTreeMap;
use crate::sync::IrqSafeSpinlock;
type BlockCacheKey = (usize, u64, u16);
struct BlockTransferCache {
    map: BTreeMap<BlockCacheKey, alloc::vec::Vec<u8>>,
}
impl BlockTransferCache {
    pub const fn new() -> Self {
        Self { map: BTreeMap::new() }
    }
}
static BLOCK_TRANSFER_CACHE: IrqSafeSpinlock<BlockTransferCache> =
    IrqSafeSpinlock::new(BlockTransferCache::new());
