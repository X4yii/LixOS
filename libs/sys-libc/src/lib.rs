#![no_std]

pub mod syscalls {
    use core::arch::asm;
    
    pub fn sys_write(s: &str) {
        unsafe {
            asm!(
                "int 0x80",
                in("rax") 0x302,
                in("rsi") s.as_ptr(),
                in("rdx") s.len(),
            );
        }
    }
    
    pub fn sys_read(fd: u64, buf: &mut [u8]) -> usize {
        let mut read: usize;
        unsafe {
            asm!(
                "int 0x80",
                in("rax") 0x301,
                in("rdi") fd,
                in("rsi") buf.as_mut_ptr(),
                in("rdx") buf.len(),
                lateout("rax") read,
            );
        }
        read
    }
    
    pub fn sys_sleep(ms: u64) {
        unsafe {
            asm!(
                "int 0x80",
                in("rax") 0x107,
                in("rdi") ms,
            );
        }
    }
    
    pub fn sys_exit(code: i32) -> ! {
        unsafe {
            asm!(
                "int 0x80",
                in("rax") 0x101,
                in("rdi") code as u64,
            );
        }
        loop {}
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    pub struct ProcessInfo {
        pub pid: u64,
        pub name: [u8; 16],
        pub state: u8,
        pub privilege_level: u8,
        pub cpu_ticks: u64,
        pub ram_bytes: u64,
    }

    pub fn sys_process_list(buffer: *mut ProcessInfo, max_count: usize) -> usize {
        let mut count: usize;
        unsafe {
            asm!(
                "int 0x80",
                in("rax") 0x10C,
                in("rdi") buffer as u64,
                in("rsi") max_count,
                lateout("rax") count,
            );
        }
        count
    }

    pub fn sys_kill(pid: u64, signal: u8) -> i32 {
        let mut result: i32;
        unsafe {
            asm!(
                "int 0x80",
                in("rax") 0x102,
                in("rdi") pid,
                in("rsi") signal as u64,
                lateout("rax") result,
            );
        }
        result
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