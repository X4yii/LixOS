#![no_std]
extern crate alloc;

use alloc::vec::Vec;

pub const GPT_HEADER_MIN_SIZE: usize = 92;
pub const GPT_MAX_ENTRY_TABLE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GptError {
    InvalidGeometry,
    InvalidHeader,
    HeaderChecksum,
    EntryChecksum,
    InvalidEntry,
    OverlappingPartitions,
    OutOfMemory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GptHeader {
    pub current_lba: u64,
    pub backup_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub disk_guid: [u8; 16],
    pub entries_lba: u64,
    pub entry_count: u32,
    pub entry_size: u32,
    pub entries_crc32: u32,
    pub header_size: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GptPartition {
    pub type_guid: [u8; 16],
    pub unique_guid: [u8; 16],
    pub first_lba: u64,
    pub last_lba: u64,
    pub attributes: u64,
}

#[derive(Clone, Debug)]
pub struct GptImage {
    pub protective_mbr: [u8; 512],
    pub primary_header: Vec<u8>,
    pub primary_entries: Vec<u8>,
    pub backup_entries: Vec<u8>,
    pub backup_header: Vec<u8>,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub primary_entries_lba: u64,
    pub backup_entries_lba: u64,
}

impl GptPartition {
    pub fn block_count(self) -> Option<u64> {
        self.last_lba.checked_sub(self.first_lba)?.checked_add(1)
    }
}

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn u32le(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn u64le(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
    ))
}
fn guid(bytes: &[u8], offset: usize) -> Option<[u8; 16]> {
    Some(
        bytes
            .get(offset..offset.checked_add(16)?)?
            .try_into()
            .ok()?,
    )
}

/// Validates a GPT header against its physical LBA and disk geometry.
pub fn parse_gpt_header(
    block: &[u8],
    disk_blocks: u64,
    expected_lba: u64,
) -> Result<GptHeader, GptError> {
    if block.len() < GPT_HEADER_MIN_SIZE
        || expected_lba >= disk_blocks
        || block.get(..8) != Some(b"EFI PART")
    {
        return Err(GptError::InvalidHeader);
    }
    let header_size = u32le(block, 12).ok_or(GptError::InvalidHeader)?;
    if (header_size as usize) < GPT_HEADER_MIN_SIZE || (header_size as usize) > block.len() {
        return Err(GptError::InvalidHeader);
    }
    let expected_crc = u32le(block, 16).ok_or(GptError::InvalidHeader)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(header_size as usize)
        .map_err(|_| GptError::OutOfMemory)?;
    bytes.extend_from_slice(&block[..header_size as usize]);
    bytes[16..20].fill(0);
    if crc32(&bytes) != expected_crc {
        return Err(GptError::HeaderChecksum);
    }

    let header = GptHeader {
        current_lba: u64le(block, 24).ok_or(GptError::InvalidHeader)?,
        backup_lba: u64le(block, 32).ok_or(GptError::InvalidHeader)?,
        first_usable_lba: u64le(block, 40).ok_or(GptError::InvalidHeader)?,
        last_usable_lba: u64le(block, 48).ok_or(GptError::InvalidHeader)?,
        disk_guid: guid(block, 56).ok_or(GptError::InvalidHeader)?,
        entries_lba: u64le(block, 72).ok_or(GptError::InvalidHeader)?,
        entry_count: u32le(block, 80).ok_or(GptError::InvalidHeader)?,
        entry_size: u32le(block, 84).ok_or(GptError::InvalidHeader)?,
        entries_crc32: u32le(block, 88).ok_or(GptError::InvalidHeader)?,
        header_size,
    };
    if header.disk_guid.iter().all(|byte| *byte == 0) {
        return Err(GptError::InvalidHeader);
    }
    let table_bytes = (header.entry_count as u64)
        .checked_mul(header.entry_size as u64)
        .ok_or(GptError::InvalidGeometry)?;
    let table_blocks = table_bytes.div_ceil(block.len() as u64);
    let table_end = header
        .entries_lba
        .checked_add(table_blocks)
        .ok_or(GptError::InvalidGeometry)?;
    if header.current_lba != expected_lba
        || header.backup_lba >= disk_blocks
        || header.backup_lba == header.current_lba
        || header.first_usable_lba > header.last_usable_lba
        || header.last_usable_lba >= disk_blocks
        || header.entry_count == 0
        || header.entry_size < 128
        || header.entry_size % 8 != 0
        || header.entry_size as usize > block.len()
        || block.len() % header.entry_size as usize != 0
        || table_bytes > GPT_MAX_ENTRY_TABLE_BYTES
        || header.entries_lba < 2
        || table_end > disk_blocks
    {
        return Err(GptError::InvalidGeometry);
    }
    Ok(header)
}

pub fn headers_match(primary: GptHeader, backup: GptHeader, disk_blocks: u64) -> bool {
    disk_blocks >= 3
        && primary.current_lba == 1
        && primary.backup_lba == disk_blocks - 1
        && backup.current_lba == disk_blocks - 1
        && backup.backup_lba == 1
        && primary.first_usable_lba == backup.first_usable_lba
        && primary.last_usable_lba == backup.last_usable_lba
        && primary.disk_guid == backup.disk_guid
        && primary.entry_count == backup.entry_count
        && primary.entry_size == backup.entry_size
        && primary.entries_crc32 == backup.entries_crc32
}

/// Parses and validates all used GPT entries, rejecting overlaps and bad bounds.
pub fn parse_gpt_partitions(
    header: GptHeader,
    entries: &[u8],
) -> Result<Vec<GptPartition>, GptError> {
    let expected = (header.entry_count as usize)
        .checked_mul(header.entry_size as usize)
        .ok_or(GptError::InvalidGeometry)?;
    if entries.len() != expected || crc32(entries) != header.entries_crc32 {
        return Err(GptError::EntryChecksum);
    }
    let mut partitions = Vec::new();
    let mut ranges = Vec::new();
    let mut unique_guids = Vec::new();
    partitions
        .try_reserve(header.entry_count as usize)
        .map_err(|_| GptError::OutOfMemory)?;
    ranges
        .try_reserve(header.entry_count as usize)
        .map_err(|_| GptError::OutOfMemory)?;
    unique_guids
        .try_reserve(header.entry_count as usize)
        .map_err(|_| GptError::OutOfMemory)?;
    for entry in entries.chunks_exact(header.entry_size as usize) {
        let type_guid = guid(entry, 0).ok_or(GptError::InvalidEntry)?;
        if type_guid.iter().all(|byte| *byte == 0) {
            continue;
        }
        let partition = GptPartition {
            type_guid,
            unique_guid: guid(entry, 16).ok_or(GptError::InvalidEntry)?,
            first_lba: u64le(entry, 32).ok_or(GptError::InvalidEntry)?,
            last_lba: u64le(entry, 40).ok_or(GptError::InvalidEntry)?,
            attributes: u64le(entry, 48).ok_or(GptError::InvalidEntry)?,
        };
        if partition.unique_guid.iter().all(|byte| *byte == 0)
            || partition.first_lba < header.first_usable_lba
            || partition.last_lba > header.last_usable_lba
            || partition.first_lba > partition.last_lba
        {
            return Err(GptError::InvalidEntry);
        }
        ranges.push((partition.first_lba, partition.last_lba));
        unique_guids.push(partition.unique_guid);
        partitions.push(partition);
    }
    ranges.sort_unstable_by_key(|range| range.0);
    if ranges.windows(2).any(|pair| pair[0].1 >= pair[1].0) {
        return Err(GptError::OverlappingPartitions);
    }
    unique_guids.sort_unstable();
    if unique_guids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(GptError::InvalidEntry);
    }
    Ok(partitions)
}

