pub trait BlockWriter {
    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<(), &'static str>;
    fn read_at(&self, offset: u64, out: &mut [u8]) -> Result<(), &'static str> {
        let _ = (offset, out);
        Err("read_at not implemented for this writer")
    }
}
impl BlockWriter for alloc::vec::Vec<u8> {
    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<(), &'static str> {
        let offset = usize::try_from(offset).map_err(|_| "offset overflow")?;
        let end = offset.checked_add(bytes.len()).ok_or("range overflow")?;
        if end > self.len() {
            self.resize(end, 0);
        }
        self[offset..end].copy_from_slice(bytes);
        Ok(())
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> Result<(), &'static str> {
        let offset = usize::try_from(offset).map_err(|_| "offset overflow")?;
        let end = offset.checked_add(out.len()).ok_or("range overflow")?;
        if end > self.len() {
            return Err("read out of bounds");
        }
        out.copy_from_slice(&self[offset..end]);
        Ok(())
    }
}
