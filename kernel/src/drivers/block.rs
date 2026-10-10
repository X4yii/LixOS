use crate::memory::address::PhysAddr;
use crate::memory::pmm::{self, PAGE_SIZE};
use core::fmt;
use core::ptr::NonNull;

/// A single physically-contiguous page suitable for simple DMA transfers.
/// Larger I/O is split into page-sized requests by the caller.
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
        unsafe { core::ptr::write_bytes(virtual_address.as_ptr(), 0, PAGE_SIZE); }
        Ok(Self { frame, virtual_address })
    }

    pub const fn physical_address(&self) -> PhysAddr {
        PhysAddr::new((self.frame * PAGE_SIZE) as u64)
    }

    pub fn as_slice(&self, length: usize) -> Result<&[u8], BlockError> {
        if length > PAGE_SIZE { return Err(BlockError::BufferTooSmall); }
        Ok(unsafe { core::slice::from_raw_parts(self.virtual_address.as_ptr(), length) })
    }

    pub fn as_mut_slice(&mut self, length: usize) -> Result<&mut [u8], BlockError> {
        if length > PAGE_SIZE { return Err(BlockError::BufferTooSmall); }
        Ok(unsafe { core::slice::from_raw_parts_mut(self.virtual_address.as_ptr(), length) })
    }
}

impl Drop for DmaBuffer {
    fn drop(&mut self) {
        pmm::free_contiguous_frames(self.frame, 1);
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

/// Bounded, read/write view of a device. Range checks happen before the child
/// driver sees an LBA, so callers cannot spill into neighbouring partitions.
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
        let end = start_block.checked_add(block_count).ok_or(BlockError::InvalidRange)?;
        if block_count == 0 || end > geometry.block_count {
            return Err(BlockError::InvalidRange);
        }
        Ok(Self { parent, start_block, block_count })
    }

    pub const fn start_block(&self) -> u64 { self.start_block }
    pub const fn block_count(&self) -> u64 { self.block_count }
}

impl BlockDevice for PartitionDevice {
    fn geometry(&self) -> Result<BlockGeometry, BlockError> {
        let parent = self.parent.geometry()?;
        Ok(BlockGeometry { block_count: self.block_count, block_size: parent.block_size })
    }

    fn read_blocks(&self, start: u64, count: u16, buffer: &mut DmaBuffer) -> Result<(), BlockError> {
        let end = start.checked_add(count as u64).ok_or(BlockError::InvalidRange)?;
        if count == 0 || end > self.block_count { return Err(BlockError::InvalidRange); }
        self.parent.read_blocks(self.start_block + start, count, buffer)
    }

    fn write_blocks(&self, start: u64, count: u16, buffer: &DmaBuffer) -> Result<(), BlockError> {
        let end = start.checked_add(count as u64).ok_or(BlockError::InvalidRange)?;
        if count == 0 || end > self.block_count { return Err(BlockError::InvalidRange); }
        self.parent.write_blocks(self.start_block + start, count, buffer)
    }

    fn sync(&self) -> Result<(), BlockError> { self.parent.sync() }
}
