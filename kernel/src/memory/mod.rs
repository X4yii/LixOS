pub mod pmm;
pub mod vmm;
pub mod heap;
pub mod buddy;
pub mod slab;
pub mod address_space;
pub mod address;
pub mod vma;

pub fn init() {
    let _ = address::hhdm_offset();
    pmm::init();
    vmm::init();
    heap::init();
}
