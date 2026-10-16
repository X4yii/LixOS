use crate::writer::BlockWriter;
use alloc::vec;
use alloc::vec::Vec;
const RESERVED_SECTORS: u32 = 32;
const FAT_COUNT: u32 = 2;
const FAT32_EOC: u32 = 0x0FFF_FFFF;
const MAX_FAT_ALLOCATION_BYTES: u64 = 32 * 1024 * 1024;
pub struct EspFiles<'a> {
    pub bootloader: &'a [u8],
    pub config: &'a [u8],
    pub kernel: &'a [u8],
    pub initrd: &'a [u8],
}
#[derive(Clone, Copy)]
struct FileEntry<'a> {
    name: [u8; 11],
    long_name: Option<&'static str>,
    content: &'a [u8],
    cluster: u32,
}
fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn allocate_chain(
    fat: &mut [u32],
    next_cluster: &mut u32,
    count: usize,
) -> Result<u32, &'static str> {
    if count == 0 {
        return Ok(0);
    }
    let count_u32 = u32::try_from(count).map_err(|_| "FAT32 file is too large")?;
    let end = next_cluster
        .checked_add(count_u32)
        .ok_or("FAT32 cluster index overflow")?;
    if end as usize > fat.len() {
        return Err("ESP is too small for its boot files");
    }
    let first = *next_cluster;
    while *next_cluster < end {
        let current = *next_cluster;
        fat[current as usize] = if current + 1 == end {
            FAT32_EOC
        } else {
            current + 1
        };
        *next_cluster += 1;
    }
    Ok(first)
}
fn make_entry(name: [u8; 11], attr: u8, cluster: u32, size: u32) -> [u8; 32] {
    let mut entry = [0; 32];
    entry[..11].copy_from_slice(&name);
    entry[11] = attr;
    put_u16(&mut entry, 16, 0x0021);
    put_u16(&mut entry, 18, 0x0021);
    put_u16(&mut entry, 24, 0x0021);
    put_u16(&mut entry, 20, (cluster >> 16) as u16);
    put_u16(&mut entry, 26, cluster as u16);
    put_u32(&mut entry, 28, size);
    entry
}
fn append_entry(
    directory: &mut [u8],
    index: &mut usize,
    entry: [u8; 32],
) -> Result<(), &'static str> {
    let start = index.checked_mul(32).ok_or("directory size overflow")?;
    let end = start.checked_add(32).ok_or("directory size overflow")?;
    let target = directory
        .get_mut(start..end)
        .ok_or("FAT32 directory is full")?;
    target.copy_from_slice(&entry);
    *index += 1;
    Ok(())
}
fn dot_entry(cluster: u32, parent: bool) -> [u8; 32] {
    let mut name = [b' '; 11];
    name[0] = b'.';
    if parent {
        name[1] = b'.';
    }
    make_entry(name, 0x10, cluster, 0)
}
fn lfn_checksum(short_name: &[u8; 11]) -> u8 {
    short_name
        .iter()
        .fold(0u8, |sum, byte| sum.rotate_right(1).wrapping_add(*byte))
}
fn append_file_entry(
    directory: &mut [u8],
    index: &mut usize,
    entry: &FileEntry<'_>,
) -> Result<(), &'static str> {
    if let Some(long_name) = entry.long_name {
        let mut units = Vec::new();
        for ch in long_name.encode_utf16() {
            units.push(ch);
        }
        if units.is_empty() || units.len() > 13 {
            return Err("FAT32 LFN must fit in a single VFAT directory entry");
        }
        let mut lfn = [0xFF; 32];
        lfn[0] = 0x41;
        lfn[11] = 0x0F;
        lfn[12] = 0;
        lfn[13] = lfn_checksum(&entry.name);
        put_u16(&mut lfn, 26, 0);
        let pairs = [
            (1, 0..5.min(units.len())),
            (14, 5..11.min(units.len())),
            (28, 11..13.min(units.len())),
        ];
        for (offset, range) in pairs {
            let mut cursor = offset;
            for index in range {
                put_u16(&mut lfn, cursor, units[index]);
                cursor += 2;
            }
            if units.len()
                < (match offset {
                    1 => 5,
                    14 => 11,
                    _ => 13,
                })
                && cursor + 2 <= 32
            {
                put_u16(&mut lfn, cursor, 0);
            }
        }
        append_entry(directory, index, lfn)?;
    }
    let size = u32::try_from(entry.content.len()).map_err(|_| "FAT32 file exceeds 4 GiB")?;
    append_entry(
        directory,
        index,
        make_entry(entry.name, 0x20, entry.cluster, size),
    )
}
fn write_fat(
    writer: &mut impl BlockWriter,
    base_offset: u64,
    sector_size: u64,
    sectors_per_fat: u32,
    fat: &[u32],
) -> Result<(), &'static str> {
    let fat_bytes = fat.len().checked_mul(4).ok_or("FAT byte overflow")?;
    let mut buffer = vec![0u8; fat_bytes];
    for (index, entry) in fat.iter().enumerate() {
        put_u32(&mut buffer, index * 4, *entry);
    }
    for fat_index in 0..FAT_COUNT {
        let offset = base_offset
            .checked_add(u64::from(RESERVED_SECTORS + fat_index * sectors_per_fat) * sector_size)
            .ok_or("FAT table offset overflow")?;
        writer.write_at(offset, &buffer)?;
    }
    Ok(())
}
fn write_cluster_data(
    writer: &mut impl BlockWriter,
    base_offset: u64,
    data_start_sector: u32,
    sectors_per_cluster: u32,
    sector_size: u64,
    cluster: u32,
    bytes: &[u8],
) -> Result<(), &'static str> {
    let cluster_offset = cluster.checked_sub(2).ok_or("invalid FAT32 data cluster")?;
    let sector = u64::from(cluster_offset)
        .checked_mul(u64::from(sectors_per_cluster))
        .and_then(|value| value.checked_add(u64::from(data_start_sector)))
        .ok_or("FAT32 cluster LBA overflow")?;
    let offset = base_offset
        .checked_add(sector * sector_size)
        .ok_or("FAT32 data offset overflow")?;
    writer.write_at(offset, bytes)
}
pub fn format_esp(
    writer: &mut impl BlockWriter,
    logical_block_size: u32,
    partition_offset: u64,
    partition_bytes: u64,
    hidden_sectors: u32,
    volume_id: u32,
    files: EspFiles<'_>,
) -> Result<(), &'static str> {
    let sector_size = u64::from(logical_block_size);
    if !matches!(logical_block_size, 512 | 4096)
        || partition_bytes == 0
        || !partition_offset.is_multiple_of(sector_size)
        || u64::from(hidden_sectors) != partition_offset / sector_size
        || files.bootloader.is_empty()
        || files.config.is_empty()
        || files.kernel.is_empty()
    {
        return Err("invalid FAT32 ESP geometry or empty boot file");
    }
    let sectors_per_cluster = 4096 / logical_block_size;
    let total_sectors =
        u32::try_from(partition_bytes / sector_size).map_err(|_| "FAT32 ESP exceeds 2 TiB")?;
    let mut sectors_per_fat = 1u32;
    let cluster_count = loop {
        let data_sectors = total_sectors
            .checked_sub(RESERVED_SECTORS + FAT_COUNT * sectors_per_fat)
            .ok_or("FAT32 ESP is too small")?;
        let clusters = data_sectors / sectors_per_cluster;
        let required_fat_sectors = clusters
            .checked_add(2)
            .and_then(|entries| entries.checked_mul(4))
            .ok_or("FAT32 allocation table overflow")?
            .div_ceil(logical_block_size);
        if required_fat_sectors == sectors_per_fat {
            break clusters;
        }
        sectors_per_fat = required_fat_sectors;
    };
    if !(65_525..=0x0FFF_FFEF).contains(&cluster_count) {
        return Err("ESP geometry is outside FAT32 cluster limits");
    }
    if u64::from(sectors_per_fat) * sector_size > MAX_FAT_ALLOCATION_BYTES {
        return Err("FAT32 allocation table exceeds memory limit");
    }
    let data_start = RESERVED_SECTORS + FAT_COUNT * sectors_per_fat;
    let cluster_bytes = (sectors_per_cluster as usize)
        .checked_mul(logical_block_size as usize)
        .ok_or("FAT32 cluster size overflow")?;
    let mut fat = vec![0u32; cluster_count as usize + 2];
    fat[0] = 0x0FFF_FFF8;
    fat[1] = FAT32_EOC;
    let mut next_cluster = 2;
    let root_cluster = allocate_chain(&mut fat, &mut next_cluster, 1)?;
    let efi_cluster = allocate_chain(&mut fat, &mut next_cluster, 1)?;
    let boot_cluster = allocate_chain(&mut fat, &mut next_cluster, 1)?;
    let mut root_files = [
        FileEntry {
            name: *b"KERNEL  ELF",
            long_name: None,
            content: files.kernel,
            cluster: 0,
        },
        FileEntry {
            name: *b"INITRD  TAR",
            long_name: None,
            content: files.initrd,
            cluster: 0,
        },
        FileEntry {
            name: *b"LIMINE  CFG",
            long_name: Some("limine.conf"),
            content: files.config,
            cluster: 0,
        },
    ];
    let mut boot_files = [
        FileEntry {
            name: *b"BOOTX64 EFI",
            long_name: None,
            content: files.bootloader,
            cluster: 0,
        },
        FileEntry {
            name: *b"LIMINE  CFG",
            long_name: Some("limine.conf"),
            content: files.config,
            cluster: 0,
        },
    ];
    for entry in root_files.iter_mut().chain(boot_files.iter_mut()) {
        if entry.content.is_empty() {
            continue;
        }
        let clusters = entry.content.len().div_ceil(cluster_bytes);
        entry.cluster = allocate_chain(&mut fat, &mut next_cluster, clusters)?;
    }
    let mut boot_sector = vec![0u8; logical_block_size as usize];
    boot_sector[..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    boot_sector[3..11].copy_from_slice(b"LIXOS   ");
    put_u16(&mut boot_sector, 11, logical_block_size as u16);
    boot_sector[13] = sectors_per_cluster as u8;
    put_u16(&mut boot_sector, 14, RESERVED_SECTORS as u16);
    boot_sector[16] = FAT_COUNT as u8;
    boot_sector[21] = 0xF8;
    put_u16(&mut boot_sector, 24, 63);
    put_u16(&mut boot_sector, 26, 255);
    put_u32(&mut boot_sector, 28, hidden_sectors);
    put_u32(&mut boot_sector, 32, total_sectors);
    put_u32(&mut boot_sector, 36, sectors_per_fat);
    put_u32(&mut boot_sector, 44, root_cluster);
    put_u16(&mut boot_sector, 48, 1);
    put_u16(&mut boot_sector, 50, 6);
    boot_sector[64] = 0x80;
    boot_sector[66] = 0x29;
    put_u32(&mut boot_sector, 67, volume_id);
    boot_sector[71..82].copy_from_slice(b"LIXOS ESP  ");
    boot_sector[82..90].copy_from_slice(b"FAT32   ");
    boot_sector[510..512].copy_from_slice(&[0x55, 0xAA]);
    let mut fsinfo = vec![0u8; logical_block_size as usize];
    fsinfo[..4].copy_from_slice(&[0x52, 0x52, 0x61, 0x41]);
    fsinfo[484..488].copy_from_slice(&[0x72, 0x72, 0x41, 0x61]);
    let free_clusters = cluster_count.saturating_sub(next_cluster - 2);
    put_u32(&mut fsinfo, 488, free_clusters);
    put_u32(&mut fsinfo, 492, next_cluster);
    fsinfo[508..512].copy_from_slice(&[0x00, 0x00, 0x55, 0xAA]);
    writer.write_at(partition_offset, &boot_sector)?;
    writer.write_at(partition_offset + sector_size, &fsinfo)?;
    writer.write_at(partition_offset + 6 * sector_size, &boot_sector)?;
    write_fat(writer, partition_offset, sector_size, sectors_per_fat, &fat)?;
    let mut root_dir = vec![0u8; cluster_bytes];
    let mut root_index = 0;
    append_entry(
        &mut root_dir,
        &mut root_index,
        make_entry(*b"EFI        ", 0x10, efi_cluster, 0),
    )?;
    for entry in &root_files {
        if !entry.content.is_empty() {
            append_file_entry(&mut root_dir, &mut root_index, entry)?;
        }
    }
    let mut efi_dir = vec![0u8; cluster_bytes];
    let mut efi_index = 0;
    append_entry(&mut efi_dir, &mut efi_index, dot_entry(efi_cluster, false))?;
    append_entry(&mut efi_dir, &mut efi_index, dot_entry(root_cluster, true))?;
    append_entry(
        &mut efi_dir,
        &mut efi_index,
        make_entry(*b"BOOT       ", 0x10, boot_cluster, 0),
    )?;
    let mut boot_dir = vec![0u8; cluster_bytes];
    let mut boot_index = 0;
    append_entry(
        &mut boot_dir,
        &mut boot_index,
        dot_entry(boot_cluster, false),
    )?;
    append_entry(&mut boot_dir, &mut boot_index, dot_entry(efi_cluster, true))?;
    for entry in &boot_files {
        append_file_entry(&mut boot_dir, &mut boot_index, entry)?;
    }
    write_cluster_data(
        writer,
        partition_offset,
        data_start,
        sectors_per_cluster,
        sector_size,
        root_cluster,
        &root_dir,
    )?;
    write_cluster_data(
        writer,
        partition_offset,
        data_start,
        sectors_per_cluster,
        sector_size,
        efi_cluster,
        &efi_dir,
    )?;
    write_cluster_data(
        writer,
        partition_offset,
        data_start,
        sectors_per_cluster,
        sector_size,
        boot_cluster,
        &boot_dir,
    )?;
    for entry in root_files.iter().chain(boot_files.iter()) {
        if entry.content.is_empty() {
            continue;
        }
        let mut cluster = entry.cluster;
        let mut offset = 0;
        while offset < entry.content.len() {
            let chunk_len = cluster_bytes.min(entry.content.len() - offset);
            let mut chunk = vec![0u8; cluster_bytes];
            chunk[..chunk_len].copy_from_slice(&entry.content[offset..offset + chunk_len]);
            write_cluster_data(
                writer,
                partition_offset,
                data_start,
                sectors_per_cluster,
                sector_size,
                cluster,
                &chunk,
            )?;
            offset += chunk_len;
            let next = fat[cluster as usize];
            if next == FAT32_EOC {
                break;
            }
            cluster = next;
        }
    }
    Ok(())
}
