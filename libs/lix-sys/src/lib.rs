#![no_std]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Handle(pub u64);
impl Handle {
    pub const INVALID: Self = Self(0);
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
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXEC: Self = Self(1 << 2);
    pub const DUP: Self = Self(1 << 3);
    pub const TRANSFER: Self = Self(1 << 4);
    pub const WAIT: Self = Self(1 << 5);
    pub const SIGNAL: Self = Self(1 << 6);
    pub const MAP: Self = Self(1 << 7);
    pub const ENUMERATE: Self = Self(1 << 8);
    pub const TRAVERSE: Self = Self(1 << 9);
    pub const MANAGE: Self = Self(1 << 10);
    pub const INSPECT: Self = Self(1 << 11);
    pub const SPAWN: Self = Self(1 << 12);
    pub const DISPLAY_OWN: Self = Self(1 << 13);
    pub const INPUT_READ: Self = Self(1 << 14);
    pub const POWER: Self = Self(1 << 15);
    pub const DEVICES: Self = Self(1 << 16);
    pub const AUTH: Self = Self(1 << 17);
    pub const LOG: Self = Self(1 << 18);
    pub const ELEVATE: Self = Self(1 << 19);
    pub const NET_READ: Self = Self(1 << 20);
    pub const NET_ADMIN: Self = Self(1 << 21);
    pub const BLUETOOTH_READ: Self = Self(1 << 22);
    pub const BLUETOOTH_ADMIN: Self = Self(1 << 23);
    pub const SECRET_USE: Self = Self(1 << 24);
    pub const NET_CONNECT: Self = Self(1 << 25);
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    #[inline]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
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
    EADDRINUSE = 98,
    ENODEV = 19,
    EINVAL = 22,
    EMFILE = 24,
    ENOSYS = 38,
    ETIMEDOUT = 110,
    ELIX_REVOKED = 1001,
    ELIX_LIMIT = 1002,
    ELIX_PEER_CLOSED = 1003,
}
include!("syscalls.rs");
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputEvent {
    pub time_ns: u64,
    pub device: u16,
    pub kind: u16,
    pub code: u32,
    pub value: i32,
}
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsbDeviceInfo {
    pub id: u16,
    pub root_port: u8,
    pub slot: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub reserved: u8,
    pub state: u32,
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
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockDeviceInfo {
    pub block_count: u64,
    pub block_size: u32,
    pub flags: u32,
    /// Nombre corto del dispositivo (p. ej. `nvme0`), terminado en NUL.
    pub name: [u8; 16],
}
impl BlockDeviceInfo {
    pub const FLAG_READ_ONLY: u32 = 1;
    /// Longitud del buffer `name`, incluido el byte NUL final.
    pub const NAME_LEN: usize = 16;
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
        assert_eq!(core::mem::size_of::<BlockDeviceInfo>(), 32);
        assert_eq!(core::mem::align_of::<BlockDeviceInfo>(), 8);
        assert_eq!(core::mem::offset_of!(BlockDeviceInfo, block_size), 8);
        assert_eq!(core::mem::offset_of!(BlockDeviceInfo, flags), 12);
        assert_eq!(core::mem::offset_of!(BlockDeviceInfo, name), 16);
    }
    #[test]
    fn block_syscalls_do_not_overlap_existing_ranges() {
        assert_eq!(SYS_BLOCK_OPEN, 0x820);
        assert_eq!(SYS_BLOCK_INFO, 0x821);
        assert_eq!(SYS_BLOCK_READ, 0x822);
        assert_eq!(SYS_BLOCK_WRITE, 0x823);
        assert_eq!(SYS_BLOCK_SYNC, 0x824);
    }
    #[test]
    fn syscall_numbers_are_unique() {
        let numbers = [
            SYS_ABI_VERSION,
            SYS_HANDLE_CLOSE,
            SYS_HANDLE_DUP,
            SYS_PROCESS_SPAWN,
            SYS_PROCESS_EXIT,
            SYS_PROCESS_KILL,
            SYS_SLEEP,
            SYS_WAITPID,
            SYS_PROCESS_LIST,
            SYS_READ,
            SYS_WRITE,
            SYS_LIST_DIR,
            SYS_READ_FILE,
            SYS_WRITE_FILE,
            SYS_READ_NONBLOCKING,
            SYS_REMOVE_FILE,
            SYS_MKDIR,
            SYS_TERMINAL_INFO,
            SYS_TERMINAL_SET_SCALE,
            SYS_INPUT_OPEN,
            SYS_SET_KEYMAP,
            SYS_GET_KEYMAP,
            SYS_LOG_WRITE,
            SYS_POWER,
            SYS_MONITOR_METRICS,
            SYS_BLOCK_OPEN,
            SYS_BLOCK_INFO,
            SYS_BLOCK_READ,
            SYS_BLOCK_WRITE,
            SYS_BLOCK_SYNC,
            SYS_USB_ENUMERATE,
            SYS_SOCKET_OPEN,
            SYS_SOCKET_BIND,
            SYS_SOCKET_SENDTO,
            SYS_SOCKET_RECVFROM,
        ];
        for i in 0..numbers.len() {
            for j in (i + 1)..numbers.len() {
                assert_ne!(numbers[i], numbers[j]);
            }
        }
    }
}
