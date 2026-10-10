use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use lix_storage::{build_gpt_image, GptPartition};

use crate::disk::{guid_string, inspect_image};
use crate::fat32::{format_esp, EspFiles};
use crate::plan::{ESP_TYPE_GUID, LINUX_FILESYSTEM_GUID};

const ESP_MIB: u64 = 512;
const GIB_BYTES: u64 = 1024 * 1024 * 1024;
const GPT_ENTRY_TABLE_BYTES: u64 = 128 * 128;
const MAX_BOOT_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;

fn validate_output_parent(output: &Path) -> Result<(PathBuf, PathBuf), String> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let canonical_parent =
        fs::canonicalize(parent).map_err(|error| format!("output parent: {error}"))?;
    let metadata = fs::metadata(&canonical_parent).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || metadata.mode() & 0o022 != 0 {
        return Err("output parent must be a directory not writable by group or others".into());
    }
    let effective_uid = fs::read_to_string("/proc/self/status")
        .map_err(|error| format!("cannot verify effective user: {error}"))?
        .lines()
        .find_map(|line| {
            line.strip_prefix("Uid:")
                .and_then(|value| value.split_whitespace().nth(1))
        })
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or("cannot determine effective user")?;
    if metadata.uid() != effective_uid {
        return Err("output parent must be owned by the invoking user".into());
    }
    let filename = output.file_name().ok_or("output must include a filename")?;
    Ok((canonical_parent.clone(), canonical_parent.join(filename)))
}

fn random_guid() -> Result<[u8; 16], String> {
    let mut guid = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut guid))
        .map_err(|error| format!("no se pudo generar GUID aleatorio: {error}"))?;
    guid[7] = (guid[7] & 0x0F) | 0x40;
    guid[8] = (guid[8] & 0x3F) | 0x80;
    Ok(guid)
}

fn write_at(file: &mut File, offset: u64, content: &[u8]) -> Result<(), String> {
    use std::io::{Seek, SeekFrom, Write};
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    file.write_all(content).map_err(|error| error.to_string())
}

fn source_size(path: &Path, label: &str) -> Result<u64, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| format!("{label}: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err(format!("{label} debe ser un archivo regular"));
    }
    Ok(metadata.len())
}

fn make_ext4(
    image_path: &Path,
    root_offset: u64,
    root_blocks_4k: u64,
    rootfs: &Path,
) -> Result<(), String> {
    let offset = format!("offset={root_offset}");
    let blocks = root_blocks_4k.to_string();
    let status = Command::new("mkfs.ext4")
        .args([
            "-q", "-F", "-b", "4096", "-O",
            "extents,uninit_bg,dir_index,filetype,has_journal,^64bit,^flex_bg,^ea_inode,^metadata_csum,^metadata_csum_seed",
            "-d", rootfs.to_str().ok_or("ruta rootfs inválida")?, "-E", &offset,
            image_path.to_str().ok_or("ruta de imagen inválida")?, &blocks,
        ])
        .status()
        .map_err(|error| format!("mkfs.ext4: {error}"))?;
    if !status.success() {
        return Err("mkfs.ext4 falló al preparar la partición raíz".into());
    }
    Ok(())
}

fn populate_image(
    file: &mut File,
    image_path: &Path,
    size_gib: u64,
    logical_block_size: u32,
    kernel: &[u8],
    initrd: &[u8],
    bootloader: &[u8],
    rootfs: &Path,
) -> Result<[u8; 16], String> {
    let block_size = logical_block_size as u64;
    let total_blocks = size_gib
        .checked_mul(GIB_BYTES / block_size)
        .ok_or("image size overflow")?;
    let disk_guid = random_guid()?;
    let esp_guid = random_guid()?;
    let root_guid = random_guid()?;
    let alignment_blocks = 1024 * 1024 / block_size;
    let esp_start = alignment_blocks;
    let esp_blocks = ESP_MIB * 1024 * 1024 / block_size;
    let root_start = (esp_start + esp_blocks).div_ceil(alignment_blocks) * alignment_blocks;
    let table_blocks = GPT_ENTRY_TABLE_BYTES.div_ceil(block_size);
    let last_usable = total_blocks
        .checked_sub(table_blocks + 2)
        .ok_or("image is too small for GPT")?;
    if root_start >= last_usable {
        return Err("image is too small for ESP and root".into());
    }
    let partitions = [
        GptPartition {
            type_guid: ESP_TYPE_GUID,
            unique_guid: esp_guid,
            first_lba: esp_start,
            last_lba: root_start - 1,
            attributes: 0,
        },
        GptPartition {
            type_guid: LINUX_FILESYSTEM_GUID,
            unique_guid: root_guid,
            first_lba: root_start,
            last_lba: last_usable,
            attributes: 0,
        },
    ];
    let gpt = build_gpt_image(logical_block_size, total_blocks, disk_guid, &partitions)
        .map_err(|error| format!("GPT generation: {error:?}"))?;
    write_at(file, 0, &gpt.protective_mbr)?;
    write_at(file, block_size, &gpt.primary_header)?;
    write_at(file, 2 * block_size, &gpt.primary_entries)?;
    write_at(
        file,
        gpt.backup_entries_lba * block_size,
        &gpt.backup_entries,
    )?;
    write_at(file, (total_blocks - 1) * block_size, &gpt.backup_header)?;
    file.sync_all().map_err(|error| error.to_string())?;

    let config = format!(
        "timeout: 3\n\n/LixOS\n    protocol: limine\n    kernel_path: boot():/KERNEL.ELF\n    module_path: boot():/INITRD.TAR\n    cmdline: root=PARTUUID={}\n",
        guid_string(root_guid)
    );
    format_esp(
        file,
        logical_block_size,
        esp_start * block_size,
        esp_blocks * block_size,
        u32::try_from(esp_start).map_err(|_| "ESP start exceeds FAT32 BPB range")?,
        u32::from_le_bytes(disk_guid[..4].try_into().unwrap()),
        EspFiles {
            bootloader,
            config: config.as_bytes(),
            kernel,
            initrd,
        },
    )?;

    let root_offset = root_start * block_size;
    let root_blocks_4k = (last_usable - root_start + 1) * block_size / 4096;
    make_ext4(image_path, root_offset, root_blocks_4k, rootfs)?;
    file.sync_all().map_err(|error| error.to_string())?;
    let disk = inspect_image(image_path, logical_block_size)?;
    if !disk
        .partitions
        .iter()
        .any(|partition| partition.unique_guid == root_guid)
    {
        return Err("GPT verification did not find the generated root PARTUUID".into());
    }
    Ok(root_guid)
}

