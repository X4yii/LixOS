use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
static TLB_SHOOTDOWN_TARGET: AtomicU64 = AtomicU64::new(0);
static TLB_SHOOTDOWN_PAGES: AtomicUsize = AtomicUsize::new(0);
static SHOOTDOWN_ACK_MASK: AtomicU32 = AtomicU32::new(0);
static SHOOTDOWN_LOCK: crate::sync::IrqSafeSpinlock<()> = crate::sync::IrqSafeSpinlock::new(());
pub fn flush_tlb_local(vaddr: u64, page_count: usize) {
    if vaddr == 0 || page_count > 32 {
        unsafe {
            let cr3: u64;
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
            core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nomem, nostack, preserves_flags));
        }
    } else {
        for i in 0..page_count {
            let page = vaddr + (i * 4096) as u64;
            unsafe {
                core::arch::asm!("invlpg [{}]", in(reg) page, options(nostack, preserves_flags));
            }
        }
    }
}
pub fn handle_tlb_shootdown_local() {
    let target = TLB_SHOOTDOWN_TARGET.load(Ordering::Acquire);
    let count = TLB_SHOOTDOWN_PAGES.load(Ordering::Acquire);
    flush_tlb_local(target, count);
    let cpu = crate::arch::x86_64::percpu::current_cpu_id() as usize;
    SHOOTDOWN_ACK_MASK.fetch_and(!(1 << cpu), Ordering::Release);
}
/// Número de iteraciones de giro antes de reenviar las IPIs de ack.
const TLB_RESEND_INTERVAL: u64 = 100_000_000;
/// Iteraciones de giro antes de rendirse (con ~4 ciclos por iteración son
/// varios segundos). Preferible un aviso ruidoso a colgar la máquina.
const TLB_ACK_TIMEOUT: u64 = 400_000_000;
/// Barrera global de TLB.
///
/// Invariantes de bloqueo:
/// * `SHOOTDOWN_LOCK` es el único lock en juego y se espera girando **con las
///   IRQs en su estado original** (`IrqSafeSpinlock::lock` solo apaga IRQs en la
///   ventana de adquisición). Girar con IRQs apagadas impediría que este CPU
///   atendiera la IPI de ack de otro shootdown concurrente.
/// * La espera de ACKs se hace con IRQs apagadas: `SHOOTDOWN_LOCK` está tomado y
///   ningún handler que intente otro shootdown puede corromper los globales del
///   protocolo (`TARGET`/`PAGES`/`ACK_MASK`). Por eso `flush_tlb_global` jamás
///   debe llamarse desde un contexto que espere por este CPU.
pub fn flush_tlb_global(vaddr: u64, page_count: usize) {
    flush_tlb_local(vaddr, page_count);
    let online = crate::arch::x86_64::smp::ONLINE_CPUS.load(Ordering::Acquire) as usize;
    if online <= 1 {
        return;
    }
    let _guard = SHOOTDOWN_LOCK.lock();
    let my_cpu = crate::arch::x86_64::percpu::current_cpu_id() as usize;
    let mut ack_mask = 0u32;
    for cpu in 0..online.min(32) {
        if cpu != my_cpu {
            ack_mask |= 1 << cpu;
        }
    }
    TLB_SHOOTDOWN_PAGES.store(page_count, Ordering::Release);
    TLB_SHOOTDOWN_TARGET.store(vaddr, Ordering::Release);
    SHOOTDOWN_ACK_MASK.store(ack_mask, Ordering::Release);
    send_shootdown_ipis(ack_mask, my_cpu, online);
    let mut spins: u64 = 0;
    while SHOOTDOWN_ACK_MASK.load(Ordering::Acquire) != 0 {
        core::hint::spin_loop();
        spins += 1;
        if spins % TLB_RESEND_INTERVAL == 0 && spins < TLB_ACK_TIMEOUT {
            send_shootdown_ipis(ack_mask, my_cpu, online);
        }
        if spins >= TLB_ACK_TIMEOUT {
            let pending = SHOOTDOWN_ACK_MASK.load(Ordering::Acquire);
            crate::kprintln!(
                "[tlb] AVISO: timeout esperando ack de shootdown (pendientes {:#x}). \
                 Un CPU no atendió la IPI; puede haber TLBs desactualizados.",
                pending
            );
            break;
        }
    }
}
fn send_shootdown_ipis(ack_mask: u32, my_cpu: usize, online: usize) {
    for cpu in 0..online.min(32) {
        if cpu == my_cpu || (ack_mask & (1 << cpu)) == 0 {
            continue;
        }
        let area = crate::arch::x86_64::percpu::percpu_ptr(cpu);
        if area.is_null() {
            continue;
        }
        let target_lapic = unsafe { (*area).lapic_id as u8 };
        unsafe {
            crate::arch::x86_64::apic::send_tlb_shootdown_ipi(target_lapic);
        }
    }
}
