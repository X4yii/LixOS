pub mod pmm;
pub mod vmm;
pub mod heap;

pub fn init() {
    pmm::init();
    vmm::init();
    heap::init();
}
