use x86_64::registers::model_specific::Msr;
use x86_64::structures::tss::TaskStateSegment;
pub const MAX_CPUS: usize = 32;
pub const PCP_CAPACITY: usize = 128;
pub const PCP_BATCH: usize = 32;
#[repr(C)]
pub struct PerCpuPageCache {
    pub count: usize,
    pub frames: [u32; PCP_CAPACITY],
}
impl PerCpuPageCache {
    pub const fn new() -> Self {
        Self {
            count: 0,
            frames: [0; PCP_CAPACITY],
        }
    }
}
#[repr(C, align(64))]
pub struct PerCpuArea {
    pub kernel_rsp: u64,
    pub user_rsp: u64,
    pub self_ptr: u64,
    pub cpu_id: u32,
    pub lapic_id: u32,
    pub current_pid: u64,
    pub tss: TaskStateSegment,
    pub page_cache: PerCpuPageCache,
}
impl PerCpuArea {
    pub const fn new(cpu_id: u32, lapic_id: u32) -> Self {
        Self {
            kernel_rsp: 0,
            user_rsp: 0,
            self_ptr: 0,
            cpu_id,
            lapic_id,
            current_pid: 0,
            tss: TaskStateSegment::new(),
            page_cache: PerCpuPageCache::new(),
        }
    }
}
static mut PERCPU_AREAS: [PerCpuArea; MAX_CPUS] = [
    PerCpuArea::new(0, 0),
    PerCpuArea::new(1, 0),
    PerCpuArea::new(2, 0),
    PerCpuArea::new(3, 0),
    PerCpuArea::new(4, 0),
    PerCpuArea::new(5, 0),
    PerCpuArea::new(6, 0),
    PerCpuArea::new(7, 0),
    PerCpuArea::new(8, 0),
    PerCpuArea::new(9, 0),
    PerCpuArea::new(10, 0),
    PerCpuArea::new(11, 0),
    PerCpuArea::new(12, 0),
    PerCpuArea::new(13, 0),
    PerCpuArea::new(14, 0),
    PerCpuArea::new(15, 0),
    PerCpuArea::new(16, 0),
    PerCpuArea::new(17, 0),
    PerCpuArea::new(18, 0),
    PerCpuArea::new(19, 0),
    PerCpuArea::new(20, 0),
    PerCpuArea::new(21, 0),
    PerCpuArea::new(22, 0),
    PerCpuArea::new(23, 0),
    PerCpuArea::new(24, 0),
    PerCpuArea::new(25, 0),
    PerCpuArea::new(26, 0),
    PerCpuArea::new(27, 0),
    PerCpuArea::new(28, 0),
    PerCpuArea::new(29, 0),
    PerCpuArea::new(30, 0),
    PerCpuArea::new(31, 0),
];
pub fn bsp_percpu_mut() -> &'static mut PerCpuArea {
    unsafe { &mut *core::ptr::addr_of_mut!(PERCPU_AREAS[0]) }
}
pub fn bsp_percpu_ptr() -> *mut PerCpuArea {
    unsafe { core::ptr::addr_of_mut!(PERCPU_AREAS[0]) }
}
pub fn percpu_mut(cpu_id: usize) -> &'static mut PerCpuArea {
    unsafe {
        let idx = cpu_id.min(MAX_CPUS - 1);
        &mut *core::ptr::addr_of_mut!(PERCPU_AREAS[idx])
    }
}
pub fn percpu_ptr(cpu_id: usize) -> *mut PerCpuArea {
    unsafe {
        let idx = cpu_id.min(MAX_CPUS - 1);
        core::ptr::addr_of_mut!(PERCPU_AREAS[idx])
    }
}
pub fn cpu_id_from_lapic(lapic_id: u32) -> usize {
    for i in 0..MAX_CPUS {
        unsafe {
            if PERCPU_AREAS[i].lapic_id == lapic_id {
                return i;
            }
        }
    }
    0
}
pub unsafe fn init_bsp() {
    let ptr = bsp_percpu_ptr() as u64;
    let bsp = bsp_percpu_mut();
    bsp.self_ptr = ptr;
    bsp.cpu_id = 0;
    Msr::new(0xC000_0101).write(ptr);
}
#[inline(always)]
pub fn current_cpu_id() -> u32 {
    let mut self_ptr: u64;
    unsafe {
        core::arch::asm!(
            "mov {}, gs:[16]",
            out(reg) self_ptr,
            options(nomem, nostack, preserves_flags)
        );
    }
    let start = unsafe { core::ptr::addr_of!(PERCPU_AREAS[0]) as u64 };
    let end = start + (core::mem::size_of::<PerCpuArea>() * MAX_CPUS) as u64;
    if self_ptr >= start && self_ptr < end {
        let id: u32;
        unsafe {
            core::arch::asm!(
                "mov {:e}, gs:[24]",
                out(reg) id,
                options(nomem, nostack, preserves_flags)
            );
        }
        id
    } else {
        if !crate::arch::x86_64::apic::is_initialized() {
            return 0;
        }
        let lapic_id = crate::arch::x86_64::apic::read_lapic_id();
        cpu_id_from_lapic(lapic_id) as u32
    }
}
#[inline(always)]
pub fn current_percpu() -> &'static mut PerCpuArea {
    let id = current_cpu_id() as usize;
    percpu_mut(id)
}
