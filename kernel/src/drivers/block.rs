use crate::memory::address::PhysAddr;
use crate::memory::pmm::{self, PAGE_SIZE};
use crate::sync::IrqSafeSpinlock;
use core::fmt;
use core::ptr::NonNull;
pub struct DmaBuffer {
    frame: usize,
    virtual_address: NonNull<u8>,
}
unsafe impl Send for DmaBuffer {}
impl DmaBuffer {
    pub fn allocate() -> Result<Self, BlockError> {
        let frame = pmm::alloc_contiguous_frames(1).ok_or(BlockError::OutOfMemory)?;
        let physical = PhysAddr::new((frame * PAGE_SIZE) as u64);
        let virtual_address = match crate::memory::address::hhdm_offset().map_phys(physical) {
            Ok(address) => address.get() as *mut u8,
            Err(_) => {
                pmm::free_contiguous_frames(frame, 1);
                return Err(BlockError::InvalidDmaAddress);
            }
        };
        let Some(virtual_address) = NonNull::new(virtual_address) else {
            pmm::free_contiguous_frames(frame, 1);
            return Err(BlockError::InvalidDmaAddress);
        };
        unsafe {
            core::ptr::write_bytes(virtual_address.as_ptr(), 0, PAGE_SIZE);
        }
        Ok(Self {
            frame,
            virtual_address,
        })
    }
    pub const fn physical_address(&self) -> PhysAddr {
        PhysAddr::new((self.frame * PAGE_SIZE) as u64)
    }
    pub fn as_slice(&self, length: usize) -> Result<&[u8], BlockError> {
        if length > PAGE_SIZE {
            return Err(BlockError::BufferTooSmall);
        }
        Ok(unsafe { core::slice::from_raw_parts(self.virtual_address.as_ptr(), length) })
    }
    pub fn as_mut_slice(&mut self, length: usize) -> Result<&mut [u8], BlockError> {
        if length > PAGE_SIZE {
            return Err(BlockError::BufferTooSmall);
        }
        Ok(unsafe { core::slice::from_raw_parts_mut(self.virtual_address.as_ptr(), length) })
    }
}
impl Drop for DmaBuffer {
    fn drop(&mut self) {
        pmm::free_contiguous_frames(self.frame, 1);
    }
}
pub struct DmaRegion {
    frame: usize,
    page_count: usize,
    capacity: usize,
    virtual_address: NonNull<u8>,
}
unsafe impl Send for DmaRegion {}
impl DmaRegion {
    pub fn allocate(length: usize) -> Result<Self, BlockError> {
        if length == 0 {
            return Err(BlockError::InvalidRange);
        }
        let page_count = length
            .checked_add(PAGE_SIZE - 1)
            .ok_or(BlockError::InvalidRange)?
            / PAGE_SIZE;
        let frame = pmm::alloc_contiguous_frames(page_count).ok_or(BlockError::OutOfMemory)?;
        let physical = PhysAddr::new((frame * PAGE_SIZE) as u64);
        let address = match crate::memory::address::hhdm_offset().map_phys(physical) {
            Ok(address) => address.get() as *mut u8,
            Err(_) => {
                pmm::free_contiguous_frames(frame, page_count);
                return Err(BlockError::InvalidDmaAddress);
            }
        };
        let Some(virtual_address) = NonNull::new(address) else {
            pmm::free_contiguous_frames(frame, page_count);
            return Err(BlockError::InvalidDmaAddress);
        };
        let capacity = page_count * PAGE_SIZE;
        unsafe {
            core::ptr::write_bytes(virtual_address.as_ptr(), 0, capacity);
        }
        Ok(Self {
            frame,
            page_count,
            capacity,
            virtual_address,
        })
    }
    pub const fn physical_address(&self) -> PhysAddr {
        PhysAddr::new((self.frame * PAGE_SIZE) as u64)
    }
    pub const fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn as_slice(&self, length: usize) -> Result<&[u8], BlockError> {
        if length > self.capacity {
            return Err(BlockError::BufferTooSmall);
        }
        Ok(unsafe { core::slice::from_raw_parts(self.virtual_address.as_ptr(), length) })
    }
    pub fn as_mut_slice(&mut self, length: usize) -> Result<&mut [u8], BlockError> {
        if length > self.capacity {
            return Err(BlockError::BufferTooSmall);
        }
        Ok(unsafe { core::slice::from_raw_parts_mut(self.virtual_address.as_ptr(), length) })
    }
}
impl Drop for DmaRegion {
    fn drop(&mut self) {
        pmm::free_contiguous_frames(self.frame, self.page_count);
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockGeometry {
    pub block_count: u64,
    pub block_size: u32,
}
impl BlockGeometry {
    pub fn capacity_bytes(self) -> Option<u64> {
        self.block_count.checked_mul(self.block_size as u64)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockError {
    NoDevice,
    InvalidGeometry,
    InvalidRange,
    BufferTooSmall,
    OutOfMemory,
    InvalidDmaAddress,
    Unsupported,
    DeviceFault,
    Timeout,
}
impl fmt::Display for BlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoDevice => "block device not present",
            Self::InvalidGeometry => "invalid block geometry",
            Self::InvalidRange => "block request outside device bounds",
            Self::BufferTooSmall => "DMA buffer is too small",
            Self::OutOfMemory => "unable to allocate DMA buffer",
            Self::InvalidDmaAddress => "invalid DMA address",
            Self::Unsupported => "block operation is unsupported",
            Self::DeviceFault => "block device reported an I/O error",
            Self::Timeout => "block device request timed out",
        })
    }
}
pub trait BlockDevice: Send + Sync {
    fn geometry(&self) -> Result<BlockGeometry, BlockError>;
    fn read_blocks(
        &self,
        start_block: u64,
        count: u16,
        buffer: &mut DmaBuffer,
    ) -> Result<(), BlockError>;
    fn write_blocks(
        &self,
        start_block: u64,
        count: u16,
        buffer: &DmaBuffer,
    ) -> Result<(), BlockError>;
    fn sync(&self) -> Result<(), BlockError> {
        Ok(())
    }
}
pub struct PartitionDevice {
    parent: &'static dyn BlockDevice,
    start_block: u64,
    block_count: u64,
}
impl PartitionDevice {
    pub fn new(
        parent: &'static dyn BlockDevice,
        start_block: u64,
        block_count: u64,
    ) -> Result<Self, BlockError> {
        let geometry = parent.geometry()?;
        let end = start_block
            .checked_add(block_count)
            .ok_or(BlockError::InvalidRange)?;
        if block_count == 0 || end > geometry.block_count {
            return Err(BlockError::InvalidRange);
        }
        Ok(Self {
            parent,
            start_block,
            block_count,
        })
    }
    pub const fn start_block(&self) -> u64 {
        self.start_block
    }
    pub const fn block_count(&self) -> u64 {
        self.block_count
    }
}
impl BlockDevice for PartitionDevice {
    fn geometry(&self) -> Result<BlockGeometry, BlockError> {
        let parent = self.parent.geometry()?;
        Ok(BlockGeometry {
            block_count: self.block_count,
            block_size: parent.block_size,
        })
    }
    fn read_blocks(
        &self,
        start: u64,
        count: u16,
        buffer: &mut DmaBuffer,
    ) -> Result<(), BlockError> {
        let end = start
            .checked_add(count as u64)
            .ok_or(BlockError::InvalidRange)?;
        if count == 0 || end > self.block_count {
            return Err(BlockError::InvalidRange);
        }
        self.parent
            .read_blocks(self.start_block + start, count, buffer)
    }
    fn write_blocks(&self, start: u64, count: u16, buffer: &DmaBuffer) -> Result<(), BlockError> {
        let end = start
            .checked_add(count as u64)
            .ok_or(BlockError::InvalidRange)?;
        if count == 0 || end > self.block_count {
            return Err(BlockError::InvalidRange);
        }
        self.parent
            .write_blocks(self.start_block + start, count, buffer)
    }
    fn sync(&self) -> Result<(), BlockError> {
        self.parent.sync()
    }
}
/// Un dispositivo de bloque expuesto a usuariospace, con un nombre estable para
/// mostrar en el instalador.
///
/// Los drivers registran su disco global en `init` mediante
/// [`register_block_device`]; `sys_block_open(index)` resuelve el índice a una
/// entrada concreta, de modo que lectura/escritura/info van al dispositivo
/// elegido y no a un global fijo.
#[derive(Clone, Copy)]
pub struct BlockDeviceEntry {
    pub name: &'static str,
    pub device: &'static dyn BlockDevice,
    pub read_only: bool,
}
pub const MAX_BLOCK_DEVICES: usize = 8;
struct BlockRegistry {
    count: usize,
    entries: [Option<BlockDeviceEntry>; MAX_BLOCK_DEVICES],
}
static REGISTRY: IrqSafeSpinlock<BlockRegistry> = IrqSafeSpinlock::new(BlockRegistry {
    count: 0,
    entries: [None; MAX_BLOCK_DEVICES],
});
/// Registra `device` bajo un nombre corto y devuelve su índice estable.
///
/// Es idempotente por nombre: si `name` ya está registrado devuelve su índice
/// existente sin duplicar la entrada (tolera reinicialización del driver y el
/// bucle de puertos de AHCI, que puede detectar varios discos).
pub fn register_block_device(
    name: &'static str,
    device: &'static dyn BlockDevice,
    read_only: bool,
) -> Result<usize, BlockError> {
    let mut registry = REGISTRY.lock();
    for (index, entry) in registry.entries.iter().enumerate() {
        if let Some(entry) = entry {
            if entry.name == name {
                return Ok(index);
            }
        }
    }
    if registry.count >= MAX_BLOCK_DEVICES {
        return Err(BlockError::OutOfMemory);
    }
    let index = registry.count;
    registry.entries[index] = Some(BlockDeviceEntry {
        name,
        device,
        read_only,
    });
    registry.count += 1;
    Ok(index)
}
/// Resuelve el índice de dispositivo a su entrada, o `None` si no existe.
///
/// Devuelve una copia de la entrada, de modo que el lock del registro se suelta
/// antes de la I/O de disco real.
pub fn block_device(index: usize) -> Option<BlockDeviceEntry> {
    let registry = REGISTRY.lock();
    if index < registry.count {
        registry.entries[index]
    } else {
        None
    }
}
/// Maximum number of biological units merged in one block-level request.
pub const BLOCK_MERGE_MAX_BIOS: usize = 16;
/// Clasificación del tipo de I/O para scheduling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockIoClass {
    Sync,
    ReadAhead,
    WriteBack,
    Journal,
}
/// Una petición fusionada a nivel de bloque.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergedBlockRequest {
    pub start_lba: u64,
    pub block_count: u16,
    pub class: BlockIoClass,
}
/// Decide si dos requests son fusionables en un único segmento.
pub fn can_merge(a: &MergedBlockRequest, b: &MergedBlockRequest) -> bool {
    !(a.start_lba + u64::from(a.block_count) != b.start_lba || a.class != b.class)
}
/// Dados requests **ordenados por start_lba**, fusiona segmentos contiguos.
pub fn merge_block_requests(requests: &[MergedBlockRequest]) -> alloc::vec::Vec<MergedBlockRequest> {
    if requests.is_empty() {
        return alloc::vec::Vec::new();
    }
    let mut out = alloc::vec::Vec::with_capacity(requests.len().min(BLOCK_MERGE_MAX_BIOS));
    out.push(requests[0]);
    for next in requests.iter().skip(1) {
        let last_idx = out.len() - 1;
        let last = out[last_idx];
        if last.class == next.class && last.start_lba + u64::from(last.block_count) == next.start_lba {
            if (last.block_count as usize).saturating_add(next.block_count as usize) <= 4096 {
                out[last_idx].block_count = last.block_count.saturating_add(next.block_count);
                continue;
            }
        }
        if out.len() < BLOCK_MERGE_MAX_BIOS {
            out.push(*next);
        }
    }
    out
}
#[cfg(test)]
mod block_merge_tests {
    use super::*;
    #[test]
    fn merges_contiguous_same_class() {
        let reqs = [
            MergedBlockRequest { start_lba: 0, block_count: 2, class: BlockIoClass::Sync },
            MergedBlockRequest { start_lba: 2, block_count: 2, class: BlockIoClass::Sync },
        ];
        let r = merge_block_requests(&reqs);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].block_count, 4);
    }
    #[test]
    fn no_merge_different_class() {
        let reqs = [
            MergedBlockRequest { start_lba: 0, block_count: 2, class: BlockIoClass::Sync },
            MergedBlockRequest { start_lba: 2, block_count: 2, class: BlockIoClass::ReadAhead },
        ];
        let r = merge_block_requests(&reqs);
        assert_eq!(r.len(), 2);
    }
    #[test]
    fn can_merge_direct() {
        let a = MergedBlockRequest { start_lba: 0, block_count: 2, class: BlockIoClass::Sync };
        let b = MergedBlockRequest { start_lba: 2, block_count: 2, class: BlockIoClass::Sync };
        assert!(can_merge(&a, &b));
    }
}
/// Buffer estándar de requests pendientes por dispositivo con fusión.
#[derive(Default)]
pub struct BlockRequestQueue {
    requests: alloc::vec::Vec<MergedBlockRequest>,
}
impl BlockRequestQueue {
    pub const fn new() -> Self {
        Self { requests: alloc::vec::Vec::new() }
    }
    pub fn push(&mut self, req: MergedBlockRequest) {
        if self.requests.len() < BLOCK_MERGE_MAX_BIOS * 4 {
            self.requests.push(req);
        }
    }
    pub fn flush(&mut self) -> alloc::vec::Vec<MergedBlockRequest> {
        self.requests.sort_by_key(|r| r.start_lba);
        let merged = merge_block_requests(&self.requests);
        self.requests.clear();
        merged
    }
    pub fn is_empty(&self) -> bool { self.requests.is_empty() }
    pub fn len(&self) -> usize {
        self.requests.len()
    }
}
#[cfg(test)]
mod block_queue_tests {
    use super::*;
    #[test]
    fn queue_flush_merges() {
        let mut q = BlockRequestQueue::new();
        q.push(MergedBlockRequest { start_lba: 0, block_count: 2, class: BlockIoClass::Sync });
        q.push(MergedBlockRequest { start_lba: 2, block_count: 2, class: BlockIoClass::Sync });
        let merged = q.flush();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].block_count, 4);
        assert_eq!(q.len(), 0);
    }
}
pub const PAGE_CACHE_DEFAULT_CAPACITY: usize = 256;
pub struct BlockPageCache {
    capacity: usize,
    pages: alloc::collections::BTreeMap<u64, alloc::vec::Vec<u8>>,
    lru: alloc::collections::VecDeque<u64>,
    hits: u64,
    misses: u64,
}
impl Default for BlockPageCache {
    fn default() -> Self { Self::new() }
}
impl BlockPageCache {
    pub fn new() -> Self { Self::with_capacity(PAGE_CACHE_DEFAULT_CAPACITY) }
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            pages: alloc::collections::BTreeMap::new(),
            lru: alloc::collections::VecDeque::new(),
            hits: 0,
            misses: 0,
        }
    }
    pub fn get(&mut self, lba: u64) -> Option<&[u8]> {
        if self.pages.contains_key(&lba) {
            if let Some(pos) = self.lru.iter().position(|&x| x == lba) {
                let item = self.lru.remove(pos).unwrap();
                self.lru.push_back(item);
            }
            self.hits += 1;
            self.pages.get(&lba).map(|v| v.as_slice())
        } else {
            self.misses += 1;
            None
        }
    }
    pub fn insert(&mut self, lba: u64, data: alloc::vec::Vec<u8>) {
        while self.pages.len() >= self.capacity && !self.pages.contains_key(&lba) {
            if let Some(victim) = self.lru.pop_front() {
                self.pages.remove(&victim);
            } else { break; }
        }
        if !self.pages.contains_key(&lba) {
            self.lru.push_back(lba);
        }
        self.pages.insert(lba, data);
    }
    pub fn readahead_lbas(&self, start: u64, depth: usize) -> alloc::vec::Vec<u64> {
        (0..depth).map(|i| start + i as u64).collect()
    }
    pub fn clear(&mut self) {
        self.pages.clear();
        self.lru.clear();
        self.hits = 0;
        self.misses = 0;
    }
    pub fn is_empty(&self) -> bool { self.pages.is_empty() }
    pub fn len(&self) -> usize { self.pages.len() }
    pub fn capacity(&self) -> usize { self.capacity }
    pub fn stats(&self) -> (u64, u64) { (self.hits, self.misses) }
}
#[cfg(test)]
mod block_page_cache_tests {
    use super::*;
    #[test]
    fn insert_and_get() {
        let mut c = BlockPageCache::new();
        let d = alloc::vec![1u8, 2, 3, 4];
        c.insert(5, d.clone());
        assert_eq!(c.get(5), Some(d.as_slice()));
    }
    #[test]
    fn lru_evicts() {
        let mut c = BlockPageCache::with_capacity(2);
        c.insert(1, alloc::vec![1]);
        c.insert(2, alloc::vec![2]);
        c.insert(3, alloc::vec![3]);
        assert!(c.get(1).is_none());
        assert!(c.get(2).is_some());
        assert!(c.get(3).is_some());
    }
    #[test]
    fn touch_promotes() {
        let mut c = BlockPageCache::with_capacity(2);
        c.insert(1, alloc::vec![1]);
        c.insert(2, alloc::vec![2]);
        assert!(c.get(1).is_some());
        c.insert(3, alloc::vec![3]);
        assert!(c.get(1).is_some());
        assert!(c.get(2).is_none());
        assert!(c.get(3).is_some());
    }
    #[test]
    fn readahead_shape() {
        let c = BlockPageCache::new();
        let ahead = c.readahead_lbas(10, 4);
        assert_eq!(ahead, alloc::vec![10u64,11,12,13]);
    }
}
/// Cola global de fusión (placeholder wire-in).
pub static BLOCK_QUEUE: IrqSafeSpinlock<BlockRequestQueue> = IrqSafeSpinlock::new(BlockRequestQueue::new());
/// Combina rangos (lba,count) contiguos en I/O merging.
pub fn coalesce_ranges(ranges: &[(u64, u16)]) -> alloc::vec::Vec<(u64, u16)> {
    if ranges.is_empty() { return alloc::vec::Vec::new(); }
    let mut sorted: alloc::vec::Vec<(u64, u16)> = ranges.to_vec();
    sorted.sort_by_key(|r| r.0);
    let mut out: alloc::vec::Vec<(u64, u16)> = alloc::vec::Vec::new();
    for item in sorted {
        let last = out.last_mut();
        if let Some((ls, lc)) = last {
            let last_end = *ls + *lc as u64;
            if last_end == item.0 {
                let total = *lc as usize + item.1 as usize;
                if total <= 4096 {
                    *lc = lc.saturating_add(item.1);
                    continue;
                }
            }
        }
        out.push(item);
    }
    out
}
#[cfg(test)]
mod coalesce_ranges_tests {
    use super::*;
    #[test]
    fn contiguous_merge() {
        let r = coalesce_ranges(&[(0, 2), (2, 3), (6, 1)]);
        assert_eq!(r, alloc::vec![(0, 5), (6, 1)]);
    }
    #[test]
    fn ordered() {
        let r = coalesce_ranges(&[(4, 2), (0, 2), (2, 1)]);
        assert_eq!(r, alloc::vec![(0, 3), (4, 2)]);
    }
    #[test]
    fn empty() {
        assert_eq!(coalesce_ranges(&[]), alloc::vec::Vec::new());
    }
}
