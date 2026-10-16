use crate::disk::FileBlockWriter;
pub use lix_storage::{format_esp as lix_format_esp, EspFiles};
use std::fs::File;
pub fn format_esp(
    file: &mut File,
    logical_block_size: u32,
    partition_offset: u64,
    partition_bytes: u64,
    hidden_sectors: u32,
    volume_id: u32,
    files: EspFiles<'_>,
) -> Result<(), String> {
    let mut writer = FileBlockWriter(file);
    lix_format_esp(
        &mut writer,
        logical_block_size,
        partition_offset,
        partition_bytes,
        hidden_sectors,
        volume_id,
        files,
    )
    .map_err(std::string::ToString::to_string)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    #[test]
    fn formats_fat32_esp_with_512_and_4096_byte_logical_sectors() {
        let path = std::env::temp_dir().join(format!("fat32-test-{}.img", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        file.set_len(512 * 1024 * 1024).unwrap();
        let files = EspFiles {
            bootloader: b"bootloader",
            config: b"config",
            kernel: b"kernel",
            initrd: b"initrd",
        };
        format_esp(
            &mut file,
            512,
            1024 * 1024,
            511 * 1024 * 1024,
            2048,
            0x12345678,
            files,
        )
        .unwrap();
        let _ = std::fs::remove_file(&path);
    }
}