/// Builds a bootable raw image without ever opening an existing path for writing.
pub fn create_bootable_image(
    output: &Path,
    size_gib: u64,
    logical_block_size: u32,
    kernel_path: &Path,
    initrd_path: &Path,
    bootloader_path: &Path,
    rootfs: &Path,
) -> Result<String, String> {
    if size_gib < 4 || size_gib > 128 {
        return Err("image size must be between 4 and 128 GiB".into());
    }
    if !matches!(logical_block_size, 512 | 4096) {
        return Err("logical block size must be 512 or 4096 bytes".into());
    }
    let (parent, final_path) = validate_output_parent(output)?;
    match final_path.symlink_metadata() {
        Ok(_) => return Err("output already exists; it was not modified".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot inspect output path: {error}")),
    }
    let rootfs_metadata = fs::symlink_metadata(rootfs).map_err(|error| error.to_string())?;
    if !rootfs_metadata.file_type().is_dir() {
        return Err("rootfs must be a directory".into());
    }
    let kernel_bytes = source_size(kernel_path, "kernel")?;
    let initrd_bytes = source_size(initrd_path, "initrd")?;
    let bootloader_bytes = source_size(bootloader_path, "UEFI bootloader")?;
    let artifact_bytes = kernel_bytes
        .checked_add(initrd_bytes)
        .and_then(|size| size.checked_add(bootloader_bytes))
        .ok_or("boot artifact size overflow")?;
    if artifact_bytes > MAX_BOOT_ARTIFACT_BYTES {
        return Err("boot artifacts exceed the installer memory limit".into());
    }
    let kernel = fs::read(kernel_path).map_err(|error| format!("kernel: {error}"))?;
    let initrd = fs::read(initrd_path).map_err(|error| format!("initrd: {error}"))?;
    let bootloader =
        fs::read(bootloader_path).map_err(|error| format!("UEFI bootloader: {error}"))?;
    let staging_path = parent.join(format!(
        ".lixos-image-{}-{}.staging",
        std::process::id(),
        guid_string(random_guid()?)
    ));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&staging_path)
        .map_err(|error| format!("cannot create unique image staging file: {error}"))?;
    let image_bytes = size_gib
        .checked_mul(GIB_BYTES)
        .ok_or("image size overflow")?;
    let result = (|| {
        file.set_len(image_bytes)
            .map_err(|error| error.to_string())?;
        populate_image(
            &mut file,
            &staging_path,
            size_gib,
            logical_block_size,
            &kernel,
            &initrd,
            &bootloader,
            rootfs,
        )
    })();
    drop(file);
    let root_guid = match result {
        Ok(root_guid) => root_guid,
        Err(error) => {
            let _ = fs::remove_file(&staging_path);
            return Err(error);
        }
    };
    if let Err(error) = fs::hard_link(&staging_path, &final_path) {
        let _ = fs::remove_file(&staging_path);
        return Err(format!(
            "completed image was not published; output may have appeared concurrently: {error}"
        ));
    }
    fs::remove_file(&staging_path).map_err(|error| {
        format!(
            "image published at {}; staging cleanup failed: {error}",
            final_path.display()
        )
    })?;
    File::open(&parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("image published but parent directory sync failed: {error}"))?;
    Ok(guid_string(root_guid))
}
