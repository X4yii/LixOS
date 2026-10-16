pub mod block;
pub mod channel;
pub mod io_uring;
pub mod io;
pub mod mem;
pub mod net;
pub mod proc;
pub mod term;
pub const MAX_PATH_BYTES: usize = 256;
pub const MAX_USER_IO_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PROCESS_INFO_ENTRIES: usize = 256;
pub fn set_cpu_kernel_rsp(cpu_id: usize, rsp: u64) {
    let percpu = crate::arch::x86_64::percpu::percpu_mut(cpu_id);
    percpu.kernel_rsp = rsp;
}
const IA32_EFER: u32 = 0xC0000080;
const IA32_STAR: u32 = 0xC0000081;
const IA32_LSTAR: u32 = 0xC0000082;
const IA32_FMASK: u32 = 0xC0000084;
const IA32_GS_BASE: u32 = 0xC0000101;
const IA32_KERNEL_GS_BASE: u32 = 0xC0000102;
unsafe fn wrmsr(msr: u32, val: u64) {
    let low = val as u32;
    let high = (val >> 32) as u32;
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") low,
        in("edx") high,
        options(nostack, preserves_flags)
    );
}
unsafe fn rdmsr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") low,
        out("edx") high,
        options(nostack, preserves_flags)
    );
    ((high as u64) << 32) | (low as u64)
}
pub unsafe fn init_syscall_msrs_for_cpu(cpu_id: usize) {
    let efer = rdmsr(IA32_EFER);
    wrmsr(IA32_EFER, efer | 1);
    let star = ((0x0010u64 | 3) << 48) | (0x0008u64 << 32);
    wrmsr(IA32_STAR, star);
    let lstar = syscall_entry_asm as *const () as u64;
    wrmsr(IA32_LSTAR, lstar);
    wrmsr(IA32_FMASK, 0x00257FD);
    let ptr = crate::arch::x86_64::percpu::percpu_ptr(cpu_id) as u64;
    wrmsr(IA32_GS_BASE, ptr);
    wrmsr(IA32_KERNEL_GS_BASE, 0);
    if cpu_id == 0 {
        crate::kprintln!(
            "[syscall] SYSCALL/SYSRET MSRs configurados (LSTAR: {:#x}).",
            lstar
        );
    }
}
pub unsafe fn init_syscall_msrs() {
    init_syscall_msrs_for_cpu(0);
}
extern "C" {
    pub fn syscall_entry_asm();
    pub fn syscall_handler_asm();
}
#[no_mangle]
pub extern "C" fn syscall_handler_rust(stack_pointer: u64) -> u64 {
    let context = unsafe { &mut *(stack_pointer as *mut crate::task::process::InterruptContext) };
    let sys_num = context.rax;
    context.rax = syscall_dispatcher(
        sys_num,
        context.rdi,
        context.rsi,
        context.rdx,
        context.r10,
        context.r8,
    );
    stack_pointer
}
pub fn validate_user_buffer(ptr: u64, len: usize, writeable: bool) -> Result<(), ()> {
    if len == 0 {
        return Ok(());
    }
    let end = ptr.checked_add(len as u64).ok_or(())?;
    if ptr < crate::memory::address::USER_ADDRESS_START
        || end > crate::memory::address::USER_ADDRESS_END
    {
        return Err(());
    }
    let page_size = crate::memory::pmm::PAGE_SIZE as u64;
    let valid = crate::task::scheduler::with_current_process(|current| {
        let mut curr_page = ptr & !(page_size - 1);
        while curr_page < end {
            match current.vma_mgr.find_vma(curr_page) {
                Some(vma)
                    if !writeable || vma.flags.contains(crate::memory::vma::VmaFlags::WRITE) =>
                {
                    curr_page = vma.end;
                }
                _ => return false,
            }
        }
        let mut curr_page = ptr & !(page_size - 1);
        while curr_page < end {
            if unsafe { crate::memory::vmm::get_mapped_frame(current.pml4_frame, curr_page) }
                .is_none()
            {
                let error_code = if writeable { 0b110 } else { 0b100 };
                if current
                    .vma_mgr
                    .handle_page_fault(current.pml4_frame, curr_page, error_code)
                    .is_err()
                {
                    return false;
                }
                if unsafe { crate::memory::vmm::get_mapped_frame(current.pml4_frame, curr_page) }
                    .is_none()
                {
                    return false;
                }
            }
            curr_page = curr_page.saturating_add(page_size);
        }
        true
    })
    .unwrap_or(false);
    if valid {
        Ok(())
    } else {
        Err(())
    }
}
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn copy_from_user<T: Copy>(src: *const T) -> Result<T, ()> {
    validate_user_buffer(src as u64, core::mem::size_of::<T>(), false)?;
    Ok(unsafe { core::ptr::read_unaligned(src) })
}
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn copy_from_user_slice(src: *const u8, dst: &mut [u8]) -> Result<(), ()> {
    if dst.is_empty() {
        return Ok(());
    }
    validate_user_buffer(src as u64, dst.len(), false)?;
    unsafe {
        core::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), dst.len());
    }
    Ok(())
}
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn copy_to_user_slice(dst: *mut u8, src: &[u8]) -> Result<(), ()> {
    if src.is_empty() {
        return Ok(());
    }
    validate_user_buffer(dst as u64, src.len(), true)?;
    unsafe {
        core::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
    }
    Ok(())
}
pub fn copy_user_string(src: *const u8, max_len: usize) -> Result<alloc::string::String, ()> {
    if max_len == 0 || max_len > MAX_PATH_BYTES {
        return Err(());
    }
    let mut buf = alloc::vec![0u8; max_len];
    copy_from_user_slice(src, &mut buf)?;
    alloc::string::String::from_utf8(buf).map_err(|_| ())
}
pub unsafe fn copy_to_user<T: Copy>(dst: *mut T, src: &T) -> Result<(), ()> {
    validate_user_buffer(dst as u64, core::mem::size_of::<T>(), true)?;
    unsafe {
        core::ptr::copy_nonoverlapping(src as *const T, dst, 1);
    }
    Ok(())
}
fn finish(result: Result<u64, u64>) -> u64 {
    result.unwrap_or_else(|error| error)
}
#[no_mangle]
pub extern "C" fn syscall_dispatcher(
    sys_num: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    _arg5: u64,
) -> u64 {
    unsafe {
        core::arch::asm!("sti", options(nomem, nostack));
    }
    let canonical = match sys_num {
        60 => lix_sys::SYS_PROCESS_EXIT as u64,
        35 => lix_sys::SYS_SLEEP as u64,
        169 => lix_sys::SYS_POWER as u64,
        value => value,
    };
    let Some(entry) = crate::task::syscall_table::lookup_syscall(canonical) else {
        return -(lix_sys::ErrorCode::ENOSYS as i32) as u64;
    };
    let Some(required) = crate::task::rights::required(entry.required_rights) else {
        return -(lix_sys::ErrorCode::EACCES as i32) as u64;
    };
    let granted =
        crate::task::scheduler::with_current_process(|current| current.rights.contains(required))
            .unwrap_or(false);
    if !granted {
        return -(lix_sys::ErrorCode::EACCES as i32) as u64;
    }
    match canonical as usize {
        x if x == lix_sys::SYS_ABI_VERSION => 0x00000001,
        x if x == lix_sys::SYS_HANDLE_CLOSE => io::sys_handle_close(arg1),
        x if x == lix_sys::SYS_HANDLE_DUP => io::sys_handle_dup(arg1, arg2),
        x if x == lix_sys::SYS_HANDLE_DUP2 => io::sys_handle_dup2(arg1, arg2),
        x if x == lix_sys::SYS_OPEN => io::sys_open(arg1, arg2, arg3),
        x if x == lix_sys::SYS_READ => io::sys_read(arg1, arg2, arg3 as usize),
        x if x == lix_sys::SYS_WRITE => io::sys_write(arg1, arg2, arg3),
        x if x == lix_sys::SYS_LSEEK => io::sys_lseek(arg1, arg2 as i64, arg3),
        x if x == lix_sys::SYS_READ_NONBLOCKING => {
            io::sys_read_nonblocking(arg1, arg2, arg3 as usize)
        }
        x if x == lix_sys::SYS_LIST_DIR => {
            finish(io::sys_list_dir(arg1, arg2 as usize, arg3, arg4 as usize))
        }
        x if x == lix_sys::SYS_READ_FILE => {
            finish(io::sys_read_file(arg1, arg2 as usize, arg3, arg4 as usize))
        }
        x if x == lix_sys::SYS_WRITE_FILE => {
            finish(io::sys_write_file(arg1, arg2 as usize, arg3, arg4 as usize))
        }
        x if x == lix_sys::SYS_REMOVE_FILE => {
            finish(io::sys_remove_file(arg1, arg2 as usize, arg3))
        }
        x if x == lix_sys::SYS_MKDIR => finish(io::sys_mkdir(arg1, arg2 as usize)),
        x if x == lix_sys::SYS_LOG_WRITE => finish(io::sys_log_write(arg1, arg2)),
        x if x == lix_sys::SYS_PROCESS_SPAWN => proc::sys_process_spawn(arg1, arg2 as usize),
        x if x == lix_sys::SYS_PROCESS_EXIT => proc::sys_process_exit(arg1 as i32),
        x if x == lix_sys::SYS_PROCESS_KILL => proc::sys_process_kill(arg1, arg2 as u8),
        x if x == lix_sys::SYS_WAITPID => proc::sys_waitpid(arg1, arg2),
        x if x == lix_sys::SYS_PROCESS_LIST => proc::sys_process_list(arg1, arg2 as usize),
        x if x == lix_sys::SYS_SLEEP => proc::sys_sleep(arg1),
        x if x == lix_sys::SYS_MMAP => mem::sys_mmap(arg1, arg2, arg3, arg4),
        x if x == lix_sys::SYS_MUNMAP => mem::sys_munmap(arg1, arg2),
        x if x == lix_sys::SYS_CHANNEL_CREATE => channel::sys_channel_create(arg1),
        x if x == lix_sys::SYS_CHANNEL_SEND => {
            channel::sys_channel_send(arg1, arg2, arg3, arg4, _arg5)
        }
        x if x == lix_sys::SYS_CHANNEL_RECV => {
            channel::sys_channel_recv(arg1, arg2, arg3, arg4, _arg5)
        }
        x if x == lix_sys::SYS_TERMINAL_INFO => term::sys_terminal_info(arg1),
        x if x == lix_sys::SYS_TERMINAL_SET_SCALE => term::sys_terminal_set_scale(arg1 as i64),
        x if x == lix_sys::SYS_SET_KEYMAP => term::sys_set_keymap(arg1 as u8),
        x if x == lix_sys::SYS_GET_KEYMAP => term::sys_get_keymap(),
        x if x == lix_sys::SYS_INPUT_OPEN => term::sys_input_open(arg1),
        x if x == lix_sys::SYS_USB_ENUMERATE => term::sys_usb_enumerate(arg1, arg2),
        x if x == lix_sys::SYS_SOCKET_OPEN => net::sys_socket_open(),
        x if x == lix_sys::SYS_SOCKET_BIND => net::sys_socket_bind(arg1, arg2),
        x if x == lix_sys::SYS_SOCKET_SENDTO => {
            net::sys_socket_sendto(arg1, arg2, arg3, arg4 as usize)
        }
        x if x == lix_sys::SYS_SOCKET_RECVFROM => {
            net::sys_socket_recvfrom(arg1, arg2, arg3 as usize, arg4)
        }
        x if x == lix_sys::SYS_MONITOR_METRICS => term::sys_monitor_metrics(arg1),
        x if x == lix_sys::SYS_POWER => {
            term::sys_power();
            0
        }
        x if x == lix_sys::SYS_BLOCK_OPEN => block::sys_block_open(arg1, arg2),
        x if x == lix_sys::SYS_BLOCK_INFO => block::sys_block_info(arg1, arg2),
        x if x == lix_sys::SYS_BLOCK_READ => block::sys_block_read(arg1, arg2, arg3, arg4),
        x if x == lix_sys::SYS_IO_URING_SETUP => crate::task::syscall::io_uring::sys_uring_setup(arg1),
        x if x == lix_sys::SYS_IO_URING_SUBMIT => crate::task::syscall::io_uring::sys_uring_submit(arg1, arg2 as u8, arg3, arg4),
        x if x == lix_sys::SYS_IO_URING_ENTER => crate::task::syscall::io_uring::sys_uring_enter(arg1),
        x if x == lix_sys::SYS_IO_URING_PEEK => crate::task::syscall::io_uring::sys_uring_peek(arg1, arg2),
        x if x == lix_sys::SYS_BLOCK_WRITE => block::sys_block_write(arg1, arg2, arg3, arg4),
        x if x == lix_sys::SYS_BLOCK_SYNC => block::sys_block_sync(arg1),
        _ => {
            crate::kprintln!("[syscall] Syscall no implementada: {:#x}", canonical);
            -(lix_sys::ErrorCode::ENOSYS as i32) as u64
        }
    }
}
