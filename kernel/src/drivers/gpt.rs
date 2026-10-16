use crate::drivers::block::{BlockDevice, BlockError, DmaBuffer, PartitionDevice};
use crate::memory::pmm::PAGE_SIZE;
use alloc::boxed::Box;
use alloc::vec::Vec;
use lix_storage::{parse_gpt_header, parse_gpt_partitions, GptError};
const LINUX_FILESYSTEM_GUID: [u8; 16] = [
    0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47, 0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4,
];
fn block_error(error: GptError) -> BlockError {
    match error {
        GptError::OutOfMemory => BlockError::OutOfMemory,
        GptError::HeaderChecksum | GptError::EntryChecksum => BlockError::DeviceFault,
        _ => BlockError::InvalidGeometry,
    }
}
fn validate_table_range(
    header: lix_storage::GptHeader,
    block_size: u64,
    disk_blocks: u64,
) -> Result<(), BlockError> {
    let table_blocks = (header.entry_count as u64)
        .checked_mul(header.entry_size as u64)
        .ok_or(BlockError::InvalidGeometry)?
        .div_ceil(block_size);
    let end = header
        .entries_lba
        .checked_add(table_blocks)
        .ok_or(BlockError::InvalidGeometry)?;
    let valid_location = if header.current_lba == 1 {
        header.entries_lba >= 2 && end <= header.first_usable_lba
    } else {
        header.entries_lba > header.last_usable_lba && end <= header.current_lba
    };
    if !valid_location || end > disk_blocks {
        return Err(BlockError::InvalidGeometry);
    }
    Ok(())
}
fn read_entry_array(
    device: &dyn BlockDevice,
    header: lix_storage::GptHeader,
    block_size: usize,
    dma: &mut DmaBuffer,
) -> Result<Vec<u8>, BlockError> {
    let table_bytes = (header.entry_count as usize)
        .checked_mul(header.entry_size as usize)
        .ok_or(BlockError::InvalidGeometry)?;
    let table_blocks = (table_bytes as u64).div_ceil(block_size as u64);
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(table_bytes)
        .map_err(|_| BlockError::OutOfMemory)?;
    for block in 0..table_blocks {
        let lba = header
            .entries_lba
            .checked_add(block)
            .ok_or(BlockError::InvalidGeometry)?;
        device.read_blocks(lba, 1, dma)?;
        let amount = (table_bytes - entries.len()).min(block_size);
        entries.extend_from_slice(&dma.as_slice(amount)?[..amount]);
    }
    Ok(entries)
}
pub fn find_linux_root(
    parent: &'static dyn BlockDevice,
    requested_guid: Option<[u8; 16]>,
) -> Result<&'static dyn BlockDevice, BlockError> {
    let geometry = parent.geometry()?;
    let block_size = geometry.block_size as usize;
    if block_size < 512 || block_size > PAGE_SIZE || geometry.block_count < 3 {
        return Err(BlockError::InvalidGeometry);
    }
    let mut dma = DmaBuffer::allocate()?;
    parent.read_blocks(1, 1, &mut dma)?;
    let primary = parse_gpt_header(dma.as_slice(block_size)?, geometry.block_count, 1)
        .map_err(block_error)?;
    validate_table_range(primary, block_size as u64, geometry.block_count)?;
    parent.read_blocks(primary.backup_lba, 1, &mut dma)?;
    let backup = parse_gpt_header(
        dma.as_slice(block_size)?,
        geometry.block_count,
        primary.backup_lba,
    )
    .map_err(block_error)?;
    validate_table_range(backup, block_size as u64, geometry.block_count)?;
    if !lix_storage::headers_match(primary, backup, geometry.block_count) {
        return Err(BlockError::InvalidGeometry);
    }
    let entries = read_entry_array(parent, primary, block_size, &mut dma)?;
    let backup_entries = read_entry_array(parent, backup, block_size, &mut dma)?;
    if entries != backup_entries {
        return Err(BlockError::DeviceFault);
    }
    let partitions = parse_gpt_partitions(primary, &entries).map_err(block_error)?;
    parse_gpt_partitions(backup, &backup_entries).map_err(block_error)?;
    let mut found = None;
    for partition in partitions
        .iter()
        .filter(|p| p.type_guid == LINUX_FILESYSTEM_GUID)
    {
        if requested_guid.is_some_and(|guid| partition.unique_guid != guid) {
            continue;
        }
        if found
            .replace((
                partition.first_lba,
                partition.block_count().ok_or(BlockError::InvalidGeometry)?,
            ))
            .is_some()
        {
            return Err(BlockError::Unsupported);
        }
    }
    let (first, count) = found.ok_or(BlockError::NoDevice)?;
    Ok(Box::leak(Box::new(PartitionDevice::new(
        parent, first, count,
    )?)))
}
pub fn parse_guid(text: &str) -> Option<[u8; 16]> {
    lix_storage::parse_guid(text)
}
