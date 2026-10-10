use crate::sync::IrqSafeSpinlock;
use crate::memory::pmm::PAGE_SIZE;
use crate::drivers::block::{BlockDevice, DmaBuffer};
use crate::fs::vfs::FileSystem;
use alloc::vec::Vec;
use alloc::string::String;

const MAX_EXT4_FILE_SIZE: usize = 64 * 1024 * 1024;
const MAX_EXTENT_TREE_DEPTH: u16 = 5;

#[repr(C, packed)]
pub struct Ext4Superblock {
    pub s_inodes_count: u32,
    pub s_blocks_count_lo: u32,
    pub s_r_blocks_count_lo: u32,
    pub s_free_blocks_count_lo: u32,
    pub s_free_inodes_count: u32,
    pub s_first_data_block: u32,
    pub s_log_block_size: u32,
    pub s_log_cluster_size: u32,
    pub s_blocks_per_group: u32,
    pub s_clusters_per_group: u32,
    pub s_inodes_per_group: u32,
    pub s_mtime: u32,
    pub s_wtime: u32,
    pub s_mnt_count: u16,
    pub s_max_mnt_count: u16,
    pub s_magic: u16,
    pub s_state: u16,
    pub s_errors: u16,
    pub s_minor_rev_level: u16,
    pub s_lastcheck: u32,
    pub s_checkinterval: u32,
    pub s_creator_os: u32,
    pub s_rev_level: u32,
    pub s_def_resuid: u16,
    pub s_def_resgid: u16,
    pub s_first_ino: u32,
    pub s_inode_size: u16,
    pub s_block_group_nr: u16,
    pub s_feature_compat: u32,
    pub s_feature_incompat: u32,
    pub s_feature_ro_compat: u32,
    pub s_uuid: [u8; 16],
    pub s_volume_name: [u8; 16],
    pub s_last_mounted: [u8; 64],
    pub s_algorithm_usage_bitmap: u32,
    pub s_prealloc_blocks: u8,
    pub s_prealloc_dir_blocks: u8,
    pub s_reserved_gdt_blocks: u16,
    pub s_journal_uuid: [u8; 16],
    pub s_journal_inum: u32,
    pub s_journal_dev: u32,
    pub s_last_orphan: u32,
    pub s_hash_seed: [u32; 4],
    pub s_def_hash_version: u8,
    pub s_jnl_backup_type: u8,
    pub s_desc_size: u16,
    pub s_default_mount_opts: u32,
    pub s_first_meta_bg: u32,
}

#[repr(C, packed)]
pub struct Ext4BlockGroupDescriptor {
    pub bg_block_bitmap_lo: u32,
    pub bg_inode_bitmap_lo: u32,
    pub bg_inode_table_lo: u32,
    pub bg_free_blocks_count_lo: u16,
    pub bg_free_inodes_count_lo: u16,
    pub bg_used_dirs_count_lo: u16,
    pub bg_flags: u16,
    pub bg_exclude_bitmap_lo: u32,
    pub bg_block_bitmap_csum_lo: u16,
    pub bg_inode_bitmap_csum_lo: u16,
    pub bg_itable_unused_lo: u16,
    pub bg_checksum: u16,
    pub bg_block_bitmap_hi: u32,
    pub bg_inode_bitmap_hi: u32,
    pub bg_inode_table_hi: u32,
}

#[repr(C, packed)]
pub struct Ext4Inode {
    pub i_mode: u16,
    pub i_uid: u16,
    pub i_size_lo: u32,
    pub i_atime: u32,
    pub i_ctime: u32,
    pub i_mtime: u32,
    pub i_dtime: u32,
    pub i_gid: u16,
    pub i_links_count: u16,
    pub i_blocks_lo: u32,
    pub i_flags: u32,
    pub i_osd1: u32,
    pub i_block: [u8; 60],
    pub i_generation: u32,
    pub i_file_acl_lo: u32,
    pub i_size_high: u32,
    pub i_obso_faddr: u32,
}

#[repr(C, packed)]
pub struct Ext4ExtentHeader {
    pub eh_magic: u16,
    pub eh_entries: u16,
    pub eh_max: u16,
    pub eh_depth: u16,
    pub eh_generation: u32,
}

