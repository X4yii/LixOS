use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
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

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        unsafe {
            idt.double_fault.set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.general_protection_fault.set_handler_fn(gpf_handler);
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        idt.divide_error.set_handler_fn(divide_error_handler);
        
        unsafe {
            idt[InterruptIndex::Timer.as_u8()].set_handler_addr(
                x86_64::VirtAddr::new(crate::task::scheduler::timer_interrupt_handler_asm as *const () as u64)
            );
        }
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);

        unsafe {
            idt[0xF0].set_handler_addr(
                x86_64::VirtAddr::new(crate::task::scheduler::reschedule_interrupt_handler_asm as *const () as u64)
            );
        }
        
        unsafe {
            idt[0x80].set_handler_addr(
                x86_64::VirtAddr::new(crate::task::syscall::syscall_handler_asm as *const () as u64)
            ).set_privilege_level(x86_64::PrivilegeLevel::Ring3);
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



extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    crate::kprintln!("EXCEPCIÓN: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(stack_frame: InterruptStackFrame, _error_code: u64) -> ! {
    panic!("EXCEPCIÓN CATASTRÓFICA: DOUBLE FAULT\n{:#?}", stack_frame);
}

fn handle_user_fault(fault_name: &str, stack_frame: &InterruptStackFrame, error_code: Option<u64>) -> bool {
    if stack_frame.code_segment.rpl() != x86_64::PrivilegeLevel::Ring3 {
        return false;
    }
    let cpu_id = crate::arch::x86_64::percpu::current_cpu_id() as usize;
    let mut rq = crate::task::scheduler::RUNQUEUES[cpu_id].lock();
    if let Some(mut current) = rq.current.take() {
        crate::kprintln!("\n[!] FATAL: Proceso Userspace '{}' (PID {}) provoco un {}.", current.name, current.id.0, fault_name);
        crate::kprintln!("    RIP: {:#x}, RSP: {:#x}, Code: {:?}",
            stack_frame.instruction_pointer.as_u64(),
            stack_frame.stack_pointer.as_u64(),
            error_code
        );
        crate::kprintln!("[!] Terminando proceso...");
        current.state = crate::task::process::ProcessState::Dead;
        crate::task::scheduler::ZOMBIES.lock().push_back(current);
        drop(rq);
        crate::task::scheduler::yield_current();
        loop {}
    }
    false
}

extern "x86-interrupt" fn page_fault_handler(stack_frame: InterruptStackFrame, error_code: PageFaultErrorCode) {
    use x86_64::registers::control::Cr2;
    let fault_addr = Cr2::read().map(|addr| addr.as_u64()).unwrap_or(0);

    if stack_frame.code_segment.rpl() == x86_64::PrivilegeLevel::Ring3 {
        let mut resolved = false;
        crate::task::scheduler::with_current_process(|current| {
            if current.vma_mgr.handle_page_fault(current.pml4_frame, fault_addr, error_code.bits()).is_ok() {
                resolved = true;
            }
        });
        if resolved {
            return;
        }
    }

    if handle_user_fault("PAGE FAULT", &stack_frame, Some(error_code.bits())) { return; }

    
    crate::kprintln!("EXCEPCIÓN KERNEL: PAGE FAULT");
    crate::kprintln!("Accessed Address: {:?}", Cr2::read());
    crate::kprintln!("Error Code: {:?}", error_code);
    crate::kprintln!("{:#?}", stack_frame);
    crate::arch::x86_64::halt_loop();
}

extern "x86-interrupt" fn gpf_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    if handle_user_fault("GENERAL PROTECTION FAULT", &stack_frame, Some(error_code)) { return; }
    
    panic!("EXCEPCIÓN KERNEL: GENERAL PROTECTION FAULT (Code: {})\n{:#?}", error_code, stack_frame);
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    if handle_user_fault("INVALID OPCODE", &stack_frame, None) { return; }
    
    panic!("EXCEPCIÓN KERNEL: INVALID OPCODE\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn divide_error_handler(stack_frame: InterruptStackFrame) {
    if handle_user_fault("DIVIDE BY ZERO", &stack_frame, None) { return; }
    
    panic!("EXCEPCIÓN KERNEL: DIVIDE BY ZERO\n{:#?}", stack_frame);
}


extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let should_reschedule = crate::drivers::ps2_keyboard::handle_interrupt();
    
    unsafe {
        crate::arch::x86_64::pic::PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
        crate::arch::x86_64::apic::end_of_interrupt();
    }

    if should_reschedule {
        crate::task::scheduler::reschedule_from_interrupt();
    }
}
