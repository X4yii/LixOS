pub trait BlockDevice: Send + Sync {
    /// Reads `count` logical sectors (usually 512 bytes) into the physical address `buf_phys`.
    fn read_sectors(&self, start_sector: u64, count: u16, buf_phys: *mut u8) -> bool;
    
    /// Reads logical blocks of `block_size` bytes.
    fn read_blocks(&self, start_block: u64, count: u16, block_size: u32, buf_phys: *mut u8) -> bool {
        let sectors_per_block = block_size / 512;
        let start_sector = if block_size == 1024 && start_block == 0 {
            // Block 0 at 1024-byte blocks is offset 1024 (sector 2)
            2
        } else {
            start_block * (sectors_per_block as u64)
        };
        self.read_sectors(start_sector, count * (sectors_per_block as u16), buf_phys)
    }
}
