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
    pub disk_read_bytes: u64,
    pub gpu_context: u64,

    pub pending_signals: u64,
    pub waiting_for: Option<ProcessId>,
    pub kernel_stack_ptr: u64,
    pub pml4_frame: usize,
    pub privilege_level: u8,
    pub handles: crate::task::handle::HandleTable,
    pub priority: u8,
    pub quantum_remaining: u8,
    pub vma_mgr: crate::memory::vma::VmaManager,
    pub affinity_mask: u64,
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
            disk_read_bytes: 0,
            gpu_context: 0,
            pending_signals: 0,
            waiting_for: None,
            kernel_stack_ptr,
            pml4_frame,
            privilege_level,
            handles: crate::task::handle::HandleTable::new(),
            priority: 16,
            quantum_remaining: 4,
            vma_mgr: crate::memory::vma::VmaManager::new(),
            affinity_mask: u64::MAX,
        }
    }
}

