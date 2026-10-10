#![no_std]

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Handle(pub u64);

impl Handle {
    pub const INVALID: Handle = Handle(0);

    #[inline]
    pub const fn new(index: u32, generation: u32) -> Self {
        Self(((generation as u64) << 32) | (index as u64))
    }

    #[inline]
    pub const fn index(self) -> u32 {
        self.0 as u32
    }

    #[inline]
    pub const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    #[inline]
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Rights(pub u32);

impl Rights {
    pub const NONE: Rights = Rights(0);
    pub const READ: Rights = Rights(1 << 0);
    pub const WRITE: Rights = Rights(1 << 1);
    pub const EXEC: Rights = Rights(1 << 2);
    pub const DUP: Rights = Rights(1 << 3);
    pub const TRANSFER: Rights = Rights(1 << 4);
    pub const WAIT: Rights = Rights(1 << 5);
    pub const SIGNAL: Rights = Rights(1 << 6);
    pub const MAP: Rights = Rights(1 << 7);
    pub const ENUMERATE: Rights = Rights(1 << 8);
    pub const TRAVERSE: Rights = Rights(1 << 9);
    pub const MANAGE: Rights = Rights(1 << 10);
    pub const INSPECT: Rights = Rights(1 << 11);
    pub const SPAWN: Rights = Rights(1 << 12);
    pub const DISPLAY_OWN: Rights = Rights(1 << 13);
    pub const INPUT_READ: Rights = Rights(1 << 14);
    pub const POWER: Rights = Rights(1 << 15);
    pub const DEVICES: Rights = Rights(1 << 16);
    pub const AUTH: Rights = Rights(1 << 17);
    pub const LOG: Rights = Rights(1 << 18);
    pub const ELEVATE: Rights = Rights(1 << 19);

    #[inline]
    pub const fn contains(self, other: Rights) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn union(self, other: Rights) -> Rights {
        Rights(self.0 | other.0)
    }

    #[inline]
    pub const fn intersection(self, other: Rights) -> Rights {
        Rights(self.0 & other.0)
    }
}

#[allow(non_camel_case_types)]
#[repr(i32)]
pub enum ErrorCode {
    Success = 0,
    EPERM = 1,
    ENOENT = 2,
    ESRCH = 3,
    EINTR = 4,
    EIO = 5,
    ENOEXEC = 8,
    EBADF = 9,
    EAGAIN = 11,
    ENOMEM = 12,
    EACCES = 13,
    EFAULT = 14,
    EBUSY = 16,
    EEXIST = 17,
    ENODEV = 19,
    EINVAL = 22,
    EMFILE = 24,
    ENOSYS = 38,
    ETIMEDOUT = 110,
    ELIX_REVOKED = 1001,
    ELIX_LIMIT = 1002,
    ELIX_PEER_CLOSED = 1003,
}

    pub const SYS_ABI_VERSION: usize = 0x000;
    pub const SYS_HANDLE_CLOSE: usize = 0x001;
    pub const SYS_HANDLE_DUP: usize = 0x002;
    pub const SYS_HANDLE_REPLACE: usize = 0x003;
    pub const SYS_HANDLE_INFO: usize = 0x004;
    pub const SYS_WAIT_ONE: usize = 0x005;
    pub const SYS_WAIT_MANY: usize = 0x006;
    pub const SYS_REVOKER_CREATE: usize = 0x007;
    pub const SYS_REVOKER_REVOKE: usize = 0x008;
    pub const SYS_HANDLE_DUP_REVOCABLE: usize = 0x009;

    pub const SYS_PROCESS_SPAWN: usize = 0x100;
    pub const SYS_PROCESS_EXIT: usize = 0x101;
    pub const SYS_PROCESS_KILL: usize = 0x102;
    pub const SYS_PROCESS_INFO: usize = 0x103;
    pub const SYS_THREAD_CREATE: usize = 0x104;
    pub const SYS_THREAD_EXIT: usize = 0x105;
    pub const SYS_THREAD_YIELD: usize = 0x106;
    pub const SYS_JOB_CREATE: usize = 0x108;
    pub const SYS_JOB_KILL: usize = 0x109;
    pub const SYS_WAITPID: usize = 0x10A;