/// Serializes a standard 128-entry GPT image layout without performing I/O.
/// The caller owns persistence, target selection and write-order journaling.
pub fn build_gpt_image(
    block_size: u32,
    disk_blocks: u64,
    disk_guid: [u8; 16],
    partitions: &[GptPartition],
) -> Result<GptImage, GptError> {
    let block_size = block_size as usize;
    if block_size < 512
        || block_size > 4096
        || !block_size.is_power_of_two()
        || disk_blocks < 3
        || disk_guid.iter().all(|byte| *byte == 0)
        || partitions.len() > 128
    {
        return Err(GptError::InvalidGeometry);
    }
    const ENTRY_COUNT: u64 = 128;
    const ENTRY_SIZE: u64 = 128;
    let entries_len = (ENTRY_COUNT * ENTRY_SIZE) as usize;
    let entries_blocks = (entries_len as u64).div_ceil(block_size as u64);
    let first_usable_lba = 2u64
        .checked_add(entries_blocks)
        .ok_or(GptError::InvalidGeometry)?;
    let backup_entries_lba = disk_blocks
        .checked_sub(entries_blocks + 1)
        .ok_or(GptError::InvalidGeometry)?;
    let last_usable_lba = backup_entries_lba
        .checked_sub(1)
        .ok_or(GptError::InvalidGeometry)?;
    if last_usable_lba < first_usable_lba {
        return Err(GptError::InvalidGeometry);
    }

    let mut sorted = Vec::new();
    sorted
        .try_reserve_exact(partitions.len())
        .map_err(|_| GptError::OutOfMemory)?;
    sorted.extend_from_slice(partitions);
    for partition in &sorted {
        if partition.type_guid.iter().all(|byte| *byte == 0)
            || partition.unique_guid.iter().all(|byte| *byte == 0)
            || partition.first_lba < first_usable_lba
            || partition.last_lba > last_usable_lba
            || partition.first_lba > partition.last_lba
        {
            return Err(GptError::InvalidEntry);
        }
    }
    sorted.sort_unstable_by_key(|partition| partition.first_lba);
    if sorted
        .windows(2)
        .any(|pair| pair[0].last_lba >= pair[1].first_lba)
    {
        return Err(GptError::OverlappingPartitions);
    }
    let mut guids = sorted.iter().map(|p| p.unique_guid).collect::<Vec<_>>();
    guids.sort_unstable();
    if guids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(GptError::InvalidEntry);
    }

    let mut entries = Vec::new();
    entries
        .try_reserve_exact(entries_len)
        .map_err(|_| GptError::OutOfMemory)?;
    entries.resize(entries_len, 0);
    for (index, partition) in partitions.iter().enumerate() {
        let at = index * ENTRY_SIZE as usize;
        entries[at..at + 16].copy_from_slice(&partition.type_guid);
        entries[at + 16..at + 32].copy_from_slice(&partition.unique_guid);
        entries[at + 32..at + 40].copy_from_slice(&partition.first_lba.to_le_bytes());
        entries[at + 40..at + 48].copy_from_slice(&partition.last_lba.to_le_bytes());
        entries[at + 48..at + 56].copy_from_slice(&partition.attributes.to_le_bytes());
    }
    let entries_crc32 = crc32(&entries);
    let make_header = |current_lba: u64, backup_lba: u64, entries_lba: u64| {
        let mut header = alloc::vec![0; block_size];
        header[..8].copy_from_slice(b"EFI PART");
        header[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        header[12..16].copy_from_slice(&(GPT_HEADER_MIN_SIZE as u32).to_le_bytes());
        header[24..32].copy_from_slice(&current_lba.to_le_bytes());
        header[32..40].copy_from_slice(&backup_lba.to_le_bytes());
        header[40..48].copy_from_slice(&first_usable_lba.to_le_bytes());
        header[48..56].copy_from_slice(&last_usable_lba.to_le_bytes());
        header[56..72].copy_from_slice(&disk_guid);
        header[72..80].copy_from_slice(&entries_lba.to_le_bytes());
        header[80..84].copy_from_slice(&(ENTRY_COUNT as u32).to_le_bytes());
        header[84..88].copy_from_slice(&(ENTRY_SIZE as u32).to_le_bytes());
        header[88..92].copy_from_slice(&entries_crc32.to_le_bytes());
        let header_crc = crc32(&header[..GPT_HEADER_MIN_SIZE]);
        header[16..20].copy_from_slice(&header_crc.to_le_bytes());
        header
    };
    let primary_header = make_header(1, disk_blocks - 1, 2);
    let backup_header = make_header(disk_blocks - 1, 1, backup_entries_lba);
    let mut protective_mbr = [0u8; 512];
    protective_mbr[446 + 4] = 0xEE;
    protective_mbr[446 + 8..446 + 12].copy_from_slice(&1u32.to_le_bytes());
    let disk_sectors_512 = disk_blocks
        .checked_mul(block_size as u64 / 512)
        .ok_or(GptError::InvalidGeometry)?;
    let protective_size = disk_sectors_512.saturating_sub(1).min(u32::MAX as u64) as u32;
    protective_mbr[446 + 12..446 + 16].copy_from_slice(&protective_size.to_le_bytes());
    protective_mbr[510..512].copy_from_slice(&[0x55, 0xAA]);
    Ok(GptImage {
        protective_mbr,
        primary_header,
        primary_entries: entries.clone(),
        backup_entries: entries,
        backup_header,
        first_usable_lba,
        last_usable_lba,
        primary_entries_lba: 2,
        backup_entries_lba,
    })
}

