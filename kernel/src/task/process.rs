use alloc::string::String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProcessId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    Blocked,
    Sleeping(u64),
    Zombie,
    Dead,
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

#[derive(Debug, Default)]
#[repr(C)]
pub struct InterruptContext {
    pub r15: u64, pub r14: u64, pub r13: u64, pub r12: u64,
    pub r11: u64, pub r10: u64, pub r9: u64,  pub r8: u64,
    pub rdi: u64, pub rsi: u64, pub rbp: u64, pub rbx: u64,
    pub rdx: u64, pub rcx: u64, pub rax: u64,
    
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

pub struct Process {
    pub id: ProcessId,
    pub name: String,
    pub state: ProcessState,
    pub stack_pointer: u64,
    pub cr3: u64,
    
    pub cpu_ticks: u64,
    pub ram_bytes_used: u64,
    pub gpu_context: u64,

    // Signal Infrastructure
    pub pending_signals: u64,
    pub kernel_stack_ptr: u64,
    pub pml4_frame: usize,
    pub privilege_level: u8,
}

impl Process {
    pub fn new(id: ProcessId, name: String, stack_pointer: u64, cr3: u64, kernel_stack_ptr: u64, pml4_frame: usize, privilege_level: u8) -> Self {
        Self {
            id,
            name,
            state: ProcessState::Ready,
            stack_pointer,
            cr3,
            cpu_ticks: 0,
            ram_bytes_used: 0,
            gpu_context: 0,
            pending_signals: 0,
            kernel_stack_ptr,
            pml4_frame,
            privilege_level,
        }
    }
}