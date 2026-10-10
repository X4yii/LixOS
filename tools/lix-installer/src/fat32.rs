use std::fs::File;
use std::io::{Seek, SeekFrom, Write};

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

fn write_at(file: &mut File, offset: u64, bytes: &[u8]) -> Result<(), String> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn allocate_chain(fat: &mut [u32], next_cluster: &mut u32, count: usize) -> Result<u32, String> {
    if count == 0 {
        return Ok(0);
    }
    let end = next_cluster
        .checked_add(u32::try_from(count).map_err(|_| "FAT32 file is too large")?)
        .ok_or("FAT32 cluster index overflow")?;
    if end as usize > fat.len() {
        return Err("ESP is too small for its boot files".into());
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

fn append_entry(directory: &mut [u8], index: &mut usize, entry: [u8; 32]) -> Result<(), String> {
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
) -> Result<(), String> {
    if let Some(long_name) = entry.long_name {
        let units = long_name.encode_utf16().collect::<Vec<_>>();
        if units.is_empty() || units.len() > 13 {
            return Err("FAT32 LFN must fit in a single VFAT directory entry".into());
        }
        let mut lfn = [0xFF; 32];
        lfn[0] = 0x41;
        lfn[11] = 0x0F;
        lfn[12] = 0;
        lfn[13] = lfn_checksum(&entry.name);
        lfn[26] = 0;
        lfn[27] = 0;
        const OFFSETS: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
        for (offset, unit) in OFFSETS.iter().zip(units.iter()) {
            lfn[*offset..*offset + 2].copy_from_slice(&unit.to_le_bytes());
        }
        if units.len() < OFFSETS.len() {
            let terminator = units.len();
            lfn[OFFSETS[terminator]..OFFSETS[terminator] + 2].copy_from_slice(&0u16.to_le_bytes());
        }
        append_entry(directory, index, lfn)?;
    }
    append_entry(
        directory,
        index,
        make_entry(
            entry.name,
            0x20,
            entry.cluster,
            u32::try_from(entry.content.len()).map_err(|_| "FAT32 file exceeds 4 GiB")?,
        ),
    )
}

/// Formats one 512-byte-sector FAT32 ESP and installs Limine's fallback UEFI path.
/// This operates on a caller-owned regular image file; it does not open devices.
pub fn format_esp(
    file: &mut File,
    logical_block_size: u32,
    partition_offset: u64,
    partition_bytes: u64,
    hidden_sectors: u32,
    volume_id: u32,
    files: EspFiles<'_>,
) -> Result<(), String> {
    let sector_size = logical_block_size as u64;
    if !matches!(logical_block_size, 512 | 4096)
        || partition_bytes % sector_size != 0
        || partition_offset % sector_size != 0
        || hidden_sectors as u64 != partition_offset / sector_size
        || files.bootloader.is_empty()
        || files.config.is_empty()
        || files.kernel.is_empty()
        || files.initrd.is_empty()
    {
        return Err("invalid FAT32 ESP geometry or empty boot file".into());
    }
    let sectors_per_cluster = (4096 / logical_block_size) as u32;
    let total_sectors =
        u32::try_from(partition_bytes / sector_size).map_err(|_| "FAT32 ESP exceeds 2 TiB")?;
    let end = partition_offset
        .checked_add(partition_bytes)
        .ok_or("FAT32 ESP range overflow")?;
    if end > file.metadata().map_err(|error| error.to_string())?.len() {
        return Err("FAT32 ESP exceeds the image bounds".into());
    }
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
    if cluster_count < 65_525 || cluster_count > 0x0FFF_FFEF {
        return Err("ESP geometry is outside FAT32 cluster limits".into());
    }
    if sectors_per_fat as u64 * sector_size > MAX_FAT_ALLOCATION_BYTES {
        return Err("FAT32 allocation table exceeds installer memory limit".into());
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
        let size = u32::try_from(entry.content.len()).map_err(|_| "FAT32 file exceeds 4 GiB")?;
        let clusters = entry.content.len().div_ceil(cluster_bytes);
        entry.cluster = allocate_chain(&mut fat, &mut next_cluster, clusters)?;
        if entry.content.len() != size as usize {
            return Err("FAT32 file length is not representable".into());
        }
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

    let mut fs_info = vec![0u8; logical_block_size as usize];
    put_u32(&mut fs_info, 0, 0x4161_5252);
    put_u32(&mut fs_info, 484, 0x6141_7272);
    put_u32(&mut fs_info, 488, cluster_count - (next_cluster - 2));
    put_u32(&mut fs_info, 492, next_cluster);
    put_u32(&mut fs_info, 508, 0xAA55_0000);
    write_at(file, partition_offset, &boot_sector)?;
    write_at(file, partition_offset + sector_size, &fs_info)?;
    write_at(file, partition_offset + 6 * sector_size, &boot_sector)?;
    write_at(file, partition_offset + 7 * sector_size, &fs_info)?;

    let fat_bytes_len = sectors_per_fat as usize * logical_block_size as usize;
    let mut fat_bytes = vec![0u8; fat_bytes_len];
    for (index, value) in fat.iter().enumerate() {
        let offset = index * 4;
        fat_bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let fat_start = partition_offset + RESERVED_SECTORS as u64 * sector_size;
    write_at(file, fat_start, &fat_bytes)?;
    write_at(
        file,
        fat_start + sectors_per_fat as u64 * sector_size,
        &fat_bytes,
    )?;

    let mut root_dir = vec![0u8; cluster_bytes];
    let mut root_index = 0;
    append_entry(
        &mut root_dir,
        &mut root_index,
        make_entry(*b"EFI        ", 0x10, efi_cluster, 0),
    )?;
    for entry in &root_files {
        append_file_entry(&mut root_dir, &mut root_index, entry)?;
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

    let write_cluster = |file: &mut File, cluster: u32, content: &[u8]| -> Result<(), String> {
        let relative_sector = data_start as u64 + (cluster as u64 - 2) * sectors_per_cluster as u64;
        let offset = partition_offset
            .checked_add(relative_sector * sector_size)
            .ok_or("FAT32 data offset overflow")?;
        let mut padded = vec![0u8; cluster_bytes];
        let amount = content.len().min(cluster_bytes);
        padded[..amount].copy_from_slice(&content[..amount]);
        write_at(file, offset, &padded)
    };
    write_cluster(file, root_cluster, &root_dir)?;
    write_cluster(file, efi_cluster, &efi_dir)?;
    write_cluster(file, boot_cluster, &boot_dir)?;

    let mut write_file_clusters = |entry: &FileEntry<'_>| -> Result<(), String> {
        if entry.cluster == 0 {
            return Ok(());
        }
        for (index, chunk) in entry.content.chunks(cluster_bytes).enumerate() {
            write_cluster(file, entry.cluster + index as u32, chunk)?;
        }
        Ok(())
    };
    for entry in root_files.iter().chain(boot_files.iter()) {
        write_file_clusters(entry)?;
    }
    file.sync_all().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn format_and_verify(logical_block_size: u32) {
        let path = std::env::temp_dir().join(format!(
            "lixos-fat32-test-{}-{logical_block_size}.img",
            std::process::id()
        ));
        assert!(path.symlink_metadata().is_err());
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|error| error.to_string())?;
            let partition_offset = 1024 * 1024;
            let partition_bytes = 320 * 1024 * 1024;
            let sectors_per_cluster = 4096 / logical_block_size;
            let sector_size = logical_block_size as u64;
            let hidden_sectors = (partition_offset / sector_size) as u32;
            file.set_len(partition_offset + partition_bytes)
                .map_err(|error| error.to_string())?;
            let config = b"timeout: 0\n";
            format_esp(
                &mut file,
                logical_block_size,
                partition_offset,
                partition_bytes,
                hidden_sectors,
                0x1357_2468,
                EspFiles {
                    bootloader: b"EFI binary",
                    config,
                    kernel: b"kernel image",
                    initrd: b"init ramdisk",
                },
            )?;

            let mut boot = vec![0; logical_block_size as usize];
            file.seek(SeekFrom::Start(partition_offset))
                .map_err(|error| error.to_string())?;
            file.read_exact(&mut boot)
                .map_err(|error| error.to_string())?;
            assert_eq!(&boot[510..512], &[0x55, 0xAA]);
            assert_eq!(
                u16::from_le_bytes(boot[11..13].try_into().unwrap()),
                logical_block_size as u16
            );
            assert_eq!(boot[13], sectors_per_cluster as u8);
            assert_eq!(
                u32::from_le_bytes(boot[28..32].try_into().unwrap()),
                hidden_sectors
            );
            assert_eq!(&boot[82..90], b"FAT32   ");

            let sectors_per_fat = u32::from_le_bytes(boot[36..40].try_into().unwrap());
            let root_cluster = u32::from_le_bytes(boot[44..48].try_into().unwrap());
            let first_data_sector = RESERVED_SECTORS + FAT_COUNT * sectors_per_fat;
            let root_offset = partition_offset
                + (first_data_sector as u64
                    + (root_cluster as u64 - 2) * sectors_per_cluster as u64)
                    * sector_size;
            let cluster_bytes = sectors_per_cluster as usize * logical_block_size as usize;
            let mut directory = vec![0; cluster_bytes];
            file.seek(SeekFrom::Start(root_offset))
                .map_err(|error| error.to_string())?;
            file.read_exact(&mut directory)
                .map_err(|error| error.to_string())?;
            assert_eq!(&directory[11..12], &[0x10]);
            assert_eq!(&directory[32 + 11..32 + 12], &[0x20]);
            let lfn = &directory[3 * 32..4 * 32];
            assert_eq!(lfn[11], 0x0F);
            assert_eq!(lfn[0], 0x41);
            assert_eq!(&directory[4 * 32..4 * 32 + 11], b"LIMINE  CFG");

            let fat_start = partition_offset + RESERVED_SECTORS as u64 * sector_size;
            let mut first_fat_entry = [0; 4];
            file.seek(SeekFrom::Start(fat_start))
                .map_err(|error| error.to_string())?;
            file.read_exact(&mut first_fat_entry)
                .map_err(|error| error.to_string())?;
            assert_eq!(u32::from_le_bytes(first_fat_entry), 0x0FFF_FFF8);
            Ok::<(), String>(())
        })();
        let _ = std::fs::remove_file(path);
        result.unwrap();
    }

    #[test]
    fn formats_fat32_esp_with_512_and_4096_byte_logical_sectors() {
        for logical_block_size in [512, 4096] {
            format_and_verify(logical_block_size);
        }
    }
}
