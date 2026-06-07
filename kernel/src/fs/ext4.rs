use crate::drivers::ahci::PRIMARY_DISK;
use crate::memory::pmm::{alloc_frame, free_frame, PAGE_SIZE};
use crate::drivers::block::BlockDevice;
use crate::fs::vfs::FileSystem;
use alloc::vec::Vec;
use alloc::string::String;

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
pub struct Ext4DirEntry {
    pub inode: u32,
    pub rec_len: u16,
    pub name_len: u8,
    pub file_type: u8,
}

pub struct Ext4Fs {
    pub disk: &'static dyn BlockDevice,
    pub block_size: u32,
    pub inode_size: u16,
    pub inodes_per_group: u32,
    pub is_64bit: bool,
    pub bg_desc_size: u16,
}

impl Ext4Fs {
    pub fn new(disk: &'static dyn BlockDevice) -> Option<Self> {
        let phys_frame = alloc_frame()?;
        let hhdm_offset = crate::HHDM_REQUEST.get_response().map(|r| r.offset()).unwrap_or(0xffff800000000000);
        let virt_addr = hhdm_offset + (phys_frame * PAGE_SIZE) as u64;
        
        let phys_ptr = (phys_frame * PAGE_SIZE) as *mut u8;
        
        if disk.read_sectors(2, 2, phys_ptr) {
            let sb = unsafe { &*(virt_addr as *const Ext4Superblock) };
            if sb.s_magic != 0xEF53 {
                free_frame(phys_frame);
                return None;
            }
            
            let block_size = 1024 << sb.s_log_block_size;
            let is_64bit = (sb.s_feature_incompat & 0x80) != 0;
            let bg_desc_size = if is_64bit { sb.s_desc_size } else { 32 };
            let inode_size = sb.s_inode_size;
            let inodes_per_group = sb.s_inodes_per_group;
            
            free_frame(phys_frame);
            Some(Self {
                disk,
                block_size,
                inode_size,
                inodes_per_group,
                is_64bit,
                bg_desc_size,
            })
        } else {
            free_frame(phys_frame);
            None
        }
    }

    fn read_block(&self, block_num: u32) -> Option<Vec<u8>> {
        let phys_frame = alloc_frame()?;
        let hhdm_offset = crate::HHDM_REQUEST.get_response().map(|r| r.offset()).unwrap_or(0xffff800000000000);
        let virt_addr = hhdm_offset + (phys_frame * PAGE_SIZE) as u64;
        let phys_ptr = (phys_frame * PAGE_SIZE) as *mut u8;
        
        if self.disk.read_blocks(block_num as u64, 1, self.block_size, phys_ptr) {
            let slice = unsafe { core::slice::from_raw_parts(virt_addr as *const u8, self.block_size as usize) };
            let vec = slice.to_vec();
            free_frame(phys_frame);
            Some(vec)
        } else {
            free_frame(phys_frame);
            None
        }
    }

    fn get_inode(&self, inode_num: u32) -> Option<Vec<u8>> {
        let bg_num = (inode_num - 1) / self.inodes_per_group;
        let inode_index = (inode_num - 1) % self.inodes_per_group;
        
        // Find BGDT
        let bgdt_block = if self.block_size == 1024 { 2 } else { 1 };
        
        let bgdt_data = self.read_block(bgdt_block)?;
        let desc_size = if self.is_64bit { self.bg_desc_size as usize } else { 32 };
        let offset = bg_num as usize * desc_size;
        
        let bg_desc = unsafe { &*(bgdt_data.as_ptr().add(offset) as *const Ext4BlockGroupDescriptor) };
        let inode_table_lo = bg_desc.bg_inode_table_lo;
        let inode_table_hi = if self.is_64bit && desc_size >= 64 { bg_desc.bg_inode_table_hi } else { 0 };
        let inode_table_block = ((inode_table_hi as u64) << 32) | (inode_table_lo as u64);
        
        let inode_offset = inode_index as u64 * (self.inode_size as u64);
        let block_offset = inode_offset / (self.block_size as u64);
        let byte_offset_in_block = inode_offset % (self.block_size as u64);
        
        let target_block = inode_table_block as u32 + block_offset as u32;
        let block_data = self.read_block(target_block)?;
        
        let inode_data = block_data[byte_offset_in_block as usize..(byte_offset_in_block as usize + self.inode_size as usize)].to_vec();
        Some(inode_data)
    }

