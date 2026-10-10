use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use lix_storage::{headers_match, parse_gpt_header, parse_gpt_partitions, GptHeader, GptPartition};

#[derive(Clone, Debug)]
pub struct DiskSnapshot {
    pub block_size: u32,
    pub block_count: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub disk_guid: [u8; 16],
    pub partitions: Vec<GptPartition>,
}

fn read_exact_at(file: &mut File, offset: u64, out: &mut [u8]) -> Result<(), String> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    file.read_exact(out).map_err(|e| e.to_string())
}

fn checked_header_table(
    header: GptHeader,
    block_size: u64,
    disk_blocks: u64,
) -> Result<(), String> {
    let table_blocks = (header.entry_count as u64)
        .checked_mul(header.entry_size as u64)
        .ok_or("GPT table length overflow")?
        .div_ceil(block_size);
    let end = header
        .entries_lba
        .checked_add(table_blocks)
        .ok_or("GPT table range overflow")?;
    if header.current_lba == 1 {
        if header.entries_lba < 2 || end > header.first_usable_lba {
            return Err("primary GPT entry array overlaps usable LBAs".into());
        }
    } else if header.entries_lba <= header.last_usable_lba || end > header.current_lba {
        return Err("backup GPT entry array overlaps usable LBAs or its header".into());
    }
    if end > disk_blocks {
        return Err("GPT entry array exceeds image bounds".into());
    }
    Ok(())
}

/// Read-only parser for regular raw disk-image files. Device nodes are deliberately refused.
pub fn inspect_image(path: &Path, block_size: u32) -> Result<DiskSnapshot, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() {
        return Err("only a regular disk-image file can be inspected".into());
    }
    if block_size < 512 || block_size > 4096 || !block_size.is_power_of_two() {
        return Err("logical block size must be a power of two from 512 through 4096".into());
    }
    let block_size_u64 = block_size as u64;
    let image_size = metadata.len();
    if image_size == 0 || image_size % block_size_u64 != 0 {
        return Err("image size is not block aligned".into());
    }
    let block_count = image_size / block_size_u64;
    if block_count < 3 {
        return Err("image is too small for GPT".into());
    }

    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut mbr = [0; 512];
    read_exact_at(&mut file, 0, &mut mbr)?;
    let mbr_entries = &mbr[446..510];
    let protective_start = u32::from_le_bytes(mbr[454..458].try_into().unwrap());
    let protective_size = u32::from_le_bytes(mbr[458..462].try_into().unwrap());
    let expected_protective_size = (image_size / 512 - 1).min(u32::MAX as u64) as u32;
    let has_hybrid_entries = mbr_entries[16..]
        .chunks_exact(16)
        .any(|entry| entry[4] != 0);
    if mbr[510..512] != [0x55, 0xAA]
        || mbr_entries[4] != 0xEE
        || protective_start != 1
        || protective_size != expected_protective_size
        || has_hybrid_entries
    {
        return Err("protective MBR signature/partition missing".into());
    }

    let mut block = vec![0; block_size as usize];
    read_exact_at(&mut file, block_size_u64, &mut block)?;
    let primary = parse_gpt_header(&block, block_count, 1)
        .map_err(|e| format!("primary GPT header: {e:?}"))?;
    checked_header_table(primary, block_size_u64, block_count)?;

    read_exact_at(&mut file, primary.backup_lba * block_size_u64, &mut block)?;
    let backup = parse_gpt_header(&block, block_count, primary.backup_lba)
        .map_err(|e| format!("backup GPT header: {e:?}"))?;
    checked_header_table(backup, block_size_u64, block_count)?;
    if !headers_match(primary, backup, block_count) {
        return Err("primary and backup GPT headers disagree".into());
    }

    let table_bytes = (primary.entry_count as usize)
        .checked_mul(primary.entry_size as usize)
        .ok_or("GPT table length overflow")?;
    let table_offset = primary
        .entries_lba
        .checked_mul(block_size_u64)
        .ok_or("GPT table offset overflow")?;
    let mut entries = vec![0; table_bytes];
    read_exact_at(&mut file, table_offset, &mut entries)?;
    let partitions = parse_gpt_partitions(primary, &entries)
        .map_err(|e| format!("GPT partition array: {e:?}"))?;
    let backup_table_offset = backup
        .entries_lba
        .checked_mul(block_size_u64)
        .ok_or("backup GPT table offset overflow")?;
    let mut backup_entries = vec![0; table_bytes];
    read_exact_at(&mut file, backup_table_offset, &mut backup_entries)?;
    parse_gpt_partitions(backup, &backup_entries)
        .map_err(|e| format!("backup GPT partition array: {e:?}"))?;
    if entries != backup_entries {
        return Err("primary and backup GPT partition arrays disagree".into());
    }
    Ok(DiskSnapshot {
        block_size,
        block_count,
        first_usable_lba: primary.first_usable_lba,
        last_usable_lba: primary.last_usable_lba,
        disk_guid: primary.disk_guid,
        partitions,
    })
}

pub fn guid_string(guid: [u8; 16]) -> String {
    format!("{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        guid[3],guid[2],guid[1],guid[0],guid[5],guid[4],guid[7],guid[6],guid[8],guid[9],
        guid[10],guid[11],guid[12],guid[13],guid[14],guid[15])
}
