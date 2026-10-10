use x86_64::VirtAddr;
use x86_64::structures::gdt::{GlobalDescriptorTable, Descriptor, SegmentSelector};
use lazy_static::lazy_static;
use crate::arch::x86_64::percpu;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

#[repr(align(16))]
struct KernelStack([u8; 4096 * 5]);

static DOUBLE_FAULT_STACK: KernelStack = KernelStack([0; 4096 * 5]);
static PRIVILEGE_STACK: KernelStack = KernelStack([0; 4096 * 5]);

pub fn set_cpu_tss_rsp0(cpu_id: usize, rsp0: u64) {
    let percpu = percpu::percpu_mut(cpu_id);
    percpu.tss.privilege_stack_table[0] = VirtAddr::new(rsp0);
}

pub fn set_tss_rsp0(rsp0: u64) {
    let cpu_id = percpu::current_cpu_id() as usize;
    set_cpu_tss_rsp0(cpu_id, rsp0);
}


lazy_static! {
    pub static ref GDT: (GlobalDescriptorTable, Selectors) = {
        let mut gdt = GlobalDescriptorTable::new();
        let code_selector = gdt.append(Descriptor::kernel_code_segment());
        let data_selector = gdt.append(Descriptor::kernel_data_segment());
        let user_data_selector = gdt.append(Descriptor::user_data_segment());
        let user_code_selector = gdt.append(Descriptor::user_code_segment());
        let percpu = percpu::bsp_percpu_mut();
        let tss_selector = gdt.append(Descriptor::tss_segment(&percpu.tss));
        
        (gdt, Selectors { code_selector, data_selector, user_code_selector, user_data_selector, tss_selector })
    };
}

pub struct Selectors {
    pub code_selector: SegmentSelector,
    pub data_selector: SegmentSelector,
    pub user_code_selector: SegmentSelector,
    pub user_data_selector: SegmentSelector,
    pub tss_selector: SegmentSelector,
}

pub fn init() {
    use x86_64::instructions::tables::load_tss;
    use x86_64::instructions::segmentation::{CS, Segment};

    crate::kprintln!("[gdt] Inicializando GDT y TSS...");
    let percpu = percpu::bsp_percpu_mut();
    percpu.self_ptr = percpu as *mut _ as u64;
    let df_start = VirtAddr::from_ptr(core::ptr::addr_of!(DOUBLE_FAULT_STACK.0));
    percpu.tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = df_start + (4096 * 5) as u64;
    let priv_start = VirtAddr::from_ptr(core::ptr::addr_of!(PRIVILEGE_STACK.0));
    percpu.tss.privilege_stack_table[0] = priv_start + (4096 * 5) as u64;

    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1.code_selector);
        x86_64::instructions::segmentation::SS::set_reg(SegmentSelector::new(0, x86_64::PrivilegeLevel::Ring0));
        load_tss(GDT.1.tss_selector);
    }
}

static mut AP_GDTS: [Option<(GlobalDescriptorTable, Selectors)>; percpu::MAX_CPUS] =
    [const { None }; percpu::MAX_CPUS];
static mut AP_DOUBLE_FAULT_STACKS: [KernelStack; percpu::MAX_CPUS] =
    [const { KernelStack([0; 4096 * 5]) }; percpu::MAX_CPUS];
static mut AP_PRIVILEGE_STACKS: [KernelStack; percpu::MAX_CPUS] =
    [const { KernelStack([0; 4096 * 5]) }; percpu::MAX_CPUS];

pub unsafe fn init_ap_gdt(cpu_id: usize) {
    use x86_64::instructions::tables::load_tss;
    use x86_64::instructions::segmentation::{CS, Segment};

    let idx = cpu_id.min(percpu::MAX_CPUS - 1);
    let percpu = percpu::percpu_mut(idx);
    percpu.self_ptr = percpu as *mut _ as u64;
    percpu.cpu_id = idx as u32;

    let df_ptr = core::ptr::addr_of_mut!(AP_DOUBLE_FAULT_STACKS[idx].0);
    percpu.tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
        VirtAddr::from_ptr(df_ptr) + (4096 * 5) as u64;

    let priv_ptr = core::ptr::addr_of_mut!(AP_PRIVILEGE_STACKS[idx].0);
    percpu.tss.privilege_stack_table[0] =
        VirtAddr::from_ptr(priv_ptr) + (4096 * 5) as u64;

    let mut gdt = GlobalDescriptorTable::new();
    let code_selector = gdt.append(Descriptor::kernel_code_segment());
    let data_selector = gdt.append(Descriptor::kernel_data_segment());
    let user_data_selector = gdt.append(Descriptor::user_data_segment());
    let user_code_selector = gdt.append(Descriptor::user_code_segment());
    let tss_selector = gdt.append(Descriptor::tss_segment(&percpu.tss));

    let ptr = core::ptr::addr_of_mut!(AP_GDTS[idx]);
    *ptr = Some((gdt, Selectors { code_selector, data_selector, user_code_selector, user_data_selector, tss_selector }));

    if let Some((ref gdt_ref, ref sel)) = *ptr {
        gdt_ref.load();
        CS::set_reg(sel.code_selector);
        x86_64::instructions::segmentation::SS::set_reg(SegmentSelector::new(0, x86_64::PrivilegeLevel::Ring0));
        load_tss(sel.tss_selector);
    }
}