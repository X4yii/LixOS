use crate::writer::BlockWriter;
use alloc::vec;
pub const EXT4_SUPERBLOCK_OFFSET: u64 = 1024;
pub const EXT4_BLOCK_SIZE: u32 = 4096;
pub const EXT4_INODE_SIZE: u16 = 256;
pub const EXT4_MAGIC: u16 = 0xEF53;
pub const EXT4_MKFS_FEATURES: &str = "extents,uninit_bg,dir_index,filetype,has_journal,^64bit,^flex_bg,^ea_inode,^metadata_csum,^metadata_csum_seed";
pub const fn ext4_mkfs_args<'a>(
    rootfs_dir: &'a str,
    offset_arg: &'a str,
    image_path: &'a str,
    blocks_arg: &'a str,
) -> [&'a str; 12] {
    [
        "-q",
        "-F",
        "-b",
        "4096",
        "-O",
        EXT4_MKFS_FEATURES,
        "-d",
        rootfs_dir,
        "-E",
        offset_arg,
        image_path,
        blocks_arg,
    ]
}
fn put_u16(buf: &mut [u8], offset: usize, val: u16) {
    buf[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
}
fn put_u32(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
}
fn put_u8(buf: &mut [u8], offset: usize, val: u8) {
    buf[offset] = val;
}
pub struct Ext4Volume {
    pub partition_offset: u64,
    pub total_blocks: u32,
    pub next_block: u32,
    pub next_inode: u32,
    pub inode_table_block: u32,
    pub root_dir_block: u32,
}
impl Ext4Volume {
    fn blk_off(&self, block: u32) -> u64 {
        self.partition_offset + (block as u64) * u64::from(EXT4_BLOCK_SIZE)
    }
    pub fn mkdir(
        &mut self,
        writer: &mut impl BlockWriter,
        parent_block: u32,
        _parent_ino: u32,
        name: &str,
    ) -> Result<(u32, u32), &'static str> {
        let ino = self.next_inode;
        self.next_inode += 1;
        let new_block = self.next_block;
        self.next_block += 1;
        let name_bytes = name.as_bytes();
        let mut buf = vec![0u8; EXT4_BLOCK_SIZE as usize];
        put_u32(&mut buf, 0, ino);
        let rec1 = 12;
        put_u16(&mut buf, 4, rec1 as u16);
        put_u8(&mut buf, 6, name_bytes.len().min(255) as u8);
        put_u8(&mut buf, 7, 0x02);
        buf[8..8 + name_bytes.len().min(255)].copy_from_slice(name_bytes);
        put_u32(&mut buf, 12, _parent_ino);
        put_u16(&mut buf, 16, (EXT4_BLOCK_SIZE as u16) - rec1 as u16);
        put_u8(&mut buf, 18, 2);
        put_u8(&mut buf, 19, 0x02);
        writer.write_at(self.blk_off(new_block), &buf)?;
        Self::append_dir_entry(writer, self.partition_offset, parent_block, ino, name)?;
        Ok((ino, new_block))
    }
    pub fn write_file(
        &mut self,
        writer: &mut impl BlockWriter,
        parent_block: u32,
        name: &str,
        content: &[u8],
        _is_exec: bool,
    ) -> Result<u32, &'static str> {
        let ino = self.next_inode;
        self.next_inode += 1;
        let new_block = self.next_block;
        self.next_block += 1;
        let mut buf = vec![0u8; EXT4_BLOCK_SIZE as usize];
        let n = content.len().min(EXT4_BLOCK_SIZE as usize);
        buf[..n].copy_from_slice(&content[..n]);
        writer.write_at(self.blk_off(new_block), &buf)?;
        Self::append_dir_entry(writer, self.partition_offset, parent_block, ino, name)?;
        Ok(ino)
    }
    fn append_dir_entry(
        writer: &mut impl BlockWriter,
        partition_offset: u64,
        parent_block: u32,
        ino: u32,
        name: &str,
    ) -> Result<(), &'static str> {
        let name_bytes = name.as_bytes();
        let off = partition_offset + (parent_block as u64) * u64::from(EXT4_BLOCK_SIZE);
        let mut buf = vec![0u8; EXT4_BLOCK_SIZE as usize];
        let _ = writer.read_at(off, &mut buf);
        let mut cursor = 0usize;
        while cursor < EXT4_BLOCK_SIZE as usize {
            let rec = u16::from_le_bytes(buf[cursor + 4..cursor + 6].try_into().unwrap_or([0, 0]))
                as usize;
            if rec == 0 || cursor + rec >= EXT4_BLOCK_SIZE as usize {
                break;
            }
            cursor += rec;
        }
        if cursor > 0 {
        }
        let entry_len = 12 + (name_bytes.len() + 3) / 4 * 4;
        let prior_len_needed = (EXT4_BLOCK_SIZE as usize) - cursor - entry_len;
        if prior_len_needed > 0 && cursor > 0 {
        }
        put_u32(&mut buf, cursor, ino);
        put_u16(&mut buf, cursor + 4, entry_len as u16);
        put_u8(&mut buf, cursor + 6, name_bytes.len() as u8);
        put_u8(&mut buf, cursor + 7, 0);
        let copy_len = name_bytes.len().min(entry_len - 8);
        buf[cursor + 8..cursor + 8 + copy_len].copy_from_slice(&name_bytes[..copy_len]);
        writer.write_at(off, &buf)?;
        Ok(())
    }
}
pub fn format_ext4(
    writer: &mut impl BlockWriter,
    partition_offset: u64,
    total_blocks_4k: u64,
    volume_name: &str,
    uuid: [u8; 16],
) -> Result<Ext4Volume, &'static str> {
    if total_blocks_4k < 1024 {
        return Err("ext4 volume is too small (minimum 4 MiB)");
    }
    let blocks_count_u32 = u32::try_from(total_blocks_4k).map_err(|_| "volume exceeds 16 TiB")?;
    let blocks_per_group: u32 = 32768;
    let inodes_per_group: u32 = 8192;
    let num_groups = blocks_count_u32.div_ceil(blocks_per_group);
    let inodes_count = num_groups * inodes_per_group;
    let inode_table_blocks =
        (inodes_per_group * u32::from(EXT4_INODE_SIZE)).div_ceil(EXT4_BLOCK_SIZE);
    let mut sb = [0u8; 1024];
    put_u32(&mut sb, 0, inodes_count);
    put_u32(&mut sb, 4, blocks_count_u32);
    put_u32(&mut sb, 8, blocks_count_u32 / 20);
    let mut bgdt = vec![0u8; (num_groups as usize * 32).max(EXT4_BLOCK_SIZE as usize)];
    let mut first_data = 1u32 + (num_groups * 32).div_ceil(EXT4_BLOCK_SIZE);
    #[derive(Clone, Copy)]
    struct G {
        bm_blk: u32,
        ino_bm: u32,
        ino_tbl: u32,
        root: u32,
        lost: u32,
    }
    let mut groups = vec![
        G {
            bm_blk: 0,
            ino_bm: 0,
            ino_tbl: 0,
            root: 0,
            lost: 0
        };
        num_groups as usize
    ];
    for g in 0..num_groups as usize {
        let bm = first_data;
        first_data += 1;
        let ino = first_data;
        first_data += 1;
        let tbl = first_data;
        first_data += inode_table_blocks;
        let (root, lost) = if g == 0 {
            let r = first_data;
            first_data += 1;
            let l = first_data;
            first_data += 1;
            (r, l)
        } else {
            (0, 0)
        };
        groups[g] = G {
            bm_blk: bm,
            ino_bm: ino,
            ino_tbl: tbl,
            root,
            lost,
        };
        let entry_off = g * 32;
        put_u32(&mut bgdt, entry_off, bm);
        put_u32(&mut bgdt, entry_off + 4, ino);
        put_u32(&mut bgdt, entry_off + 8, tbl);
        let gc = (blocks_count_u32 - (g as u32 * blocks_per_group)).min(blocks_per_group);
        put_u16(
            &mut bgdt,
            entry_off + 12,
            (gc.saturating_sub(3 + inode_table_blocks)) as u16,
        );
        put_u16(
            &mut bgdt,
            entry_off + 14,
            if g == 0 {
                (inodes_per_group - 11) as u16
            } else {
                inodes_per_group as u16
            },
        );
        put_u16(&mut bgdt, entry_off + 16, if g == 0 { 2 } else { 0 });
    }
    let journal_start = first_data;
    let journal_blocks: u32 = (blocks_count_u32 / 1024).clamp(64, 1024);
    first_data += journal_blocks;
    let used_total = first_data;
    put_u32(
        &mut sb,
        12,
        (0..num_groups)
            .map(|g| {
                let gc = (blocks_count_u32 - (g as u32 * blocks_per_group)).min(blocks_per_group);
                gc.saturating_sub(3 + inode_table_blocks)
            })
            .sum(),
    );
    put_u32(&mut sb, 16, inodes_count.saturating_sub(11));
    put_u32(&mut sb, 20, 0);
    put_u32(&mut sb, 24, 2);
    put_u32(&mut sb, 28, 2);
    put_u32(&mut sb, 32, blocks_per_group);
    put_u32(&mut sb, 36, blocks_per_group);
    put_u32(&mut sb, 40, inodes_per_group);
    put_u16(&mut sb, 56, EXT4_MAGIC);
    put_u16(&mut sb, 58, 1);
    put_u16(&mut sb, 60, 1);
    put_u16(&mut sb, 76, 1);
    put_u16(&mut sb, 84, 11);
    put_u16(&mut sb, 88, EXT4_INODE_SIZE);
    put_u32(&mut sb, 92, 0x4);
    put_u32(&mut sb, 220, 0);
    put_u32(&mut sb, 224, 8);
    put_u32(&mut sb, 96, 0x0024);
    put_u32(&mut sb, 100, 0x0042);
    put_u32(&mut sb, 104, 0x0003);
    sb[108..124].copy_from_slice(&uuid);
    let name_bytes = volume_name.as_bytes();
    let name_len = name_bytes.len().min(16);
    sb[124..124 + name_len].copy_from_slice(&name_bytes[..name_len]);
    writer.write_at(partition_offset + EXT4_SUPERBLOCK_OFFSET, &sb)?;
    let bgdt_offset = partition_offset + u64::from(EXT4_BLOCK_SIZE);
    writer.write_at(bgdt_offset, &bgdt)?;
    for g in 0..num_groups as usize {
        let group_first = g as u32 * blocks_per_group;
        let group_last = ((g + 1) as u32 * blocks_per_group).min(blocks_count_u32);
        let mut block_bitmap = vec![0u8; EXT4_BLOCK_SIZE as usize];
        for b in group_first..group_last {
            if b < used_total {
                let idx = (b - group_first) as usize;
                block_bitmap[idx / 8] |= 1 << (idx % 8);
            }
        }
        writer.write_at(
            groups[g].bm_blk as u64 * EXT4_BLOCK_SIZE as u64 + partition_offset,
            &block_bitmap,
        )?;
        let mut inode_bitmap = vec![0u8; EXT4_BLOCK_SIZE as usize];
        if g == 0 {
            for ino in 1..=11u32 {
                inode_bitmap[(ino - 1) as usize / 8] |= 1 << ((ino - 1) % 8);
            }
        }
        writer.write_at(
            partition_offset + (groups[g].ino_bm as u64) * u64::from(EXT4_BLOCK_SIZE),
            &inode_bitmap,
        )?;
    }
    let inode_table_blocks = inode_table_blocks;
    let mut inode_table = vec![0u8; (inode_table_blocks * EXT4_BLOCK_SIZE) as usize];
    let root_off = EXT4_INODE_SIZE as usize;
    put_u16(&mut inode_table, root_off, 0x41ED);
    put_u32(&mut inode_table, root_off + 4, EXT4_BLOCK_SIZE);
    put_u16(&mut inode_table, root_off + 26, 3);
    put_u32(&mut inode_table, root_off + 28, 8);
    put_u32(&mut inode_table, root_off + 52, 0);
    put_u32(&mut inode_table, root_off + 60, groups[0].root);
    let lf_off = (11 - 1) * EXT4_INODE_SIZE as usize;
    put_u16(&mut inode_table, lf_off, 0x41C0);
    put_u32(&mut inode_table, lf_off + 4, EXT4_BLOCK_SIZE);
    put_u16(&mut inode_table, lf_off + 26, 2);
    put_u32(&mut inode_table, lf_off + 28, 8);
    put_u32(&mut inode_table, lf_off + 60, groups[0].lost);
    let j_off = (8 - 1) * EXT4_INODE_SIZE as usize;
    put_u16(&mut inode_table, j_off, 0x81C0);
    put_u32(&mut inode_table, j_off + 4, journal_blocks * EXT4_BLOCK_SIZE);
    put_u16(&mut inode_table, j_off + 26, 1);
    put_u32(&mut inode_table, j_off + 28, journal_blocks * 8);
    put_u16(&mut inode_table, j_off + 40, 0xF30A);
    put_u16(&mut inode_table, j_off + 42, 1);
    put_u16(&mut inode_table, j_off + 44, 4);
    put_u16(&mut inode_table, j_off + 46, 0);
    put_u32(&mut inode_table, j_off + 48, 0);
    put_u32(&mut inode_table, j_off + 52, 0);
    put_u16(&mut inode_table, j_off + 56, journal_blocks as u16);
    put_u16(&mut inode_table, j_off + 58, 0);
    put_u32(&mut inode_table, j_off + 60, journal_start);
    writer.write_at(
        partition_offset + (groups[0].ino_tbl as u64) * u64::from(EXT4_BLOCK_SIZE),
        &inode_table,
    )?;
    let mut jsb = vec![0u8; EXT4_BLOCK_SIZE as usize];
    put_u32(&mut jsb, 0, 0xC03B_3998);
    put_u32(&mut jsb, 4, 4);
    put_u32(&mut jsb, 12, journal_blocks);
    put_u32(&mut jsb, 16, 1);
    put_u32(&mut jsb, 20, 1);
    put_u32(&mut jsb, 24, 0);
    put_u32(&mut jsb, 28, 0);
    jsb[32..48].copy_from_slice(&uuid);
    put_u32(&mut jsb, 0x40, 0x0001);
    writer.write_at(partition_offset + (journal_start as u64) * EXT4_BLOCK_SIZE as u64, &jsb)?;
    Ok(Ext4Volume {
        partition_offset,
        total_blocks: blocks_count_u32,
        next_block: used_total,
        next_inode: 12,
        inode_table_block: groups[0].ino_tbl,
        root_dir_block: groups[0].root,
    })
}
