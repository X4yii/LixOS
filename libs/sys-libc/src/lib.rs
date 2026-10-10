#![no_std]

pub mod syscalls {
    use core::arch::asm;
    
    #[inline(always)]
    pub unsafe fn syscall1(num: u64, a1: u64) -> u64 {
        let ret: u64;
        asm!(
            "syscall",
            inout("rax") num => ret,
            in("rdi") a1,
            out("rcx") _,
            out("r11") _,
            options(nostack)
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall2(num: u64, a1: u64, a2: u64) -> u64 {
        let ret: u64;
        asm!(
            "syscall",
            inout("rax") num => ret,
            in("rdi") a1,
            in("rsi") a2,
            out("rcx") _,
            out("r11") _,
            options(nostack)
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall3(num: u64, a1: u64, a2: u64, a3: u64) -> u64 {
        let ret: u64;
        asm!(
            "syscall",
            inout("rax") num => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            out("rcx") _,
            out("r11") _,
            options(nostack)
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall4(num: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
        let ret: u64;
        asm!(
            "syscall",
            inout("rax") num => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            in("r10") a4,
            out("rcx") _,
            out("r11") _,
            options(nostack)
        );
        ret
    }
    
    pub fn sys_write(s: &str) {
        if s.is_empty() {
            return;
        }
        unsafe {
            syscall3(0x302, 1, s.as_ptr() as u64, s.len() as u64);
        }
    }
    
    pub fn sys_read(fd: u64, buf: &mut [u8]) -> usize {
        unsafe {
            syscall3(0x301, fd, buf.as_mut_ptr() as u64, buf.len() as u64) as usize
        }
    }

    /// Reads currently available terminal input and returns immediately when empty.
    pub fn sys_read_nonblocking(fd: u64, buf: &mut [u8]) -> usize {
        unsafe {
            syscall3(0x309, fd, buf.as_mut_ptr() as u64, buf.len() as u64) as usize
        }
    }
    
    pub fn sys_sleep(ms: u64) {
        unsafe {
            syscall1(0x107, ms);
        }
    }
    
    pub fn sys_exit(code: i32) -> ! {
        unsafe {
            syscall1(0x101, code as u64);
        }
        loop {}
    }

    pub fn sys_power(cmd: u64) -> ! {
        unsafe {
            syscall1(0x803, cmd);
        }
        loop {}
    }

    pub fn sys_terminal_set_scale(scale: u8) -> i64 {
        unsafe {
            syscall1(0x704, scale as u64) as i64
        }
    }

    pub fn sys_terminal_zoom(delta: i8) -> i64 {
        let cmd = if delta > 0 { 10 } else if delta < 0 { 11 } else { 0 };
        unsafe {
            syscall1(0x704, cmd as u64) as i64
        }
    }

    pub fn sys_set_keymap(layout_id: u8) -> i64 {
        unsafe {
            syscall1(0x706, layout_id as u64) as i64
        }
    }

    pub fn sys_get_keymap() -> u8 {
        unsafe {
            syscall1(0x707, 0) as u8
        }
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    pub struct ProcessInfo {
        pub pid: u64,
        pub name: [u8; 16],
        pub state: u8,
        pub privilege_level: u8,
        pub cpu_ticks: u64,
        pub ram_bytes: u64,
        pub disk_read_bytes: u64,
    }

    pub fn sys_process_list(buffer: *mut ProcessInfo, max_count: usize) -> usize {
        unsafe {
            syscall2(0x10C, buffer as u64, max_count as u64) as usize
        }
    }

    pub use lix_sys::SystemMetrics;

    pub use lix_sys::BlockDeviceInfo;
    pub use lix_sys::Handle;

    pub fn sys_handle_close(handle: Handle) -> Result<(), i32> {
        let result = unsafe { syscall1(lix_sys::SYS_HANDLE_CLOSE as u64, handle.0) as i64 };
        if result < 0 { Err(result as i32) } else { Ok(()) }
    }

    /// Acquire an exclusive block-device lease. `writable` adds write rights;
    /// the handle is released by `SYS_HANDLE_CLOSE` or process termination.
    pub fn sys_block_open(device_index: u64, writable: bool) -> Result<Handle, i32> {
        let mut flags = lix_sys::block_open_flags::READ;
        if writable {
            flags |= lix_sys::block_open_flags::WRITE;
        }
        let result = unsafe {
            syscall2(lix_sys::SYS_BLOCK_OPEN as u64, device_index, flags) as i64
        };
        if result < 0 { Err(result as i32) } else { Ok(Handle(result as u64)) }
    }

    /// Query block geometry for a kernel-enumerated physical device.
    pub fn sys_block_info(handle: Handle, info: &mut BlockDeviceInfo) -> Result<(), i32> {
        let result = unsafe {
            syscall2(lix_sys::SYS_BLOCK_INFO as u64, handle.0, info as *mut BlockDeviceInfo as u64) as i64
        };
        if result < 0 { Err(result as i32) } else { Ok(()) }
    }

    /// Read complete logical blocks into `out` through a block-device handle.
    pub fn sys_block_read(handle: Handle, start_block: u64, block_count: u16, out: &mut [u8]) -> Result<usize, i32> {
        let result = unsafe {
            syscall4(
                lix_sys::SYS_BLOCK_READ as u64,
                handle.0,
                start_block,
                block_count as u64,
                out.as_mut_ptr() as u64,
            ) as i64
        };
        if result < 0 { Err(result as i32) } else { Ok(result as usize) }
    }

    /// Write complete logical blocks through a writable exclusive handle.
    pub fn sys_block_write(handle: Handle, start_block: u64, block_count: u16, data: &[u8]) -> Result<usize, i32> {
        let result = unsafe {
            syscall4(
                lix_sys::SYS_BLOCK_WRITE as u64,
                handle.0,
                start_block,
                block_count as u64,
                data.as_ptr() as u64,
            ) as i64
        };
        if result < 0 { Err(result as i32) } else { Ok(result as usize) }
    }

    /// Flush in-flight and cached block writes to physical non-volatile storage.
    pub fn sys_block_sync(handle: Handle) -> Result<(), i32> {
        let result = unsafe {
            syscall1(lix_sys::SYS_BLOCK_SYNC as u64, handle.0) as i64
        };
        if result < 0 { Err(result as i32) } else { Ok(()) }
    }

    pub fn sys_system_metrics(metrics: &mut SystemMetrics) -> i32 {
        unsafe {
            syscall1(lix_sys::SYS_MONITOR_METRICS as u64, metrics as *mut SystemMetrics as u64) as i32
        }
    }

    pub fn sys_kill(pid: u64, signal: u8) -> i32 {
        unsafe {
            syscall2(0x102, pid, signal as u64) as i32
        }
    }

    pub use lix_sys::TerminalInfo;

    pub fn sys_terminal_info(info: &mut TerminalInfo) -> i32 {
        unsafe {
            syscall1(lix_sys::SYS_TERMINAL_INFO as u64, info as *mut TerminalInfo as u64) as i32
        }
    }

    pub fn sys_spawn(path: &str) -> Result<u64, i32> {
        let res = unsafe {
            syscall2(lix_sys::SYS_PROCESS_SPAWN as u64, path.as_ptr() as u64, path.len() as u64) as i64
        };
        if res >= 0 {
            Ok(res as u64)
        } else {
            Err(res as i32)
        }
    }

    pub fn sys_waitpid(pid: u64, status: &mut i32) -> Result<u64, i32> {
        let res = unsafe {
            syscall2(lix_sys::SYS_WAITPID as u64, pid, status as *mut i32 as u64) as i64
        };
        if res >= 0 {
            Ok(res as u64)
        } else {
            Err(res as i32)
        }
    }

    pub fn sys_list_dir(path: &str, out: &mut [u8]) -> Result<usize, i32> {
        let res = unsafe {
            syscall3(
                lix_sys::SYS_LIST_DIR as u64,
                path.as_ptr() as u64,
                path.len() as u64,
                out.as_mut_ptr() as u64,
            ) as i64
        };
        if res >= 0 {
            Ok(res as usize)
        } else {
            Err(res as i32)
        }
    }

    pub fn sys_read_file(path: &str, out: &mut [u8]) -> Result<usize, i32> {
        let res = unsafe {
            syscall4(
                lix_sys::SYS_READ_FILE as u64,
                path.as_ptr() as u64,
                path.len() as u64,
                out.as_mut_ptr() as u64,
                out.len() as u64,
            ) as i64
        };
        if res >= 0 {
            Ok(res as usize)
        } else {
            Err(res as i32)
        }
    }
}

pub mod io {
    use core::fmt;
    pub struct Stdout;
    impl fmt::Write for Stdout {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            crate::syscalls::sys_write(s);
            Ok(())
        }
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        let _ = core::fmt::Write::write_fmt(&mut $crate::io::Stdout, core::format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => {
        let _ = core::fmt::Write::write_fmt(&mut $crate::io::Stdout, core::format_args!($($arg)*));
        $crate::print!("\n");
    };
}