#[repr(C, packed)]
pub struct Ext4Extent {
    pub ee_block: u32,
    pub ee_len: u16,
    pub ee_start_hi: u16,
    pub ee_start_lo: u32,
}

#[repr(C, packed)]
pub struct Ext4ExtentIdx {
    pub ei_block: u32,
    pub ei_leaf_lo: u32,
    pub ei_leaf_hi: u16,
    pub ei_unused: u16,
}

#[repr(C, packed)]
pub struct Ext4DirEntry {
    pub inode: u32,
    pub rec_len: u16,
    pub name_len: u8,
    pub file_type: u8,
}

pub struct Ext4Fs {
    pub disk: &'static dyn BlockDevice,
    pub block_size: u32,
    pub block_count: u32,
    pub inode_count: u32,
    pub inode_size: u16,
    pub inodes_per_group: u32,
    pub is_64bit: bool,
    pub bg_desc_size: u16,
    dma: IrqSafeSpinlock<DmaBuffer>,
}

impl Ext4Fs {
    pub fn new(disk: &'static dyn BlockDevice) -> Option<Self> {
        let geometry = disk.geometry().ok()?;
        if geometry.block_size == 0 || geometry.block_size as usize > PAGE_SIZE {
            return None;
        }
        let mut dma = DmaBuffer::allocate().ok()?;
        let sb_lba = 1024 / geometry.block_size as u64;
        let sb_offset = (1024 % geometry.block_size as u64) as usize;
        let bytes_needed = sb_offset.checked_add(core::mem::size_of::<Ext4Superblock>())?;
        let count = bytes_needed.div_ceil(geometry.block_size as usize);
        if count == 0 || count > u16::MAX as usize || count * geometry.block_size as usize > PAGE_SIZE {
            return None;
        }
        disk.read_blocks(sb_lba, count as u16, &mut dma).ok()?;
        crate::fs::vfs::record_disk_read(count * geometry.block_size as usize);
        let superblock_bytes = &dma.as_slice(count * geometry.block_size as usize).ok()?[sb_offset..];
        let sb = unsafe { &*(superblock_bytes.as_ptr() as *const Ext4Superblock) };
        {
            let incompat = sb.s_feature_incompat;
            let supported_incompat = 0x0000_00C2;
            let blocks_count_hi = unsafe {
                core::ptr::read_unaligned(superblock_bytes.as_ptr().add(0x150) as *const u32)
            };
            if sb.s_magic != 0xEF53
                || sb.s_log_block_size > 2
                || sb.s_inodes_per_group == 0
                || sb.s_blocks_per_group == 0
                || sb.s_inode_size < core::mem::size_of::<Ext4Inode>() as u16
                || sb.s_inode_size as u32 > (1024u32 << sb.s_log_block_size)
                || (1024u32 << sb.s_log_block_size) % sb.s_inode_size as u32 != 0
                || incompat & !supported_incompat != 0
                || incompat & 0x40 == 0
                || incompat & 0x02 == 0
                || sb.s_blocks_count_lo == 0
                || blocks_count_hi != 0
            {
                return None;
            }

            let block_size = 1024 << sb.s_log_block_size;
            let is_64bit = incompat & 0x80 != 0;
            let bg_desc_size = if is_64bit { sb.s_desc_size } else { 32 };
            let inode_size = sb.s_inode_size;
            let inodes_per_group = sb.s_inodes_per_group;
            let block_count = sb.s_blocks_count_lo;
            let inode_count = sb.s_inodes_count;
            if (is_64bit && bg_desc_size < 64)
                || (!is_64bit && bg_desc_size != 32)
                || bg_desc_size as u32 > block_size
            {
                return None;
            }

            Some(Self {
                disk,
                block_size,
                block_count,
                inode_count,
                inode_size,
                inodes_per_group,
                is_64bit,
                bg_desc_size,
                dma: IrqSafeSpinlock::new(dma),
            })
        }
    }

