pub mod proc;
pub mod io;
pub mod block;
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
        crate::kprintln!("[syscall] SYSCALL/SYSRET MSRs configurados (LSTAR: {:#x}).", lstar);
    }
}

pub unsafe fn init_syscall_msrs() {
    init_syscall_msrs_for_cpu(0);
}

#[unsafe(naked)]
pub extern "C" fn syscall_entry_asm() {
    core::arch::naked_asm!(
        "swapgs",
        "mov gs:[8], rsp",
        "mov rsp, gs:[0]",
        "push qword ptr gs:[8]",
        "push r11",
        "push rcx",
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "push rdi",
        "push rsi",
        "push rdx",
        "push r10",
        "push r8",
        "push r9",
        "sub rsp, 8",
        "mov rdi, rax",
        "mov rsi, [rsp + 48]",
        "mov rdx, [rsp + 40]",
        "mov rcx, [rsp + 32]",
        "mov r8,  [rsp + 24]",
        "mov r9,  [rsp + 16]",
        "mov rax, cr3",
        "push rax",
        "cld",
        "call syscall_dispatcher",
        "pop rbx",
        "add rsp, 8",
        "pop r9",
        "pop r8",
        "pop r10",
        "pop rdx",
        "pop rsi",
        "pop rdi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "pop rcx",
        "pop r11",
        "pop rsp",
        "swapgs",
        "sysretq"
    );
}

#[unsafe(naked)]
pub extern "C" fn syscall_handler_asm() {
    core::arch::naked_asm!(
        "push rax", "push rcx", "push rdx", "push rbx",
        "push rbp", "push rsi", "push rdi",
        "push r8", "push r9", "push r10", "push r11",
        "push r12", "push r13", "push r14", "push r15",
        "mov rdi, rsp",
        "and rsp, -16",
        "call syscall_handler_rust",
        "mov rsp, rax",
        "pop r15", "pop r14", "pop r13", "pop r12",
        "pop r11", "pop r10", "pop r9", "pop r8",
        "pop rdi", "pop rsi", "pop rbp",
        "pop rbx", "pop rdx", "pop rcx", "pop rax",
        "iretq"
    );
}

#[no_mangle]
pub extern "C" fn syscall_handler_rust(stack_pointer: u64) -> u64 {
    let context = unsafe { &mut *(stack_pointer as *mut crate::task::process::InterruptContext) };
    let sys_num = context.rax;
    context.rax = syscall_dispatcher(sys_num, context.rdi, context.rsi, context.rdx, context.r10, context.r8);
    stack_pointer
}

pub fn validate_user_buffer(ptr: u64, len: usize, writeable: bool) -> Result<(), ()> {
    if len == 0 {
        return Ok(());
    }
    let end = ptr.checked_add(len as u64).ok_or(())?;
    if ptr < crate::memory::address::USER_ADDRESS_START || end > crate::memory::address::USER_ADDRESS_END {
        return Err(());
    }

    let valid = crate::task::scheduler::with_current_process(|current| {
        let vma_mgr = &current.vma_mgr;
        let mut curr_page = ptr & !0xFFF;
        while curr_page < end {
            if let Some(vma) = vma_mgr.find_vma(curr_page) {
                if writeable && !vma.flags.contains(crate::memory::vma::VmaFlags::WRITE) {
                    return false;
                }
                curr_page = vma.end;
            } else {
                return false;
            }
        }
        true
    }).unwrap_or(true);

    if !valid {
        return Err(());
    }

    Ok(())
}

pub unsafe fn user_slice<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], ()> {
    validate_user_buffer(ptr as u64, len, false)?;
    Ok(unsafe { core::slice::from_raw_parts(ptr, len) })
}

pub unsafe fn user_slice_mut<'a>(ptr: *mut u8, len: usize) -> Result<&'a mut [u8], ()> {
    validate_user_buffer(ptr as u64, len, true)?;
    Ok(unsafe { core::slice::from_raw_parts_mut(ptr, len) })
}

pub unsafe fn copy_to_user<T: Copy>(dst: *mut T, src: &T) -> Result<(), ()> {
    validate_user_buffer(dst as u64, core::mem::size_of::<T>(), true)?;
    unsafe {
        core::ptr::copy_nonoverlapping(src as *const T, dst, 1);
    }
    Ok(())
}

#[no_mangle]
pub extern "C" fn syscall_dispatcher(sys_num: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64, _arg5: u64) -> u64 {
    match sys_num {
        0x000 => 0x00000001,
        0x001 => {
            if arg2 >= 0x1000 && arg3 > 0 {
                io::sys_write(1, arg2, arg3)
            } else {
                io::sys_handle_close(arg1)
            }
        }
        0x002 => io::sys_handle_dup(arg1, arg2),
        0x301 => io::sys_read(arg1, arg2, arg3 as usize),
        0x302 => io::sys_write(arg1, arg2, arg3),
        0x309 => io::sys_read_nonblocking(arg1, arg2, arg3 as usize),
        0x306 => io::sys_list_dir(arg1 as *const u8, arg2 as usize, arg3 as *mut u8),
        0x307 => io::sys_read_file(arg1 as *const u8, arg2 as usize, arg3 as *mut u8, arg4 as usize),

        0x100 => proc::sys_process_spawn(arg1 as *const u8, arg2 as usize),
        60 | 0x101 => proc::sys_process_exit(arg1 as i32),
        0x102 => proc::sys_process_kill(arg1, arg2 as u8),
        0x10A => proc::sys_waitpid(arg1, arg2 as *mut i32),
        0x10C => proc::sys_process_list(arg1 as *mut crate::task::process::ProcessInfo, arg2 as usize),
        35 | 0x107 => proc::sys_sleep(arg1),

        0x703 => term::sys_terminal_info(arg1),
        0x704 => term::sys_terminal_set_scale(arg1 as i64),
        0x706 => term::sys_set_keymap(arg1 as u8),
        0x707 => term::sys_get_keymap(),
        0x80A => term::sys_monitor_metrics(arg1),
        0x803 | 169 => {
            term::sys_power();
            0
        }

        value if value == lix_sys::SYS_BLOCK_OPEN as u64 => block::sys_block_open(arg1, arg2),
        value if value == lix_sys::SYS_BLOCK_INFO as u64 => block::sys_block_info(arg1, arg2),
        value if value == lix_sys::SYS_BLOCK_READ as u64 => block::sys_block_read(arg1, arg2, arg3, arg4),
        value if value == lix_sys::SYS_BLOCK_WRITE as u64 => block::sys_block_write(arg1, arg2, arg3, arg4),
        value if value == lix_sys::SYS_BLOCK_SYNC as u64 => block::sys_block_sync(arg1),

        _ => {
            crate::kprintln!("[syscall] Syscall no implementada: {:#x}", sys_num);
            0xFFFFFFFFFFFFFFFF
        }
    }
}
