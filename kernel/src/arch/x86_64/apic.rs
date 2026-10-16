use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::registers::model_specific::Msr;
const IA32_APIC_BASE_MSR: u32 = 0x1B;
/// Offset del registro SVR del LAPIC (Software Interrupt Vector Register).
const SVR_REGISTER: u32 = 0x0F0;
/// Vector configurado en el SVR del LAPIC para interrupciones espurias.
/// Debe tener una entrada en la IDT: entregarlo a una puerta vacía provoca #GP.
pub const SPURIOUS_VECTOR: u8 = 0xFF;
const APIC_ID_REGISTER: u32 = 0x020;
const EOI_REGISTER: u32 = 0x0B0;
const TIMER_LVT: u32 = 0x320;
const LVT_LINT0: u32 = 0x350;
const LVT_LINT1: u32 = 0x360;
const TIMER_INITIAL_COUNT: u32 = 0x380;
const TIMER_DIVIDE_CONFIG: u32 = 0x3E0;
const TIMER_CURRENT_COUNT: u32 = 0x390;
const APIC_TIMER_FALLBACK_COUNT: u32 = 0x0010_0000;
const ICR_LOW: u32 = 0x300;
const ICR_HIGH: u32 = 0x310;
static LAPIC_VADDR: AtomicU64 = AtomicU64::new(0);
static CALIBRATED_TIMER_COUNT: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(APIC_TIMER_FALLBACK_COUNT);
pub unsafe fn init() {
    crate::kprintln!("[apic] Inicializando Local APIC...");
    let msr = Msr::new(IA32_APIC_BASE_MSR);
    let apic_base = msr.read();
    let phys_addr = apic_base & 0xFFFFF000;
    let lapic_vaddr = crate::memory::address::hhdm_offset()
        .map_phys(crate::memory::address::PhysAddr::new(phys_addr))
        .unwrap_or_else(|error| panic!("invalid Local APIC address: {error}"))
        .get();
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3);
    let pml4_frame = (cr3 & 0x000FFFFFFFFFF000) as usize / 4096;
    if let Err(e) =
        crate::memory::vmm::map_page(pml4_frame, lapic_vaddr, phys_addr, false, true, true)
    {
        crate::kprintln!("[apic] Error fatal mapeando APIC: {}", e);
        crate::arch::x86_64::halt_loop();
    }
    LAPIC_VADDR.store(lapic_vaddr, Ordering::Release);
    write_reg(SVR_REGISTER, SPURIOUS_VECTOR as u32 | 0x100);
    let apic_id = read_reg(APIC_ID_REGISTER) >> 24;
    crate::arch::x86_64::percpu::bsp_percpu_mut().lapic_id = apic_id;
    crate::kprintln!(
        "[apic] LAPIC habilitado en {:#x}. BSP Core ID: {}",
        lapic_vaddr,
        apic_id
    );
    write_reg(LVT_LINT0, 0x00000700);
    write_reg(LVT_LINT1, 0x00010000);
    write_reg(TIMER_DIVIDE_CONFIG, 0x03);
    write_reg(TIMER_LVT, 32 | 0x20000);
    let initial_count = calibrate_timer_100hz().unwrap_or(APIC_TIMER_FALLBACK_COUNT);
    CALIBRATED_TIMER_COUNT.store(initial_count, Ordering::Release);
    write_reg(TIMER_LVT, 32 | 0x20000);
    write_reg(TIMER_INITIAL_COUNT, initial_count);
    crate::kprintln!(
        "[apic] Timer calibrado: {} cuentas por intervalo de 10 ms",
        initial_count
    );
    crate::kprintln!("[apic] APIC Timer configurado en Vector 32 y LINT0 ExtINT activo.");
}
unsafe fn calibrate_timer_100hz() -> Option<u32> {
    const PIT_COMMAND: u16 = 0x43;
    const PIT_CHANNEL_2: u16 = 0x42;
    const PIT_SPEAKER: u16 = 0x61;
    const PIT_10MS_COUNT: u16 = 11_932;
    write_reg(TIMER_LVT, 32 | 0x10000);
    write_reg(TIMER_INITIAL_COUNT, u32::MAX);
    let apic_before = read_reg(TIMER_CURRENT_COUNT);
    let speaker = crate::arch::x86_64::io::inb(PIT_SPEAKER);
    crate::arch::x86_64::io::outb(PIT_SPEAKER, speaker & !0x02);
    crate::arch::x86_64::io::outb(PIT_COMMAND, 0xB0);
    crate::arch::x86_64::io::outb(PIT_CHANNEL_2, PIT_10MS_COUNT as u8);
    crate::arch::x86_64::io::outb(PIT_CHANNEL_2, (PIT_10MS_COUNT >> 8) as u8);
    crate::arch::x86_64::io::outb(PIT_SPEAKER, (speaker & !0x02) | 0x01);
    let mut timeout = 20_000_000usize;
    while crate::arch::x86_64::io::inb(PIT_SPEAKER) & 0x20 == 0 && timeout > 0 {
        core::hint::spin_loop();
        timeout -= 1;
    }
    crate::arch::x86_64::io::outb(PIT_SPEAKER, speaker);
    if timeout == 0 {
        return None;
    }
    let apic_after = read_reg(TIMER_CURRENT_COUNT);
    let elapsed = apic_before.wrapping_sub(apic_after);
    if elapsed < 1000 {
        return None;
    }
    elapsed.checked_mul(10)
}
unsafe fn write_reg(offset: u32, value: u32) {
    let vaddr = LAPIC_VADDR.load(Ordering::Acquire);
    let ptr = (vaddr + offset as u64) as *mut u32;
    write_volatile(ptr, value);
}
unsafe fn read_reg(offset: u32) -> u32 {
    let vaddr = LAPIC_VADDR.load(Ordering::Acquire);
    let ptr = (vaddr + offset as u64) as *const u32;
    read_volatile(ptr)
}
pub unsafe fn end_of_interrupt() {
    write_reg(EOI_REGISTER, 0);
}
pub fn is_initialized() -> bool {
    LAPIC_VADDR.load(Ordering::Acquire) != 0
}
pub fn read_lapic_id() -> u32 {
    let vaddr = LAPIC_VADDR.load(Ordering::Acquire);
    if vaddr != 0 {
        unsafe { read_reg(APIC_ID_REGISTER) >> 24 }
    } else {
        0
    }
}
pub unsafe fn init_ap() {
    write_reg(SVR_REGISTER, SPURIOUS_VECTOR as u32 | 0x100);
    write_reg(LVT_LINT0, 0x00000700);
    write_reg(LVT_LINT1, 0x00010000);
    write_reg(TIMER_DIVIDE_CONFIG, 0x03);
    write_reg(TIMER_LVT, 32 | 0x20000);
    let count = CALIBRATED_TIMER_COUNT.load(Ordering::Acquire);
    write_reg(TIMER_INITIAL_COUNT, count);
}
pub unsafe fn send_ipi(dest_apic_id: u8, vector: u8, delivery_mode: u32) {
    write_reg(ICR_HIGH, (dest_apic_id as u32) << 24);
    let cmd = (delivery_mode & 0x700) | (1 << 14) | (vector as u32);
    write_reg(ICR_LOW, cmd);
    let mut timeout = 100_000usize;
    while (read_reg(ICR_LOW) & (1 << 12)) != 0 && timeout > 0 {
        core::hint::spin_loop();
        timeout -= 1;
    }
}
pub unsafe fn send_init_ipi(dest_apic_id: u8) {
    write_reg(ICR_HIGH, (dest_apic_id as u32) << 24);
    write_reg(ICR_LOW, 0x0000_4500);
    let mut timeout = 100_000usize;
    while (read_reg(ICR_LOW) & (1 << 12)) != 0 && timeout > 0 {
        core::hint::spin_loop();
        timeout -= 1;
    }
}
pub unsafe fn send_startup_ipi(dest_apic_id: u8, vector: u8) {
    write_reg(ICR_HIGH, (dest_apic_id as u32) << 24);
    write_reg(ICR_LOW, 0x0000_4600 | (vector as u32));
    let mut timeout = 100_000usize;
    while (read_reg(ICR_LOW) & (1 << 12)) != 0 && timeout > 0 {
        core::hint::spin_loop();
        timeout -= 1;
    }
}
pub const RESCHEDULE_IPI_VECTOR: u8 = 0xF0;
pub const TLB_SHOOTDOWN_IPI_VECTOR: u8 = 0xF1;
pub unsafe fn send_reschedule_ipi(dest_apic_id: u8) {
    send_ipi(dest_apic_id, RESCHEDULE_IPI_VECTOR, 0);
}
pub unsafe fn send_tlb_shootdown_ipi(dest_apic_id: u8) {
    send_ipi(dest_apic_id, TLB_SHOOTDOWN_IPI_VECTOR, 0);
}
