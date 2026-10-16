pub mod address;
pub mod address_space;
pub mod buddy;
pub mod heap;
pub mod pmm;
pub mod slab;
pub mod vma;
pub mod vmm;
pub mod zram;
pub fn init() {
    let _ = address::hhdm_offset();
    pmm::init();
    vmm::init();
    heap::init();
    zram::init();
}
