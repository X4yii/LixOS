use core::ptr::NonNull;
use crate::memory::pmm::{self, PAGE_SIZE};

struct FreeNode {
    next: Option<NonNull<FreeNode>>,
}

pub struct SlabCache {
    pub object_size: usize,
    free_list: Option<NonNull<FreeNode>>,
    pub allocated_pages: usize,
}

impl SlabCache {
    pub const fn new(size: usize) -> Self {
        Self {
            object_size: size,
            free_list: None,
            allocated_pages: 0,
        }
    }

    /// Asigna un objeto del tamaño del slab en O(1).
    pub fn allocate(&mut self) -> Option<NonNull<u8>> {
        if self.free_list.is_none() {
            self.grow()?;
        }

        let node = self.free_list.take()?;
        unsafe {
            self.free_list = (*node.as_ptr()).next;
            Some(node.cast::<u8>())
        }
    }

    /// Libera un objeto al slab en O(1).
    pub unsafe fn deallocate(&mut self, ptr: NonNull<u8>) {
        let node_ptr = ptr.as_ptr() as *mut FreeNode;
        (*node_ptr).next = self.free_list;
        self.free_list = Some(NonNull::new_unchecked(node_ptr));
    }

    fn grow(&mut self) -> Option<()> {
        let frame = pmm::alloc_frame()?;
        let hhdm = crate::memory::address::hhdm_offset().get();
        let page_virt = (hhdm + (frame * PAGE_SIZE) as u64) as *mut u8;

        self.allocated_pages += 1;
        let objects_per_page = PAGE_SIZE / self.object_size;

        for i in 0..objects_per_page {
            let obj_ptr = unsafe { page_virt.add(i * self.object_size) as *mut FreeNode };
            unsafe {
                (*obj_ptr).next = self.free_list;
                self.free_list = Some(NonNull::new_unchecked(obj_ptr));
            }
        }

        Some(())
    }
}

unsafe impl Send for SlabCache {}
unsafe impl Send for SlabAllocator {}

pub struct SlabAllocator {
    pub caches: [SlabCache; 7],
}

impl SlabAllocator {
    pub const fn new() -> Self {
        Self {
            caches: [
                SlabCache::new(32),
                SlabCache::new(64),
                SlabCache::new(128),
                SlabCache::new(256),
                SlabCache::new(512),
                SlabCache::new(1024),
                SlabCache::new(2048),
            ],
        }
    }

    #[inline]
    pub fn can_handle(layout: core::alloc::Layout) -> bool {
        layout.size() <= 2048 && layout.align() <= layout.size().max(32)
    }

    pub fn allocate(&mut self, layout: core::alloc::Layout) -> Option<NonNull<u8>> {
        let size = layout.size().max(layout.align()).max(32);
        for cache in self.caches.iter_mut() {
            if cache.object_size >= size {
                return cache.allocate();
            }
        }
        None
    }

    pub unsafe fn deallocate(&mut self, ptr: NonNull<u8>, layout: core::alloc::Layout) {
        let size = layout.size().max(layout.align()).max(32);
        for cache in self.caches.iter_mut() {
            if cache.object_size >= size {
                cache.deallocate(ptr);
                return;
            }
        }
    }
}

pub static SLAB: crate::sync::IrqSafeSpinlock<SlabAllocator> =
    crate::sync::IrqSafeSpinlock::new(SlabAllocator::new());