pub fn parse_guid(text: &str) -> Option<[u8; 16]> {
    let bytes = text.as_bytes();
    if bytes.len() != 36 || [8, 13, 18, 23].iter().any(|&i| bytes[i] != b'-') {
        return None;
    }
    let mut canonical = [0u8; 16];
    let mut out = 0;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'-' {
            at += 1;
            continue;
        }
        let hi = (bytes[at] as char).to_digit(16)? as u8;
        let lo = (bytes[at + 1] as char).to_digit(16)? as u8;
        canonical[out] = (hi << 4) | lo;
        out += 1;
        at += 2;
    }
    if out != 16 {
        return None;
    }
    Some([
        canonical[3],
        canonical[2],
        canonical[1],
        canonical[0],
        canonical[5],
        canonical[4],
        canonical[7],
        canonical[6],
        canonical[8],
        canonical[9],
        canonical[10],
        canonical[11],
        canonical[12],
        canonical[13],
        canonical[14],
        canonical[15],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn guid_text_is_converted_to_gpt_byte_order() {
        assert_eq!(
            parse_guid("0FC63DAF-8483-4772-8E79-3D69D8477DE4"),
            Some([
                0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47, 0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47,
                0x7D, 0xE4
            ])
        );
        assert!(parse_guid("0FC63DAF---8483-4772-8E79-3D69D8477DE4").is_none());
    }

    #[test]
    fn partition_parser_rejects_overlapping_ranges() {
        let mut entries = [0u8; 256];
        for index in 0..2 {
            let at = index * 128;
            entries[at] = 1;
            entries[at + 16] = index as u8 + 1;
            entries[at + 32..at + 40].copy_from_slice(&(100u64 + index as u64 * 50).to_le_bytes());
            entries[at + 40..at + 48].copy_from_slice(&(200u64 + index as u64 * 50).to_le_bytes());
        }
        let header = GptHeader {
            current_lba: 1,
            backup_lba: 4095,
            first_usable_lba: 34,
            last_usable_lba: 4000,
            disk_guid: [1; 16],
            entries_lba: 2,
            entry_count: 2,
            entry_size: 128,
            entries_crc32: crc32(&entries),
            header_size: 92,
        };
        assert_eq!(
            parse_gpt_partitions(header, &entries),
            Err(GptError::OverlappingPartitions)
        );
    }

    #[test]
    fn generated_layout_round_trips_through_parser() {
        let partition = GptPartition {
            type_guid: [0xAA; 16],
            unique_guid: [0xBB; 16],
            first_lba: 2048,
            last_lba: 4095,
            attributes: 0,
        };
        let image = build_gpt_image(512, 16_384, [0xCC; 16], &[partition]).unwrap();
        let primary = parse_gpt_header(&image.primary_header, 16_384, 1).unwrap();
        let backup = parse_gpt_header(&image.backup_header, 16_384, 16_383).unwrap();
        assert!(headers_match(primary, backup, 16_384));
        assert_eq!(
            parse_gpt_partitions(primary, &image.primary_entries).unwrap(),
            [partition]
        );
        assert_eq!(image.backup_entries, image.primary_entries);
        assert_eq!(&image.protective_mbr[510..512], &[0x55, 0xAA]);
    }
}