    fn read_block(&self, block_num: u32) -> Option<Vec<u8>> {
        if block_num >= self.block_count || self.block_size as usize > PAGE_SIZE {
            return None;
        }
        let geometry = self.disk.geometry().ok()?;
        let byte_offset = (block_num as u64).checked_mul(self.block_size as u64)?;
        let start = byte_offset / geometry.block_size as u64;
        let offset = (byte_offset % geometry.block_size as u64) as usize;
        let bytes = offset.checked_add(self.block_size as usize)?;
        let count = bytes.div_ceil(geometry.block_size as usize);
        if count == 0 || count > u16::MAX as usize || count * geometry.block_size as usize > PAGE_SIZE {
            return None;
        }
        let mut dma = self.dma.lock();
        self.disk.read_blocks(start, count as u16, &mut dma).ok()?;
        crate::fs::vfs::record_disk_read(count * geometry.block_size as usize);
        let source = dma.as_slice(count * geometry.block_size as usize).ok()?;
        let data = source.get(offset..offset + self.block_size as usize)?;
        let mut result = Vec::new();
        result.try_reserve_exact(data.len()).ok()?;
        result.extend_from_slice(data);
        Some(result)
    }

    fn get_inode(&self, inode_num: u32) -> Option<Vec<u8>> {
        if inode_num == 0 || inode_num > self.inode_count || self.inodes_per_group == 0 {
            return None;
        }
        let bg_num = (inode_num - 1) / self.inodes_per_group;
        let inode_index = (inode_num - 1) % self.inodes_per_group;
        
        let bgdt_block: u32 = if self.block_size == 1024 { 2 } else { 1 };
        
        let desc_size = if self.is_64bit { self.bg_desc_size as usize } else { 32 };
        let descriptors_per_block = self.block_size as usize / desc_size;
        if descriptors_per_block == 0 { return None; }
        let descriptor_block_offset = bg_num as usize / descriptors_per_block;
        let offset = (bg_num as usize % descriptors_per_block).checked_mul(desc_size)?;
        let descriptor_block = bgdt_block.checked_add(u32::try_from(descriptor_block_offset).ok()?)?;
        let bgdt_data = self.read_block(descriptor_block)?;
        let descriptor_bytes = if self.is_64bit && desc_size >= 64 { 44 } else { 12 };
        let desc_end = offset.checked_add(descriptor_bytes)?;
        let desc_bytes = bgdt_data.get(offset..desc_end)?;
        let inode_table_lo = u32::from_le_bytes(desc_bytes.get(8..12)?.try_into().ok()?);
        let inode_table_hi = if self.is_64bit && desc_size >= 64 {
            u32::from_le_bytes(desc_bytes.get(40..44)?.try_into().ok()?)
        } else { 0 };
        let inode_table_block = ((inode_table_hi as u64) << 32) | (inode_table_lo as u64);
        
        let inode_offset = inode_index as u64 * (self.inode_size as u64);
        let block_offset = inode_offset / (self.block_size as u64);
        let byte_offset_in_block = inode_offset % (self.block_size as u64);
        
        let target_block = inode_table_block.checked_add(block_offset)?;
        let target_block = u32::try_from(target_block).ok()?;
        let block_data = self.read_block(target_block)?;

        let inode_start = usize::try_from(byte_offset_in_block).ok()?;
        let inode_end = inode_start.checked_add(self.inode_size as usize)?;
        let inode_bytes = block_data.get(inode_start..inode_end)?;
        let mut inode_data = Vec::new();
        inode_data.try_reserve_exact(inode_bytes.len()).ok()?;
        inode_data.extend_from_slice(inode_bytes);
        Some(inode_data)
    }

