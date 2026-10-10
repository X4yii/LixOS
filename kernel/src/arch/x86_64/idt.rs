use x86_64::structures::idt::InterruptDescriptorTable;
use lazy_static::lazy_static;
use crate::arch::x86_64::gdt;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ExceptionFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub error_code: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

extern "C" {
    pub fn keyboard_interrupt_handler_asm();
    pub fn page_fault_handler_asm();
    pub fn gpf_handler_asm();
    pub fn double_fault_handler_asm();
    pub fn divide_error_handler_asm();
    pub fn invalid_opcode_handler_asm();
    pub fn breakpoint_handler_asm();
}

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        
        unsafe {
            idt[InterruptIndex::Timer.as_u8()].set_handler_addr(
                x86_64::VirtAddr::new(crate::task::scheduler::timer_interrupt_handler_asm as *const () as u64)
            );
            idt[InterruptIndex::Keyboard.as_u8()].set_handler_addr(
                x86_64::VirtAddr::new(keyboard_interrupt_handler_asm as *const () as u64)
            );

            idt[0xF0].set_handler_addr(
                x86_64::VirtAddr::new(crate::task::scheduler::reschedule_interrupt_handler_asm as *const () as u64)
            );
            
            idt[0x80].set_handler_addr(
                x86_64::VirtAddr::new(crate::task::syscall::syscall_handler_asm as *const () as u64)
            ).set_privilege_level(x86_64::PrivilegeLevel::Ring3);

            idt.page_fault.set_handler_addr(x86_64::VirtAddr::new(page_fault_handler_asm as *const () as u64));
            idt.general_protection_fault.set_handler_addr(x86_64::VirtAddr::new(gpf_handler_asm as *const () as u64));
            idt.double_fault.set_handler_addr(x86_64::VirtAddr::new(double_fault_handler_asm as *const () as u64))
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
            idt.invalid_opcode.set_handler_addr(x86_64::VirtAddr::new(invalid_opcode_handler_asm as *const () as u64));
            idt.divide_error.set_handler_addr(x86_64::VirtAddr::new(divide_error_handler_asm as *const () as u64));
            idt.breakpoint.set_handler_addr(x86_64::VirtAddr::new(breakpoint_handler_asm as *const () as u64));
        }
        
        idt
    };
}

pub fn init() {
    crate::kprintln!("[idt] Cargando Interrupt Descriptor Table...");
    IDT.load();
}

pub fn load_idt() {
    IDT.load();
}

#[no_mangle]
pub extern "C" fn breakpoint_handler_rust(frame_ptr: *mut ExceptionFrame) {
    let frame = unsafe { &mut *frame_ptr };
    crate::kprintln!("EXCEPCIÓN: BREAKPOINT en RIP: {:#x}, RSP: {:#x}", frame.rip, frame.rsp);
}

#[no_mangle]
pub extern "C" fn double_fault_handler_rust(frame_ptr: *mut ExceptionFrame) -> ! {
    let frame = unsafe { &mut *frame_ptr };
    panic!("EXCEPCIÓN CATASTRÓFICA: DOUBLE FAULT (Code: {:#x})\nRIP: {:#x}, RSP: {:#x}", frame.error_code, frame.rip, frame.rsp);
}

fn handle_user_fault(fault_name: &str, rip: u64, cs: u64, rsp: u64, error_code: Option<u64>) -> bool {
    if (cs & 3) != 3 {
        return false;
    }
    let cpu_id = crate::arch::x86_64::percpu::current_cpu_id() as usize;
    let mut rq = crate::task::scheduler::RUNQUEUES[cpu_id].lock();
    if let Some(mut current) = rq.current.take() {
        crate::kprintln!("\n[!] FATAL: Proceso Userspace '{}' (PID {}) provoco un {}.", current.name, current.id.0, fault_name);
        crate::kprintln!("    RIP: {:#x}, RSP: {:#x}, Code: {:?}", rip, rsp, error_code);
        crate::kprintln!("[!] Terminando proceso...");
        current.state = crate::task::process::ProcessState::Dead;
        let pid = current.id;
        crate::task::scheduler::ZOMBIES.lock().push_back(current);
        drop(rq);
        crate::task::scheduler::wake_waiters_for(pid);
        crate::task::scheduler::yield_current();
        loop {}
    }
    false
}

#[no_mangle]
pub extern "C" fn page_fault_handler_rust(frame_ptr: *mut ExceptionFrame) {
    let frame = unsafe { &mut *frame_ptr };
    let cr2: u64;
    unsafe { core::arch::asm!("mov {}, cr2", out(reg) cr2); }

    let is_user = (frame.cs & 3) == 3;
    if is_user {
        let mut resolved = false;
        crate::task::scheduler::with_current_process(|current| {
            if current.vma_mgr.handle_page_fault(current.pml4_frame, cr2, frame.error_code).is_ok() {
                resolved = true;
            }
        });
        if resolved {
            return;
        }
    }

    if handle_user_fault("PAGE FAULT", frame.rip, frame.cs, frame.rsp, Some(frame.error_code)) {
        return;
    }

    crate::kprintln!("EXCEPCIÓN KERNEL: PAGE FAULT");
    crate::kprintln!("Accessed Address: {:#x}", cr2);
    crate::kprintln!("Error Code: {:#x}", frame.error_code);
    crate::kprintln!("RIP: {:#x}, RSP: {:#x}, CS: {:#x}", frame.rip, frame.rsp, frame.cs);
    crate::arch::x86_64::halt_loop();
}

#[no_mangle]
pub extern "C" fn gpf_handler_rust(frame_ptr: *mut ExceptionFrame) {
    let frame = unsafe { &mut *frame_ptr };
    if handle_user_fault("GENERAL PROTECTION FAULT", frame.rip, frame.cs, frame.rsp, Some(frame.error_code)) {
        return;
    }
    panic!("EXCEPCIÓN KERNEL: GENERAL PROTECTION FAULT (Code: {:#x})\nRIP: {:#x}, RSP: {:#x}", frame.error_code, frame.rip, frame.rsp);
}

#[no_mangle]
pub extern "C" fn invalid_opcode_handler_rust(frame_ptr: *mut ExceptionFrame) {
    let frame = unsafe { &mut *frame_ptr };
    if handle_user_fault("INVALID OPCODE", frame.rip, frame.cs, frame.rsp, None) {
        return;
    }
    panic!("EXCEPCIÓN KERNEL: INVALID OPCODE\nRIP: {:#x}, RSP: {:#x}", frame.rip, frame.rsp);
}

#[no_mangle]
pub extern "C" fn divide_error_handler_rust(frame_ptr: *mut ExceptionFrame) {
    let frame = unsafe { &mut *frame_ptr };
    if handle_user_fault("DIVIDE BY ZERO", frame.rip, frame.cs, frame.rsp, None) {
        return;
    }
    panic!("EXCEPCIÓN KERNEL: DIVIDE BY ZERO\nRIP: {:#x}, RSP: {:#x}", frame.rip, frame.rsp);
}

#[no_mangle]
pub extern "C" fn keyboard_interrupt_handler_rust(stack_pointer: u64) -> u64 {
    let should_reschedule = crate::drivers::ps2_keyboard::handle_interrupt();
    
    unsafe {
        crate::arch::x86_64::pic::PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
        crate::arch::x86_64::apic::end_of_interrupt();
    }

    if should_reschedule {
        let cpu_id = crate::arch::x86_64::percpu::current_cpu_id() as usize;
        crate::task::scheduler::schedule_on_cpu(cpu_id, stack_pointer, false)
    } else {
        stack_pointer
    }
}
