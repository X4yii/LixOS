use crate::util;
use lix_storage::{
    build_gpt_image, ext4_mkfs_args, write_gpt_layout, BlockWriter, GptPartition,
    LINUX_FILESYSTEM_TYPE_GUID,
};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::process::Command;
pub const ISO_ROOT_DIR: &str = "target/iso_root";
pub const ISO_OUTPUT: &str = "target/lixos.iso";
pub const INITRAMFS_TAR: &str = "target/iso_root/initramfs.tar";
pub const NVME_IMAGE_512: &str = "target/lixos_nvme.img";
pub const NVME_IMAGE_4K: &str = "target/lixos_nvme_4k.img";
pub const ROOTFS_DIR: &str = "rootfs";
pub const KERNEL_DEBUG_ELF: &str = "target/x86_64-unknown-none/debug/kernel";
struct FileBlockWriter<'a>(&'a mut File);
impl BlockWriter for FileBlockWriter<'_> {
    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<(), &'static str> {
        self.0
            .seek(SeekFrom::Start(offset))
            .map_err(|_| "error de seek en la imagen")?;
        self.0
            .write_all(bytes)
            .map_err(|_| "error de escritura en la imagen")?;
        Ok(())
    }
}
fn format_ext4_at(
    image_path: &Path,
    partition_offset: u64,
    partition_blocks_4k: u64,
    rootfs_dir: &Path,
) -> Result<(), String> {
    let image = image_path.to_str().ok_or("ruta de imagen inválida")?;
    let rootfs = rootfs_dir.to_str().ok_or("ruta de rootfs inválida")?;
    let offset = format!("offset={partition_offset}");
    let blocks = partition_blocks_4k.to_string();
    let status = Command::new("mkfs.ext4")
        .args(ext4_mkfs_args(rootfs, &offset, image, &blocks))
        .status()
        .map_err(|error| format!("mkfs.ext4: {error}"))?;
    if !status.success() {
        return Err(format!("mkfs.ext4 terminó con estado {status}"));
    }
    Ok(())
}
pub fn create_nvme_test_image(path: &Path, logical_block_size: u64) -> Result<(), String> {
    if !matches!(logical_block_size, 512 | 4096) {
        return Err("logical block size must be 512 or 4096 bytes".into());
    }
    if path.symlink_metadata().is_ok() {
        return Err(format!("{} ya existe; no se sobrescribió", path.display()));
    }
    let image_path = path.to_str().ok_or("ruta inválida")?;
    util::run_checked(
        "qemu-img",
        &["create", "-f", "raw", image_path, "10G"],
        "creación de la imagen NVMe de prueba",
    )?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("no se pudo abrir {}: {error}", path.display()))?;
    let image_bytes = file
        .metadata()
        .map_err(|error| format!("no se pudo medir {}: {error}", path.display()))?
        .len();
    let total_lbas = image_bytes / logical_block_size;
    let alignment_blocks = 1024 * 1024 / logical_block_size;
    let partition_first = alignment_blocks;
    let table_blocks = lix_storage::GPT_ENTRY_TABLE_BYTES.div_ceil(logical_block_size);
    let partition_last = total_lbas
        .checked_sub(table_blocks + 2)
        .ok_or("imagen demasiado pequeña")?;
    let partition_lba_count = partition_last - partition_first + 1;
    let layout = build_gpt_image(
        logical_block_size as u32,
        total_lbas,
        [
            0x4c, 0x69, 0x78, 0x4f, 0x53, 0x54, 0x45, 0x53, 0x54, 0, 0, 0, 0, 0, 0, 1,
        ],
        &[GptPartition {
            type_guid: LINUX_FILESYSTEM_TYPE_GUID,
            unique_guid: [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            first_lba: partition_first,
            last_lba: partition_last,
            attributes: 0,
        }],
    )
    .map_err(|error| format!("GPT layout: {error:?}"))?;
    {
        let mut writer = FileBlockWriter(&mut file);
        write_gpt_layout(&mut writer, &layout, logical_block_size, total_lbas)?;
    }
    file.sync_all()
        .map_err(|error| format!("no se pudo sincronizar {}: {error}", path.display()))?;
    let partition_blocks = partition_lba_count
        .checked_mul(logical_block_size)
        .ok_or("partición demasiado grande")?
        / 4096;
    format_ext4_at(
        path,
        partition_first * logical_block_size,
        partition_blocks,
        Path::new(ROOTFS_DIR),
    )
}
