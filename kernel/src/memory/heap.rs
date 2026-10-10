use linked_list_allocator::LockedHeap;
use core::sync::atomic::{AtomicUsize, Ordering};
use crate::memory::pmm::{self, PAGE_SIZE};
use crate::memory::vmm;

static FALLBACK_ALLOCATOR: LockedHeap = LockedHeap::empty();

pub struct LixKernelAllocator;

unsafe impl core::alloc::GlobalAlloc for LixKernelAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if crate::memory::slab::SlabAllocator::can_handle(layout) {
            if let Some(ptr) = crate::memory::slab::SLAB.lock().allocate(layout) {
                return ptr.as_ptr();
            }
        }
        core::alloc::GlobalAlloc::alloc(&FALLBACK_ALLOCATOR, layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        if (ptr as u64) < crate::memory::address::layout::KERNEL_HEAP_START {
            if let Some(non_null) = core::ptr::NonNull::new(ptr) {
                crate::memory::slab::SLAB.lock().deallocate(non_null, layout);
                return;
            }
        }
        core::alloc::GlobalAlloc::dealloc(&FALLBACK_ALLOCATOR, ptr, layout);
    }
}

#[global_allocator]
static ALLOCATOR: LixKernelAllocator = LixKernelAllocator;

pub const HEAP_START: usize = crate::memory::address::layout::KERNEL_HEAP_START as usize;
pub const INITIAL_HEAP_SIZE: usize = 8 * 1024 * 1024;
pub const MAX_HEAP_SIZE: usize = 256 * 1024 * 1024;

static CURRENT_HEAP_SIZE: AtomicUsize = AtomicUsize::new(0);

pub fn init() {
    crate::kprintln!("[heap] Inicializando Kernel Heap (SLUB O(1) + Fallback Heap)...");

    let pml4_frame = unsafe {
        let mut cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3);
        (cr3 as usize) / PAGE_SIZE
    };

    let initial_frames = INITIAL_HEAP_SIZE / PAGE_SIZE;
    for i in 0..initial_frames {
        let phys_frame = pmm::alloc_frame().expect("Sin memoria física para el heap inicial");
        let phys_addr = (phys_frame * PAGE_SIZE) as u64;
        let virt_addr = (HEAP_START + i * PAGE_SIZE) as u64;
        
        unsafe {
            vmm::map_page(pml4_frame, virt_addr, phys_addr, false, true, true)
                .expect("Fallo al mapear la página del heap");
        }
    }

    unsafe {
        FALLBACK_ALLOCATOR.lock().init(HEAP_START as *mut u8, INITIAL_HEAP_SIZE);
    }
    CURRENT_HEAP_SIZE.store(INITIAL_HEAP_SIZE, Ordering::SeqCst);

    crate::kprintln!(
        "[heap] Heap inicial de {} KB (máx {} MB) listo en {:#x}",
        INITIAL_HEAP_SIZE / 1024,
        MAX_HEAP_SIZE / (1024 * 1024),
        HEAP_START
    );
}

/// Expande el kernel heap mapeando dinámicamente nuevas páginas físicas.
/// Retorna Ok(()) si se expandió con éxito, o un error si no hay memoria física o se alcanzó MAX_HEAP_SIZE.
pub fn expand_heap(additional_bytes: usize) -> Result<(), &'static str> {
    let pml4_frame = unsafe {
        let mut cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3);
        (cr3 as usize) / PAGE_SIZE
    };

    let current = CURRENT_HEAP_SIZE.load(Ordering::SeqCst);
    let frames_needed = additional_bytes.checked_add(PAGE_SIZE - 1)
        .ok_or("Tamaño de expansión desbordado")? / PAGE_SIZE;
    if frames_needed == 0 { return Ok(()); }
    let actual_bytes = frames_needed.checked_mul(PAGE_SIZE)
        .ok_or("Tamaño de expansión desbordado")?;

    if current.checked_add(actual_bytes).ok_or("Heap desbordado")? > MAX_HEAP_SIZE {
        return Err("Heap excedió el límite máximo de memoria");
    }

    let mut mapped_frames = alloc::vec::Vec::new();
    mapped_frames.try_reserve_exact(frames_needed)
        .map_err(|_| "Sin memoria para registrar expansión del heap")?;

    for i in 0..frames_needed {
        let page_offset = i.checked_mul(PAGE_SIZE).and_then(|offset| current.checked_add(offset));
        let virt_addr = page_offset.and_then(|offset| HEAP_START.checked_add(offset));
        let Some(virt_addr) = virt_addr else {
            rollback_heap_expansion(pml4_frame, current, &mapped_frames);
            return Err("Dirección de heap desbordada");
        };
        let phys_frame = match pmm::alloc_frame() {
            Some(frame) => frame,
            None => {
                rollback_heap_expansion(pml4_frame, current, &mapped_frames);
                return Err("Sin memoria física para expandir heap");
            }
        };
        let phys_addr = (phys_frame * PAGE_SIZE) as u64;
        if let Err(error) = unsafe { vmm::map_page(pml4_frame, virt_addr as u64, phys_addr, false, true, true) } {
            pmm::free_frame(phys_frame);
            rollback_heap_expansion(pml4_frame, current, &mapped_frames);
            return Err(error);
        }
        mapped_frames.push(phys_frame);
    }

    unsafe {
        FALLBACK_ALLOCATOR.lock().extend(actual_bytes);
    }
    CURRENT_HEAP_SIZE.fetch_add(actual_bytes, Ordering::SeqCst);
    Ok(())
}

fn rollback_heap_expansion(pml4_frame: usize, initial_size: usize, mapped_frames: &[usize]) {
    for index in (0..mapped_frames.len()).rev() {
        let Some(offset) = index.checked_mul(PAGE_SIZE).and_then(|offset| initial_size.checked_add(offset)) else { continue; };
        let Some(address) = HEAP_START.checked_add(offset) else { continue; };
        let _ = unsafe { vmm::unmap_page(pml4_frame, address as u64, true) };
    }
}
