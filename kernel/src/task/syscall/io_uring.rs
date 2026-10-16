use crate::io_uring::IoRing;
use alloc::collections::BTreeMap;
use crate::sync::IrqSafeSpinlock;
struct UringManager {
    rings: BTreeMap<u64, IoRing>,
    next_id: u64,
}
impl UringManager {
    pub const fn new() -> Self {
        Self { rings: BTreeMap::new(), next_id: 1 }
    }
}
static MANAGER: IrqSafeSpinlock<UringManager> = IrqSafeSpinlock::new(UringManager::new());
pub fn sys_uring_setup(entries: u64) -> u64 {
    if entries == 0 || entries > 128 {
        return -(lix_sys::ErrorCode::EINVAL as i32) as u64;
    }
    let mut manager = MANAGER.lock();
    let id = if manager.next_id == u64::MAX {
        return (-(lix_sys::ErrorCode::EINVAL as i32)) as u64;
    } else {
        let id = manager.next_id;
        manager.next_id += 1;
        id
    };
    manager.rings.insert(id, IoRing::new(entries as u32, entries as u32));
    id
}
fn io_uring_read_one_block(lba: u64, user_ptr: u64) -> i32 {
    let Some(entry) = crate::drivers::block::block_device(0) else { return -(lix_sys::ErrorCode::ENODEV as i32); };
    let dev = entry.device;
    let mut buffer = match crate::drivers::block::DmaBuffer::allocate() { Ok(b) => b, Err(_) => return -(lix_sys::ErrorCode::ENOMEM as i32) };
    let geom = dev.geometry().ok();
    let bs = geom.as_ref().map(|g| g.block_size as usize).unwrap_or(512);
    let read = dev.read_blocks(lba, 1, &mut buffer);
    if read.is_err() { return -(lix_sys::ErrorCode::EIO as i32); }
    let source = match buffer.as_slice(bs) { Ok(s) => s, Err(_) => return -(lix_sys::ErrorCode::EIO as i32)};
    if crate::task::syscall::copy_to_user_slice(user_ptr as *mut u8, source).is_err() { return -(lix_sys::ErrorCode::EFAULT as i32); }
    bs as i32
}
fn io_uring_write_one_block(lba: u64, user_ptr: u64) -> i32 {
    let Some(entry) = crate::drivers::block::block_device(0) else { return -(lix_sys::ErrorCode::ENODEV as i32); };
    let dev = entry.device;
    let mut buffer = match crate::drivers::block::DmaBuffer::allocate() { Ok(b) => b, Err(_) => return -(lix_sys::ErrorCode::ENOMEM as i32) };
    let geom = dev.geometry().ok(); let bs = geom.as_ref().map(|g| g.block_size as usize).unwrap_or(512);
    let dest = match buffer.as_mut_slice(bs) { Ok(d) => d, Err(_) => return -(lix_sys::ErrorCode::EFAULT as i32) };
    if crate::task::syscall::copy_from_user_slice(user_ptr as *const u8, dest).is_err() { return -(lix_sys::ErrorCode::EFAULT as i32); }
    match dev.write_blocks(lba, 1, &buffer) { Ok(()) => bs as i32, Err(_) => -(lix_sys::ErrorCode::EIO as i32) }
}
pub fn sys_uring_submit(
    handle: u64,
    op: u8,
    arg1: u64,
    arg2: u64,
) -> u64 {
    let mut manager = MANAGER.lock();
    let Some(ring) = manager.rings.get_mut(&handle) else {
        return -(lix_sys::ErrorCode::EBADF as i32) as u64;
    };
    let sqe = crate::io_uring::IoUringSqe { op, fd: 0, len: 0, off: arg1, user_data: arg2 };
    if !ring.submit(sqe) {
        return -(lix_sys::ErrorCode::EAGAIN as i32) as u64;
    }
    let result = match op {
        1 => io_uring_read_one_block(arg1, arg2),
        2 => io_uring_write_one_block(arg1, arg2),
        _ => 0,
    };
    ring.complete(arg2, result);
    0
}
pub fn sys_uring_enter(handle: u64) -> u64 {
    let mut manager = MANAGER.lock();
    let Some(ring) = manager.rings.get_mut(&handle) else {
        return -(lix_sys::ErrorCode::EBADF as i32) as u64;
    };
    for _ in 0..1000 { core::hint::spin_loop(); }
    ring.pop_completion().map(|_| 1).unwrap_or(0)
}
pub fn sys_uring_peek(handle: u64, user_ptr: u64) -> u64 {
    let mut manager = MANAGER.lock();
    let Some(ring) = manager.rings.get_mut(&handle) else {
        return -(lix_sys::ErrorCode::EBADF as i32) as u64;
    };
    let Some(cqe) = ring.pop_completion() else { return 0; };
    let bytes: [u8; 16] = unsafe { core::mem::transmute(cqe) };
    match crate::task::syscall::copy_to_user_slice(user_ptr as *mut u8, &bytes) {
        Ok(_) => 1,
        Err(_) => (-(lix_sys::ErrorCode::EFAULT as i32)) as u64,
    }
}