    fn parse_extent_node(
        &self,
        node_data: &[u8],
        file_data: &mut Vec<u8>,
        max_size: usize,
        expected_depth: u16,
    ) -> Option<()> {
        if node_data.len() < core::mem::size_of::<Ext4ExtentHeader>() { return None; }
        let header = unsafe { core::ptr::read_unaligned(node_data.as_ptr() as *const Ext4ExtentHeader) };
        if header.eh_magic != 0xF30A || header.eh_depth != expected_depth || expected_depth > MAX_EXTENT_TREE_DEPTH {
            return None;
        }

        let entries_count = header.eh_entries as usize;
        let max_entries = (node_data.len() - core::mem::size_of::<Ext4ExtentHeader>()) / 12;
        if entries_count > max_entries || entries_count > header.eh_max as usize || header.eh_max as usize > max_entries {
            return None;
        }

        for index in 0..entries_count {
            if file_data.len() >= max_size { break; }
            let entry_offset = core::mem::size_of::<Ext4ExtentHeader>() + index * 12;
            let entry = node_data.get(entry_offset..entry_offset + 12)?;
            if expected_depth == 0 {
                let extent = unsafe { core::ptr::read_unaligned(entry.as_ptr() as *const Ext4Extent) };
                let logical_start = (extent.ee_block as usize).checked_mul(self.block_size as usize)?;
                if logical_start < file_data.len() { return None; }
                if logical_start >= max_size { break; }
                file_data.try_reserve(logical_start - file_data.len()).ok()?;
                file_data.resize(logical_start, 0);

                let raw_len = extent.ee_len;
                let uninitialized = raw_len > 32768;
                let block_count = if uninitialized { raw_len - 32768 } else { raw_len };
                let physical_start = ((extent.ee_start_hi as u64) << 32) | extent.ee_start_lo as u64;
                for block_offset in 0..block_count as u64 {
                    if file_data.len() >= max_size { break; }
                    let physical_block = physical_start.checked_add(block_offset)?;
                    let physical_block = u32::try_from(physical_block).ok()?;
                    let append_len = (max_size - file_data.len()).min(self.block_size as usize);
                    file_data.try_reserve(append_len).ok()?;
                    if uninitialized {
                        file_data.resize(file_data.len() + append_len, 0);
                    } else {
                        let block = self.read_block(physical_block)?;
                        file_data.extend_from_slice(block.get(..append_len)?);
                    }
                }
            } else {
                let index_entry = unsafe { core::ptr::read_unaligned(entry.as_ptr() as *const Ext4ExtentIdx) };
                let child_block = ((index_entry.ei_leaf_hi as u64) << 32) | index_entry.ei_leaf_lo as u64;
                let child_block = u32::try_from(child_block).ok()?;
                let child_data = self.read_block(child_block)?;
                self.parse_extent_node(&child_data, file_data, max_size, expected_depth - 1)?;
            }
        }
        Some(())
    }

    fn read_inode_data(&self, inode_data: &[u8]) -> Option<Vec<u8>> {
        if inode_data.len() < core::mem::size_of::<Ext4Inode>() { return None; }
        let inode = unsafe { core::ptr::read_unaligned(inode_data.as_ptr() as *const Ext4Inode) };
        let size_lo = inode.i_size_lo as u64;
        let size_hi = if inode.i_mode & 0xF000 == 0x8000 { (inode.i_size_high as u64) << 32 } else { 0 };
        let full_size = usize::try_from(size_hi | size_lo).ok()?;
        if full_size > MAX_EXT4_FILE_SIZE { return None; }

        let extent_header = unsafe { core::ptr::read_unaligned(inode.i_block.as_ptr() as *const Ext4ExtentHeader) };
        if extent_header.eh_magic != 0xF30A || extent_header.eh_depth > MAX_EXTENT_TREE_DEPTH {
            return None;
        }

        let mut file_data = Vec::new();
        file_data.try_reserve_exact(full_size).ok()?;
        self.parse_extent_node(&inode.i_block, &mut file_data, full_size, extent_header.eh_depth)?;
        if file_data.len() < full_size {
            file_data.resize(full_size, 0);
        }
        Some(file_data)
    }

    fn inode_is_directory(inode_data: &[u8]) -> bool {
        inode_data.get(..2)
            .map(|mode| u16::from_le_bytes([mode[0], mode[1]]) & 0xF000 == 0x4000)
            .unwrap_or(false)
    }

