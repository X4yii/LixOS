use linked_list_allocator::LockedHeap;
use crate::memory::pmm::{self, PAGE_SIZE};
use crate::memory::vmm;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

pub const HEAP_START: usize = 0xFFFF_9000_0000_0000;
pub const HEAP_SIZE: usize = 128 * 1024 * 1024; // 128 MB

pub fn init() {
    crate::kprintln!("[heap] Inicializando Kernel Heap Allocator...");

    let pml4_frame = unsafe {
        let mut cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3);
        (cr3 as usize) / PAGE_SIZE
    };

    for i in 0..(HEAP_SIZE / PAGE_SIZE) {
        let phys_frame = pmm::alloc_frame().expect("Sin memoria física para el heap");
        let phys_addr = (phys_frame * PAGE_SIZE) as u64;
        let virt_addr = (HEAP_START + i * PAGE_SIZE) as u64;
        
        unsafe {
            vmm::map_page(pml4_frame, virt_addr, phys_addr, false, true, true)
                .expect("Fallo al mapear la página del heap");
        }
    }

    unsafe {
        ALLOCATOR.lock().init(HEAP_START as *mut u8, HEAP_SIZE);
    }

    crate::kprintln!("[heap] Heap de {} KB listo en {:#x}", HEAP_SIZE / 1024, HEAP_START);
}