    fn read_inode_data(&self, inode_data: &[u8]) -> Option<Vec<u8>> {
        let inode = unsafe { &*(inode_data.as_ptr() as *const Ext4Inode) };
        let size = inode.i_size_lo;
        
        let extent_header = unsafe { &*(inode.i_block.as_ptr() as *const Ext4ExtentHeader) };
        if extent_header.eh_magic != 0xF30A {
            return None; // Non-extent files not supported right now
        }
        
        if extent_header.eh_depth == 0 && extent_header.eh_entries > 0 {
            let mut file_data = Vec::new();
            let entries_count = extent_header.eh_entries as usize;

            for i in 0..entries_count {
                let extent = unsafe { &*(inode.i_block.as_ptr().offset(12 + (i * 12) as isize) as *const Ext4Extent) };
                let phys_data_block = ((extent.ee_start_hi as u64) << 32) | (extent.ee_start_lo as u64);
                let num_blocks = extent.ee_len;

                for b in 0..num_blocks {
                    if let Some(mut block_data) = self.read_block((phys_data_block + b as u64) as u32) {
                        file_data.append(&mut block_data);
                    }
                }
            }
            
            file_data.truncate(size as usize);
            return Some(file_data);
        }
        None
    }
    
    // Finds an inode by name starting from a parent directory inode
    fn find_in_dir(&self, dir_inode_num: u32, name: &str) -> Option<u32> {
        let inode_data = self.get_inode(dir_inode_num)?;
        let data = self.read_inode_data(&inode_data)?;
        
        let mut offset = 0;
        while offset < data.len() {
            let dir_entry = unsafe { &*(data.as_ptr().add(offset) as *const Ext4DirEntry) };
            let rec_len = dir_entry.rec_len;
            let name_len = dir_entry.name_len;
            let inode = dir_entry.inode;
            
            if inode != 0 && name_len > 0 {
                let name_slice = &data[offset + 8..offset + 8 + name_len as usize];
                if let Ok(name_str) = core::str::from_utf8(name_slice) {
                    if name_str == name {
                        return Some(inode);
                    }
                }
            }
            
            if rec_len == 0 { break; }
            offset += rec_len as usize;
        }
        None
    }
}

impl FileSystem for Ext4Fs {
    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        let mut current_inode = 2; // root inode
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        
        for part in parts {
            if let Some(next_inode) = self.find_in_dir(current_inode, part) {
                current_inode = next_inode;
            } else {
                return None;
            }
        }
        
        let inode_data = self.get_inode(current_inode)?;
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
        let data = self.read_inode_data(&inode_data)?;
        
        let mut list = Vec::new();
        let mut offset = 0;
        while offset < data.len() {
            let dir_entry = unsafe { &*(data.as_ptr().add(offset) as *const Ext4DirEntry) };
            let rec_len = dir_entry.rec_len;
            let name_len = dir_entry.name_len;
            let inode = dir_entry.inode;
            
            if inode != 0 && name_len > 0 {
                let name_slice = &data[offset + 8..offset + 8 + name_len as usize];
                if let Ok(name_str) = core::str::from_utf8(name_slice) {
                    list.push(String::from(name_str));
                }
            }
            
            if rec_len == 0 { break; }
            offset += rec_len as usize;
        }
        Some(list)
    }
}

pub static mut EXT4: Option<Ext4Fs> = None;

pub fn init_from_primary_disk() {
    unsafe {
        if let Some(disk) = &PRIMARY_DISK {
            if let Some(fs) = Ext4Fs::new(disk) {
                crate::kprintln!("[ext4] Montado correctamente: Bloque {}, {} Inodos.", fs.block_size, fs.inodes_per_group);
                
                // Static allocation of the struct so we can pass a 'static reference to VFS
                EXT4 = Some(fs);
                crate::fs::vfs::VFS.lock().root_fs = EXT4.as_ref().map(|f| f as &'static dyn FileSystem);
                
                crate::kprintln!("[ext4] VFS root redirigido a Ext4.");
                
                // Let's test the new VFS integration
                let vfs = crate::fs::vfs::VFS.lock();
                if let Some(files) = vfs.root_fs.as_ref().unwrap().list_files("/") {
                    crate::kprintln!("[ext4-vfs] Contenido de /: {:?}", files);
                }
            } else {
                crate::kprintln!("[ext4] Falla al inicializar Ext4.");
            }
        }
    }
}
