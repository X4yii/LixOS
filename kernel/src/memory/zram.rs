use crate::arch::x86_64::percpu::{self, MAX_CPUS};
use crate::sync::IrqSafeSpinlock;
use alloc::vec::Vec;
pub const MAX_ZRAM_SLOTS: usize = 4096;
pub enum SlotState {
    Empty,
    Compressed(Vec<u8>),
    Raw(Vec<u8>),
    OnDisk { lba: u64, size: usize },
}
struct PerCpuBuffer {
    buf: [u8; 8192],
}
impl PerCpuBuffer {
    pub const fn new() -> Self {
        Self { buf: [0u8; 8192] }
    }
}
static PERCPU_COMPRESS_BUFS: [IrqSafeSpinlock<PerCpuBuffer>; MAX_CPUS] = [
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
    IrqSafeSpinlock::new(PerCpuBuffer::new()),
];
pub struct ZramPool {
    pub slots: Vec<SlotState>,
    pub total_compressed_bytes: usize,
    pub total_saved_bytes: usize,
    pub active_slots: usize,
}
impl ZramPool {
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            total_compressed_bytes: 0,
            total_saved_bytes: 0,
            active_slots: 0,
        }
    }
}
static ZRAM: IrqSafeSpinlock<ZramPool> = IrqSafeSpinlock::new(ZramPool::new());
pub fn init() {
    let mut pool = ZRAM.lock();
    pool.slots.reserve_exact(MAX_ZRAM_SLOTS);
    for _ in 0..MAX_ZRAM_SLOTS {
        pool.slots.push(SlotState::Empty);
    }
    crate::kprintln!(
        "[zram] Pool ZRAM LZ4 inicializado ({} slots maximos, buffer per-cpu listo)",
        MAX_ZRAM_SLOTS
    );
}
pub fn compress_page(page_data: &[u8]) -> Result<u32, &'static str> {
    if page_data.len() < 4096 {
        return Err("Tamaño de página inválido para ZRAM");
    }
    let cpu_id = percpu::current_cpu_id() as usize % MAX_CPUS;
    let mut scratch = PERCPU_COMPRESS_BUFS[cpu_id].lock();
    let compressed_len = lz4_flex::block::compress_into(&page_data[..4096], &mut scratch.buf)
        .map_err(|_| "Fallo en compresión LZ4 de ZRAM")?;
    let state = if compressed_len < 4096 {
        let mut v = Vec::new();
        v.try_reserve_exact(compressed_len)
            .map_err(|_| "Sin memoria para almacenar página comprimida")?;
        v.extend_from_slice(&scratch.buf[..compressed_len]);
        SlotState::Compressed(v)
    } else {
        let mut v = Vec::new();
        v.try_reserve_exact(4096)
            .map_err(|_| "Sin memoria para almacenar página en ZRAM")?;
        v.extend_from_slice(&page_data[..4096]);
        SlotState::Raw(v)
    };
    let mut pool = ZRAM.lock();
    for i in 0..pool.slots.len() {
        if matches!(pool.slots[i], SlotState::Empty) {
            match &state {
                SlotState::Compressed(c) => {
                    pool.total_compressed_bytes += c.len();
                    pool.total_saved_bytes += 4096 - c.len();
                }
                SlotState::Raw(_) => {
                    pool.total_compressed_bytes += 4096;
                }
                _ => {}
            }
            pool.active_slots += 1;
            pool.slots[i] = state;
            return Ok(i as u32);
        }
    }
    Err("Pool ZRAM agotado")
}
pub fn decompress_page(slot_id: u32, out_page: &mut [u8]) -> Result<(), &'static str> {
    if out_page.len() < 4096 {
        return Err("Buffer de salida menor a 4096 bytes");
    }
    let mut pool = ZRAM.lock();
    let slot = pool
        .slots
        .get_mut(slot_id as usize)
        .ok_or("Slot ZRAM inexistente")?;
    let old_state = core::mem::replace(slot, SlotState::Empty);
    match old_state {
        SlotState::Compressed(compressed_data) => {
            lz4_flex::block::decompress_into(&compressed_data, &mut out_page[..4096])
                .map_err(|_| "Fallo al descomprimir bloque LZ4 de ZRAM")?;
            pool.active_slots = pool.active_slots.saturating_sub(1);
            pool.total_compressed_bytes = pool
                .total_compressed_bytes
                .saturating_sub(compressed_data.len());
            pool.total_saved_bytes = pool
                .total_saved_bytes
                .saturating_sub(4096 - compressed_data.len());
            Ok(())
        }
        SlotState::Raw(raw_data) => {
            out_page[..4096].copy_from_slice(&raw_data[..4096]);
            pool.active_slots = pool.active_slots.saturating_sub(1);
            pool.total_compressed_bytes = pool.total_compressed_bytes.saturating_sub(4096);
            Ok(())
        }
        SlotState::OnDisk { lba: _, size: _ } => {
            pool.active_slots = pool.active_slots.saturating_sub(1);
            Ok(())
        }
        SlotState::Empty => Err("Slot ZRAM ya vacio"),
    }
}
pub fn free_slot(slot_id: u32) {
    let mut pool = ZRAM.lock();
    if let Some(slot) = pool.slots.get_mut(slot_id as usize) {
        match core::mem::replace(slot, SlotState::Empty) {
            SlotState::Compressed(c) => {
                pool.active_slots = pool.active_slots.saturating_sub(1);
                pool.total_compressed_bytes = pool.total_compressed_bytes.saturating_sub(c.len());
                pool.total_saved_bytes = pool.total_saved_bytes.saturating_sub(4096 - c.len());
            }
            SlotState::Raw(_) => {
                pool.active_slots = pool.active_slots.saturating_sub(1);
                pool.total_compressed_bytes = pool.total_compressed_bytes.saturating_sub(4096);
            }
            SlotState::OnDisk { .. } => {
                pool.active_slots = pool.active_slots.saturating_sub(1);
            }
            SlotState::Empty => {}
        }
    }
}
pub fn stats() -> (usize, usize, usize) {
    let pool = ZRAM.lock();
    (
        pool.active_slots,
        pool.total_compressed_bytes,
        pool.total_saved_bytes,
    )
}
