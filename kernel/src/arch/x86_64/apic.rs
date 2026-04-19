use x86_64::registers::model_specific::Msr;
use core::ptr::{read_volatile, write_volatile};

const IA32_APIC_BASE_MSR: u32 = 0x1B;
const SPURIOUS_INTERRUPT_VECTOR: u32 = 0x0F0;
const APIC_ID_REGISTER: u32 = 0x020;
const EOI_REGISTER: u32 = 0x0B0;
const TIMER_LVT: u32 = 0x320;
const TIMER_INITIAL_COUNT: u32 = 0x380;
const TIMER_CURRENT_COUNT: u32 = 0x390;
const TIMER_DIVIDE_CONFIG: u32 = 0x3E0;

static mut LAPIC_VADDR: u64 = 0;

pub unsafe fn init() {
    crate::kprintln!("[apic] Inicializando Local APIC...");
    
    let mut msr = Msr::new(IA32_APIC_BASE_MSR);
    let apic_base = msr.read();
    
    let phys_addr = apic_base & 0xFFFFF000;
    
    let hhdm_offset = crate::HHDM_REQUEST.get_response().map(|r| r.offset()).unwrap_or(0xffff800000000000);
    LAPIC_VADDR = hhdm_offset + phys_addr;
    
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3);
    let pml4_frame = (cr3 & 0x000FFFFFFFFFF000) as usize / 4096;
    
    if let Err(e) = crate::memory::vmm::map_page(pml4_frame, LAPIC_VADDR, phys_addr, false, true, true) {
        crate::kprintln!("[apic] Error fatal mapeando APIC: {}", e);
        crate::arch::x86_64::halt_loop();
    }
    
    let spurious_vector: u32 = 0xFF;
    write_reg(SPURIOUS_INTERRUPT_VECTOR, spurious_vector | 0x100);

    let apic_id = read_reg(APIC_ID_REGISTER) >> 24;
    crate::kprintln!("[apic] LAPIC habilitado en {:#x}. BSP Core ID: {}", LAPIC_VADDR, apic_id);
    
    write_reg(TIMER_DIVIDE_CONFIG, 0x03);
    write_reg(TIMER_LVT, 32 | 0x20000); 
    write_reg(TIMER_INITIAL_COUNT, 0x1000000);
    
    crate::kprintln!("[apic] APIC Timer configurado en Vector 32.");
}

unsafe fn write_reg(offset: u32, value: u32) {
    let ptr = (LAPIC_VADDR + offset as u64) as *mut u32;
    write_volatile(ptr, value);
}

unsafe fn read_reg(offset: u32) -> u32 {
    let ptr = (LAPIC_VADDR + offset as u64) as *const u32;
    read_volatile(ptr)
}

pub unsafe fn end_of_interrupt() {
    write_reg(EOI_REGISTER, 0);
}