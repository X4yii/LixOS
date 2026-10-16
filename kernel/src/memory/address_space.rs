use crate::memory::vmm;
pub struct AddressSpace {
    pml4_frame: usize,
    committed: bool,
}
impl AddressSpace {
    pub fn new() -> Result<Self, &'static str> {
        let frame = unsafe { vmm::create_userspace_pml4() }?;
        Ok(Self {
            pml4_frame: frame,
            committed: false,
        })
    }
    #[inline]
    pub fn pml4_frame(&self) -> usize {
        self.pml4_frame
    }
    pub fn map_page(
        &self,
        virt_addr: u64,
        phys_addr: u64,
        user: bool,
        writable: bool,
        nx: bool,
    ) -> Result<(), &'static str> {
        unsafe { vmm::map_page(self.pml4_frame, virt_addr, phys_addr, user, writable, nx) }
    }
    pub fn commit(mut self) -> usize {
        self.committed = true;
        self.pml4_frame
    }
}
impl Drop for AddressSpace {
    fn drop(&mut self) {
        if !self.committed {
            unsafe {
                vmm::destroy_userspace_pml4(self.pml4_frame);
            }
        }
    }
}