    fn directory_entry(data: &[u8], offset: usize) -> Option<(u32, usize, usize)> {
        let header_end = offset.checked_add(core::mem::size_of::<Ext4DirEntry>())?;
        let header = data.get(offset..header_end)?;
        let inode = u32::from_le_bytes(header.get(0..4)?.try_into().ok()?);
        let rec_len = u16::from_le_bytes(header.get(4..6)?.try_into().ok()?) as usize;
        let name_len = header[6] as usize;
        let entry_end = offset.checked_add(rec_len)?;
        if rec_len < 8 || rec_len % 4 != 0 || entry_end > data.len() || name_len > rec_len - 8 {
            return None;
        }
        Some((inode, rec_len, name_len))
    }
    
    fn find_in_dir(&self, dir_inode_num: u32, name: &str) -> Option<u32> {
        let inode_data = self.get_inode(dir_inode_num)?;
        if !Self::inode_is_directory(&inode_data) { return None; }
        let data = self.read_inode_data(&inode_data)?;
        
        let mut offset = 0;
        while offset < data.len() {
            let (inode, rec_len, name_len) = Self::directory_entry(&data, offset)?;
            
            if inode != 0 && name_len > 0 {
                let name_slice = data.get(offset + 8..offset + 8 + name_len)?;
                if let Ok(name_str) = core::str::from_utf8(name_slice) {
                    if name_str == name {
                        return Some(inode);
                    }
                }
            }
            
            offset = offset.checked_add(rec_len)?;
        }
        None
    }
}

impl FileSystem for Ext4Fs {
    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        let mut current_inode = 2;
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        
        for part in parts {
            if let Some(next_inode) = self.find_in_dir(current_inode, part) {
                current_inode = next_inode;
            } else {
                return None;
            }
        }
        
        let inode_data = self.get_inode(current_inode)?;
        if Self::inode_is_directory(&inode_data) { return None; }
        self.read_inode_data(&inode_data)
    }

    fn list_files(&self, path: &str) -> Option<Vec<String>> {
        let mut current_inode = 2;
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        
        for part in parts {
            if let Some(next_inode) = self.find_in_dir(current_inode, part) {
                current_inode = next_inode;
            } else {
                return None;
            }
        }
        
        let inode_data = self.get_inode(current_inode)?;
        if !Self::inode_is_directory(&inode_data) { return None; }
        let data = self.read_inode_data(&inode_data)?;
        
        let mut list = Vec::new();
        let mut offset = 0;
        while offset < data.len() {
            let (inode, rec_len, name_len) = Self::directory_entry(&data, offset)?;
            
            if inode != 0 && name_len > 0 {
                let name_slice = data.get(offset + 8..offset + 8 + name_len)?;
                if let Ok(name_str) = core::str::from_utf8(name_slice) {
                    list.push(String::from(name_str));
                }
            }
            
            offset = offset.checked_add(rec_len)?;
        }
        Some(list)
    }
}

pub static EXT4: IrqSafeSpinlock<Option<Ext4Fs>> = IrqSafeSpinlock::new(None);

pub struct GlobalExt4Fs;
impl FileSystem for GlobalExt4Fs {
    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        let mounted = EXT4.lock();
        mounted.as_ref()?.read_file(path)
    }
    fn list_files(&self, path: &str) -> Option<Vec<String>> {
        let mounted = EXT4.lock();
        mounted.as_ref()?.list_files(path)
    }
}
pub static GLOBAL_EXT4: GlobalExt4Fs = GlobalExt4Fs;

pub fn is_mounted() -> bool {
    EXT4.lock().is_some()
}

pub fn init_from_disk(disk: &'static dyn BlockDevice) {
    if let Some(fs) = Ext4Fs::new(disk) {
        crate::kprintln!("[ext4] Montado correctamente: Bloque {}, {} Inodos.", fs.block_size, fs.inodes_per_group);
        
        *EXT4.lock() = Some(fs);
        crate::fs::vfs::VFS.lock().root_fs = Some(&GLOBAL_EXT4);
        
        crate::kprintln!("[ext4] VFS root redirigido a Ext4.");
        
        crate::kprintln!("[ext4-vfs] Contenido de /: {:?}", crate::fs::vfs::list_files("/"));
    } else {
        crate::kprintln!("[ext4] Falla al inicializar Ext4.");
    }
}
