use crate::disk::DiskSnapshot;
pub use lix_storage::{
    EFI_SYSTEM_PARTITION_TYPE_GUID as ESP_TYPE_GUID,
    LINUX_FILESYSTEM_TYPE_GUID as LINUX_FILESYSTEM_GUID,
};
const MIB: u64 = 1024 * 1024;
const MIN_ROOT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const ESP_BYTES: u64 = 512 * MIB;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallMode {
    WholeDisk,
    FreeSpace,
}
#[derive(Clone, Debug)]
pub struct PlannedPartition {
    pub name: &'static str,
    pub type_guid: [u8; 16],
    pub first_lba: u64,
    pub last_lba: u64,
}
impl PlannedPartition {
    pub const fn blocks(&self) -> u64 {
        self.last_lba - self.first_lba + 1
    }
}
#[derive(Clone, Debug)]
pub struct InstallPlan {
    pub mode: InstallMode,
    pub create_gpt: bool,
    pub erase_existing_partitions: bool,
    pub reused_esp: Option<[u8; 16]>,
    pub preserved_partitions: Vec<[u8; 16]>,
    pub partitions_to_create: Vec<PlannedPartition>,
}
fn align_up(value: u64, alignment: u64) -> Option<u64> {
    value
        .checked_add(alignment - 1)
        .map(|v| v / alignment * alignment)
}
fn validate_geometry(disk: &DiskSnapshot) -> Result<(u64, u64), String> {
    let block_size = u64::from(disk.block_size);
    if block_size < 512 || !block_size.is_power_of_two() || !MIB.is_multiple_of(block_size) {
        return Err("unsupported disk logical block size".into());
    }
    let mib_blocks = MIB / block_size;
    let min_root = MIN_ROOT_BYTES.div_ceil(block_size);
    if disk.block_count < min_root + 1024 {
        return Err("disk is below minimum install capacity".into());
    }
    if disk.first_usable_lba > disk.last_usable_lba || disk.last_usable_lba >= disk.block_count {
        return Err("invalid GPT usable-LBA range".into());
    }
    let mut ranges = disk.partitions.iter().collect::<Vec<_>>();
    ranges.sort_unstable_by_key(|partition| partition.first_lba);
    if ranges.iter().any(|partition| {
        partition.first_lba < disk.first_usable_lba
            || partition.last_lba > disk.last_usable_lba
            || partition.first_lba > partition.last_lba
            || partition.type_guid.iter().all(|byte| *byte == 0)
            || partition.unique_guid.iter().all(|byte| *byte == 0)
    }) || ranges
        .windows(2)
        .any(|pair| pair[0].last_lba >= pair[1].first_lba)
    {
        return Err("partition layout is out of range or overlapping".into());
    }
    let mut unique_guids = disk
        .partitions
        .iter()
        .map(|partition| partition.unique_guid)
        .collect::<Vec<_>>();
    unique_guids.sort_unstable();
    if unique_guids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("duplicate partition GUID".into());
    }
    Ok((mib_blocks, min_root))
}
pub fn plan_whole_disk(disk: &DiskSnapshot) -> Result<InstallPlan, String> {
    let (alignment, min_root) = validate_geometry(disk)?;
    let table_blocks = lix_storage::GPT_ENTRY_TABLE_BYTES.div_ceil(u64::from(disk.block_size));
    let backup_entries = disk
        .block_count
        .checked_sub(table_blocks + 1)
        .ok_or("disk too small for GPT")?;
    let last_usable = backup_entries
        .checked_sub(1)
        .ok_or("disk too small for GPT")?;
    let esp_blocks = ESP_BYTES.div_ceil(u64::from(disk.block_size));
    let esp_start = alignment.max(2 + table_blocks);
    let esp_last = esp_start
        .checked_add(esp_blocks - 1)
        .ok_or("ESP size overflow")?;
    let root_start = align_up(
        esp_last.checked_add(1).ok_or("root start overflow")?,
        alignment,
    )
    .ok_or("root start overflow")?;
    if last_usable < root_start || last_usable - root_start + 1 < min_root {
        return Err("not enough room for ESP and minimum root partition".into());
    }
    Ok(InstallPlan {
        mode: InstallMode::WholeDisk,
        create_gpt: true,
        erase_existing_partitions: true,
        reused_esp: None,
        preserved_partitions: Vec::new(),
        partitions_to_create: vec![
            PlannedPartition {
                name: "EFI System",
                type_guid: ESP_TYPE_GUID,
                first_lba: esp_start,
                last_lba: esp_last,
            },
            PlannedPartition {
                name: "LixOS root",
                type_guid: LINUX_FILESYSTEM_GUID,
                first_lba: root_start,
                last_lba: last_usable,
            },
        ],
    })
}
fn free_extents(disk: &DiskSnapshot) -> Vec<(u64, u64)> {
    let mut parts = disk.partitions.iter().collect::<Vec<_>>();
    parts.sort_unstable_by_key(|p| p.first_lba);
    let mut gaps = Vec::new();
    let mut cursor = disk.first_usable_lba;
    for part in parts {
        if part.first_lba > cursor {
            gaps.push((cursor, part.first_lba - 1));
        }
        cursor = part.last_lba.saturating_add(1).max(cursor);
    }
    if cursor <= disk.last_usable_lba {
        gaps.push((cursor, disk.last_usable_lba));
    }
    gaps
}
pub fn plan_free_space(disk: &DiskSnapshot) -> Result<InstallPlan, String> {
    let (alignment, min_root) = validate_geometry(disk)?;
    let mut esps = disk
        .partitions
        .iter()
        .filter(|part| part.type_guid == ESP_TYPE_GUID);
    let esp = esps
        .next()
        .ok_or("free-space mode requires an existing EFI System Partition")?;
    if esps.next().is_some() {
        return Err("multiple ESPs are ambiguous; selecting one is not supported".into());
    }
    let candidate = free_extents(disk)
        .into_iter()
        .filter_map(|(start, end)| {
            let aligned_start = align_up(start, alignment)?;
            (end >= aligned_start).then_some((aligned_start, end))
        })
        .filter(|(start, end)| end - start + 1 >= min_root)
        .max_by_key(|(start, end)| end - start + 1);
    let (root_start, root_end) = candidate.ok_or(
        "no unallocated extent can fit the minimum root partition; resizing is unsupported",
    )?;
    let mut preserved_partitions = Vec::new();
    preserved_partitions
        .try_reserve_exact(disk.partitions.len())
        .map_err(|_| "out of memory")?;
    preserved_partitions.extend(disk.partitions.iter().map(|part| part.unique_guid));
    Ok(InstallPlan {
        mode: InstallMode::FreeSpace,
        create_gpt: false,
        erase_existing_partitions: false,
        reused_esp: Some(esp.unique_guid),
        preserved_partitions,
        partitions_to_create: vec![PlannedPartition {
            name: "LixOS root",
            type_guid: LINUX_FILESYSTEM_GUID,
            first_lba: root_start,
            last_lba: root_end,
        }],
    })
}
pub fn plan_for(mode: InstallMode, disk: &DiskSnapshot) -> Result<InstallPlan, String> {
    match mode {
        InstallMode::WholeDisk => plan_whole_disk(disk),
        InstallMode::FreeSpace => plan_free_space(disk),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use lix_storage::GptPartition;
    fn empty_disk() -> DiskSnapshot {
        DiskSnapshot {
            block_size: 512,
            block_count: 20_971_520,
            first_usable_lba: 34,
            last_usable_lba: 20_971_486,
            disk_guid: [1; 16],
            partitions: Vec::new(),
        }
    }
    #[test]
    fn whole_disk_layout_aligns_esp_and_root_and_marks_erase() {
        let plan = plan_whole_disk(&empty_disk()).unwrap();
        assert!(plan.create_gpt && plan.erase_existing_partitions);
        assert_eq!(plan.partitions_to_create[0].first_lba, 2048);
        assert_eq!(plan.partitions_to_create[0].blocks(), ESP_BYTES / 512);
        assert_eq!(plan.partitions_to_create[1].first_lba % 2048, 0);
    }
    #[test]
    fn free_space_plan_keeps_existing_partitions_and_esp() {
        let mut disk = empty_disk();
        let esp_guid = [9; 16];
        disk.partitions.push(GptPartition {
            type_guid: ESP_TYPE_GUID,
            unique_guid: esp_guid,
            first_lba: 2048,
            last_lba: 1_050_623,
            attributes: 0,
        });
        disk.partitions.push(GptPartition {
            type_guid: [7; 16],
            unique_guid: [8; 16],
            first_lba: 1_200_000,
            last_lba: 1_500_000,
            attributes: 0,
        });
        let plan = plan_free_space(&disk).unwrap();
        assert!(!plan.create_gpt && !plan.erase_existing_partitions);
        assert_eq!(plan.reused_esp, Some(esp_guid));
        assert_eq!(plan.preserved_partitions.len(), 2);
        assert!(plan.partitions_to_create[0].first_lba > 1_500_000);
    }
    #[test]
    fn free_space_mode_refuses_missing_esp() {
        assert!(plan_free_space(&empty_disk()).is_err());
    }
}
