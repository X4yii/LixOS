#![no_std]
extern crate alloc;
pub mod allocator;
pub use lix_sys::{Handle, Rights};
#[allow(unsafe_code)]
pub mod syscalls {
    use alloc::vec::Vec;
    use core::arch::asm;
    #[inline(always)]
    pub unsafe fn syscall0(num: u64) -> u64 {
        let ret: u64;
        asm!("syscall", inout("rax") num => ret, out("rcx") _, out("r11") _, options(nostack));
        ret
    }
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
    #[inline(always)]
    pub unsafe fn syscall5(num: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
        let ret: u64;
        asm!(
            "syscall",
            inout("rax") num => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            in("r10") a4,
            in("r8") a5,
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
            syscall3(
                lix_sys::SYS_WRITE as u64,
                1,
                s.as_ptr() as u64,
                s.len() as u64,
            );
        }
    }
    pub fn sys_read(fd: u64, buf: &mut [u8]) -> Result<usize, i32> {
        let result = unsafe {
            syscall3(
                lix_sys::SYS_READ as u64,
                fd,
                buf.as_mut_ptr() as u64,
                buf.len() as u64,
            )
        } as i64;
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(result as usize)
        }
    }
    pub fn sys_read_nonblocking(fd: u64, buf: &mut [u8]) -> Result<usize, i32> {
        let result = unsafe {
            syscall3(
                lix_sys::SYS_READ_NONBLOCKING as u64,
                fd,
                buf.as_mut_ptr() as u64,
                buf.len() as u64,
            )
        } as i64;
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(result as usize)
        }
    }
    pub const INPUT_CLASS_ALL: u32 = 0;
    pub const INPUT_CLASS_KEYBOARD: u32 = 1;
    pub const INPUT_CLASS_POINTER: u32 = 2;
    pub fn sys_input_open(class: u32) -> Result<lix_sys::Handle, i32> {
        let result = unsafe { syscall1(lix_sys::SYS_INPUT_OPEN as u64, u64::from(class)) };
        if (result as i64) < 0 {
            Err(result as i64 as i32)
        } else {
            Ok(lix_sys::Handle(result))
        }
    }
    pub fn sys_read_input_events(
        handle: lix_sys::Handle,
        events: &mut [lix_sys::InputEvent],
    ) -> Result<usize, i32> {
        let bytes = unsafe {
            core::slice::from_raw_parts_mut(
                events.as_mut_ptr().cast::<u8>(),
                core::mem::size_of_val(events),
            )
        };
        let result = unsafe {
            syscall3(
                lix_sys::SYS_READ as u64,
                handle.0,
                bytes.as_mut_ptr() as u64,
                bytes.len() as u64,
            )
        };
        if (result as i64) < 0 {
            Err(result as i64 as i32)
        } else if !(result as usize).is_multiple_of(core::mem::size_of::<lix_sys::InputEvent>()) {
            Err(-(lix_sys::ErrorCode::EIO as i32))
        } else {
            Ok(result as usize / core::mem::size_of::<lix_sys::InputEvent>())
        }
    }
    pub fn sys_usb_enumerate(devices: &mut [lix_sys::UsbDeviceInfo]) -> Result<usize, i32> {
        let result = unsafe {
            syscall2(
                lix_sys::SYS_USB_ENUMERATE as u64,
                devices.as_mut_ptr() as u64,
                devices.len() as u64,
            )
        };
        if (result as i64) < 0 {
            Err(result as i64 as i32)
        } else {
            Ok(result as usize)
        }
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct SocketAddrV4 {
        pub family: u16,
        pub port_be: u16,
        pub address: [u8; 4],
        pub reserved: [u8; 8],
    }
    pub fn socket_udp() -> Result<lix_sys::Handle, i32> {
        let value = unsafe { syscall0(lix_sys::SYS_SOCKET_OPEN as u64) };
        if (value as i64) < 0 {
            Err(value as i64 as i32)
        } else {
            Ok(lix_sys::Handle(value))
        }
    }
    pub fn socket_bind(socket: lix_sys::Handle, port: u16) -> Result<(), i32> {
        let value = unsafe { syscall2(lix_sys::SYS_SOCKET_BIND as u64, socket.0, u64::from(port)) };
        if (value as i64) < 0 {
            Err(value as i64 as i32)
        } else {
            Ok(())
        }
    }
    pub fn socket_sendto(
        socket: lix_sys::Handle,
        addr: &SocketAddrV4,
        data: &[u8],
    ) -> Result<usize, i32> {
        let value = unsafe {
            syscall4(
                lix_sys::SYS_SOCKET_SENDTO as u64,
                socket.0,
                core::ptr::from_ref(addr) as u64,
                data.as_ptr() as u64,
                data.len() as u64,
            )
        };
        if (value as i64) < 0 {
            Err(value as i64 as i32)
        } else {
            Ok(value as usize)
        }
    }
    pub fn socket_recvfrom(
        socket: lix_sys::Handle,
        data: &mut [u8],
        addr: &mut SocketAddrV4,
    ) -> Result<usize, i32> {
        let value = unsafe {
            syscall4(
                lix_sys::SYS_SOCKET_RECVFROM as u64,
                socket.0,
                data.as_mut_ptr() as u64,
                data.len() as u64,
                core::ptr::from_mut(addr) as u64,
            )
        };
        if (value as i64) < 0 {
            Err(value as i64 as i32)
        } else {
            Ok(value as usize)
        }
    }
    pub fn sys_sleep(ms: u64) {
        unsafe {
            syscall1(lix_sys::SYS_SLEEP as u64, ms);
        }
    }
    pub fn sys_exit(code: i32) -> ! {
        unsafe {
            syscall1(lix_sys::SYS_PROCESS_EXIT as u64, code as u64);
        }
        loop {}
    }
    pub fn sys_power(cmd: u64) -> ! {
        unsafe {
            syscall1(lix_sys::SYS_POWER as u64, cmd);
        }
        loop {}
    }
    pub fn sys_terminal_set_scale(scale: u8) -> i64 {
        unsafe { syscall1(lix_sys::SYS_TERMINAL_SET_SCALE as u64, u64::from(scale)) as i64 }
    }
    pub fn sys_terminal_zoom(delta: i8) -> i64 {
        let cmd = if delta > 0 {
            10
        } else if delta < 0 {
            11
        } else {
            0
        };
        unsafe { syscall1(lix_sys::SYS_TERMINAL_SET_SCALE as u64, cmd as u64) as i64 }
    }
    pub fn sys_set_keymap(layout_id: u8) -> i64 {
        unsafe { syscall1(lix_sys::SYS_SET_KEYMAP as u64, u64::from(layout_id)) as i64 }
    }
    pub fn sys_get_keymap() -> u8 {
        unsafe { syscall1(lix_sys::SYS_GET_KEYMAP as u64, 0) as u8 }
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
            syscall2(
                lix_sys::SYS_PROCESS_LIST as u64,
                buffer as u64,
                max_count as u64,
            ) as usize
        }
    }
    pub const PROT_NONE: u64 = 0x0;
    pub const PROT_READ: u64 = 0x1;
    pub const PROT_WRITE: u64 = 0x2;
    pub const PROT_EXEC: u64 = 0x4;
    pub const MAP_SHARED: u64 = 0x01;
    pub const MAP_PRIVATE: u64 = 0x02;
    pub const MAP_FIXED: u64 = 0x10;
    pub const MAP_ANONYMOUS: u64 = 0x20;
    pub fn sys_mmap(addr: u64, len: u64, prot: u64, flags: u64) -> Result<u64, i32> {
        let res = unsafe {
            syscall4(
                lix_sys::SYS_MMAP as u64,
                addr,
                len,
                prot,
                flags,
            ) as i64
        };
        if res >= 0 {
            Ok(res as u64)
        } else {
            Err(res as i32)
        }
    }
    pub fn sys_munmap(addr: u64, len: u64) -> Result<(), i32> {
        let res = unsafe {
            syscall2(
                lix_sys::SYS_MUNMAP as u64,
                addr,
                len,
            ) as i64
        };
        if res >= 0 {
            Ok(())
        } else {
            Err(res as i32)
        }
    }
    pub fn sys_open(path: &str, flags: u64) -> Result<Handle, i32> {
        let res = unsafe {
            syscall3(
                lix_sys::SYS_OPEN as u64,
                path.as_ptr() as u64,
                path.len() as u64,
                flags,
            ) as i64
        };
        if res >= 0 {
            Ok(Handle(res as u64))
        } else {
            Err(res as i32)
        }
    }
    pub fn sys_lseek(handle: Handle, offset: i64, whence: u64) -> Result<u64, i32> {
        let res = unsafe {
            syscall3(
                lix_sys::SYS_LSEEK as u64,
                handle.0,
                offset as u64,
                whence,
            ) as i64
        };
        if res >= 0 {
            Ok(res as u64)
        } else {
            Err(res as i32)
        }
    }
    pub fn sys_channel_create() -> Result<(Handle, Handle), i32> {
        let mut handles = [0u64; 2];
        let res = unsafe {
            syscall1(
                lix_sys::SYS_CHANNEL_CREATE as u64,
                handles.as_mut_ptr() as u64,
            ) as i64
        };
        if res >= 0 {
            Ok((Handle(handles[0]), Handle(handles[1])))
        } else {
            Err(res as i32)
        }
    }
    pub fn sys_channel_send(
        channel: Handle,
        data: &[u8],
        handles: &[Handle],
    ) -> Result<(), i32> {
        let raw_handles: alloc::vec::Vec<u64> = handles.iter().map(|h| h.0).collect();
        let res = unsafe {
            syscall5(
                lix_sys::SYS_CHANNEL_SEND as u64,
                channel.0,
                data.as_ptr() as u64,
                data.len() as u64,
                raw_handles.as_ptr() as u64,
                raw_handles.len() as u64,
            ) as i64
        };
        if res >= 0 {
            Ok(())
        } else {
            Err(res as i32)
        }
    }
    pub fn sys_channel_recv(
        channel: Handle,
        data: &mut [u8],
        handles: &mut alloc::vec::Vec<Handle>,
    ) -> Result<usize, i32> {
        let mut raw_handles = [0u64; 16];
        let res = unsafe {
            syscall5(
                lix_sys::SYS_CHANNEL_RECV as u64,
                channel.0,
                data.as_mut_ptr() as u64,
                data.len() as u64,
                raw_handles.as_mut_ptr() as u64,
                raw_handles.len() as u64,
            ) as i64
        };
        if res >= 0 {
            for &rh in &raw_handles {
                if rh != 0 {
                    handles.push(Handle(rh));
                }
            }
            Ok(res as usize)
        } else {
            Err(res as i32)
        }
    }
    pub use lix_sys::BlockDeviceInfo;
    pub use lix_sys::Handle;
    pub use lix_sys::SystemMetrics;
    pub fn sys_handle_close(handle: Handle) -> Result<(), i32> {
        let result = unsafe { syscall1(lix_sys::SYS_HANDLE_CLOSE as u64, handle.0) as i64 };
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(())
        }
    }
    pub fn sys_handle_dup(handle: Handle, rights: u32) -> Result<Handle, i32> {
        let result =
            unsafe { syscall2(lix_sys::SYS_HANDLE_DUP as u64, handle.0, rights as u64) as i64 };
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(Handle(result as u64))
        }
    }
    pub fn sys_handle_dup2(src: Handle, dst_idx: u64) -> Result<Handle, i32> {
        let result = unsafe { syscall2(lix_sys::SYS_HANDLE_DUP2 as u64, src.0, dst_idx) as i64 };
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(Handle(result as u64))
        }
    }
    pub fn sys_block_open(device_index: u64, writable: bool) -> Result<Handle, i32> {
        let mut flags = lix_sys::block_open_flags::READ;
        if writable {
            flags |= lix_sys::block_open_flags::WRITE;
        }
        let result =
            unsafe { syscall2(lix_sys::SYS_BLOCK_OPEN as u64, device_index, flags) as i64 };
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(Handle(result as u64))
        }
    }
    pub fn sys_block_info(handle: Handle, info: &mut BlockDeviceInfo) -> Result<(), i32> {
        let result = unsafe {
            syscall2(
                lix_sys::SYS_BLOCK_INFO as u64,
                handle.0,
                core::ptr::from_mut::<BlockDeviceInfo>(info) as u64,
            ) as i64
        };
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(())
        }
    }
    pub fn sys_block_read(
        handle: Handle,
        start_block: u64,
        block_count: u16,
        out: &mut [u8],
    ) -> Result<usize, i32> {
        let result = unsafe {
            syscall4(
                lix_sys::SYS_BLOCK_READ as u64,
                handle.0,
                start_block,
                u64::from(block_count),
                out.as_mut_ptr() as u64,
            ) as i64
        };
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(result as usize)
        }
    }
    pub fn sys_block_write(
        handle: Handle,
        start_block: u64,
        block_count: u16,
        data: &[u8],
    ) -> Result<usize, i32> {
        let result = unsafe {
            syscall4(
                lix_sys::SYS_BLOCK_WRITE as u64,
                handle.0,
                start_block,
                u64::from(block_count),
                data.as_ptr() as u64,
            ) as i64
        };
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(result as usize)
        }
    }
    pub fn sys_block_sync(handle: Handle) -> Result<(), i32> {
        let result = unsafe { syscall1(lix_sys::SYS_BLOCK_SYNC as u64, handle.0) as i64 };
        if result < 0 {
            Err(result as i32)
        } else {
            Ok(())
        }
    }
    pub fn sys_system_metrics(metrics: &mut SystemMetrics) -> i32 {
        unsafe {
            syscall1(
                lix_sys::SYS_MONITOR_METRICS as u64,
                core::ptr::from_mut::<SystemMetrics>(metrics) as u64,
            ) as i32
        }
    }
    pub fn sys_kill(pid: u64, signal: u8) -> i32 {
        unsafe { syscall2(lix_sys::SYS_PROCESS_KILL as u64, pid, u64::from(signal)) as i32 }
    }
    pub use lix_sys::TerminalInfo;
    pub fn sys_terminal_info(info: &mut TerminalInfo) -> i32 {
        unsafe {
            syscall1(
                lix_sys::SYS_TERMINAL_INFO as u64,
                core::ptr::from_mut::<TerminalInfo>(info) as u64,
            ) as i32
        }
    }
    pub fn sys_spawn(path: &str) -> Result<u64, i32> {
        let res = unsafe {
            syscall2(
                lix_sys::SYS_PROCESS_SPAWN as u64,
                path.as_ptr() as u64,
                path.len() as u64,
            ) as i64
        };
        if res >= 0 {
            Ok(res as u64)
        } else {
            Err(res as i32)
        }
    }
    pub fn sys_waitpid(pid: u64, status: &mut i32) -> Result<u64, i32> {
        let res = unsafe {
            syscall2(
                lix_sys::SYS_WAITPID as u64,
                pid,
                core::ptr::from_mut::<i32>(status) as u64,
            ) as i64
        };
        if res >= 0 {
            Ok(res as u64)
        } else {
            Err(res as i32)
        }
    }
    pub fn sys_list_dir(path: &str, out: &mut [u8]) -> Result<usize, i32> {
        let res = unsafe {
            syscall4(
                lix_sys::SYS_LIST_DIR as u64,
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
    pub fn sys_read_file_all(path: &str) -> Result<Vec<u8>, i32> {
        const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
        let mut capacity = 8192usize;
        loop {
            let mut buf: Vec<u8> = Vec::new();
            if buf.try_reserve_exact(capacity).is_err() {
                return Err(-(lix_sys::ErrorCode::ENOMEM as i32));
            }
            buf.resize(capacity, 0);
            let read = sys_read_file(path, &mut buf)?;
            if read < capacity || capacity >= MAX_FILE_BYTES {
                buf.truncate(read);
                return Ok(buf);
            }
            capacity *= 2;
        }
    }
    pub fn sys_write_file(path: &str, data: &[u8]) -> Result<usize, i32> {
        let res = unsafe {
            syscall4(
                lix_sys::SYS_WRITE_FILE as u64,
                path.as_ptr() as u64,
                path.len() as u64,
                data.as_ptr() as u64,
                data.len() as u64,
            ) as i64
        };
        if res >= 0 {
            Ok(res as usize)
        } else {
            Err(res as i32)
        }
    }
    pub fn sys_remove_file(path: &str, recursive: bool) -> Result<(), i32> {
        let res = unsafe {
            syscall3(
                lix_sys::SYS_REMOVE_FILE as u64,
                path.as_ptr() as u64,
                path.len() as u64,
                u64::from(recursive),
            ) as i64
        };
        if res >= 0 {
            Ok(())
        } else {
            Err(res as i32)
        }
    }
    pub fn sys_mkdir(path: &str) -> Result<(), i32> {
        let res = unsafe {
            syscall2(
                lix_sys::SYS_MKDIR as u64,
                path.as_ptr() as u64,
                path.len() as u64,
            ) as i64
        };
        if res >= 0 {
            Ok(())
        } else {
            Err(res as i32)
        }
    }
}
pub const fn strerror(code: i32) -> &'static str {
    let code = code.unsigned_abs();
    match code {
        0 => "operación exitosa",
        1 => "permiso denegado",
        2 => "archivo o directorio no encontrado",
        3 => "proceso no encontrado",
        4 => "interrupción del sistema",
        5 => "error de entrada/salida",
        8 => "ejecutable no válido",
        9 => "descriptor de archivo inválido",
        11 => "recurso no disponible",
        12 => "sin memoria",
        13 => "acceso denegado",
        14 => "dirección de memoria inválida",
        16 => "recurso ocupado",
        17 => "el archivo ya existe",
        19 => "dispositivo no encontrado",
        22 => "argumento inválido",
        24 => "demasiados archivos abiertos",
        38 => "función no implementada",
        98 => "dirección ya en uso",
        110 => "tiempo de espera agotado",
        1001 => "autorización revocada",
        1002 => "límite de licencia alcanzado",
        1003 => "el par cerró la conexión",
        _ => "error desconocido",
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