    pub const SYS_VMO_CREATE: usize = 0x200;
    pub const SYS_VMO_RESIZE: usize = 0x201;
    pub const SYS_VM_MAP: usize = 0x202;
    pub const SYS_VM_UNMAP: usize = 0x203;
    pub const SYS_VM_PROTECT: usize = 0x204;
    pub const SYS_VM_INFO: usize = 0x205;

    pub const SYS_OPEN_AT: usize = 0x300;
    pub const SYS_READ: usize = 0x301;
    pub const SYS_READ_NONBLOCK: usize = 0x309;
    pub const SYS_WRITE: usize = 0x302;
    pub const SYS_SEEK: usize = 0x305;
    pub const SYS_STAT: usize = 0x306;
    pub const SYS_LIST_DIR: usize = 0x306;
    pub const SYS_READ_FILE: usize = 0x307;
    pub const SYS_NAMESPACE_CREATE: usize = 0x313;
    pub const SYS_NAMESPACE_BIND: usize = 0x314;

    pub const SYS_DISPLAY_INFO: usize = 0x700;
    pub const SYS_DISPLAY_ACQUIRE: usize = 0x701;
    pub const SYS_DISPLAY_PRESENT: usize = 0x702;
    pub const SYS_TERMINAL_INFO: usize = 0x703;
    pub const SYS_TERMINAL_SET_SCALE: usize = 0x704;
    pub const SYS_INPUT_OPEN: usize = 0x705;
    pub const SYS_SET_KEYMAP: usize = 0x706;
    pub const SYS_GET_KEYMAP: usize = 0x707;

    pub const SYS_SYSTEM_INFO: usize = 0x800;
    pub const SYS_LOG_WRITE: usize = 0x801;
    pub const SYS_POWER: usize = 0x803;
    pub const SYS_MONITOR_METRICS: usize = 0x80A;
    pub const SYS_BLOCK_OPEN: usize = 0x820;
    pub const SYS_BLOCK_INFO: usize = 0x821;
    pub const SYS_BLOCK_READ: usize = 0x822;
    pub const SYS_BLOCK_WRITE: usize = 0x823;
    pub const SYS_BLOCK_SYNC: usize = 0x824;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TerminalInfo {
    pub cols: u16,
    pub rows: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
    pub font_width: u8,
    pub font_height: u8,
    pub scale: u8,
    pub _reserved: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemMetrics {
    pub scheduler_ticks: u64,
    pub total_ram_bytes: u64,
    pub free_ram_bytes: u64,
    pub disk_read_ops: u64,
    pub disk_read_bytes: u64,
    pub disk_mounted: u64,
}

/// Stable userspace view of the primary physical block device.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockDeviceInfo {
    pub block_count: u64,
    pub block_size: u32,
    /// This initial ABI is deliberately read-only.
    pub flags: u32,
}

impl BlockDeviceInfo {
    pub const FLAG_READ_ONLY: u32 = 1;
}

pub mod block_open_flags {
    pub const READ: u64 = 1 << 0;
    pub const WRITE: u64 = 1 << 1;
    pub const VALID: u64 = READ | WRITE;
}

#[cfg(test)]
mod abi_tests {
    use super::*;

    #[test]
    fn block_info_layout_is_stable() {
        assert_eq!(core::mem::size_of::<BlockDeviceInfo>(), 16);
        assert_eq!(core::mem::align_of::<BlockDeviceInfo>(), 8);
        assert_eq!(core::mem::offset_of!(BlockDeviceInfo, block_size), 8);
        assert_eq!(core::mem::offset_of!(BlockDeviceInfo, flags), 12);
    }

    #[test]
    fn block_syscalls_do_not_overlap_existing_ranges() {
        assert_eq!(SYS_BLOCK_OPEN, 0x820);
        assert_eq!(SYS_BLOCK_INFO, 0x821);
        assert_eq!(SYS_BLOCK_READ, 0x822);
        assert_eq!(SYS_BLOCK_WRITE, 0x823);
        assert_eq!(SYS_BLOCK_SYNC, 0x824);
    }
}
