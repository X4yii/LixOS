use crate::memory::pmm::{self, PageState, PAGE_SIZE};
use core::ptr::NonNull;
use core::sync::atomic::Ordering;
pub struct SlabCache {
    pub object_size: usize,
    pub cache_idx: usize,
    pub partial_head: u32,
    pub allocated_pages: usize,
    pub active_objects: usize,
}
impl SlabCache {
    pub const fn new(size: usize, idx: usize) -> Self {
        Self {
            object_size: size,
            cache_idx: idx,
            partial_head: u32::MAX,
            allocated_pages: 0,
            active_objects: 0,
        }
    }
    pub fn allocate(&mut self) -> Option<NonNull<u8>> {
        let hhdm = crate::memory::address::hhdm_offset().get();
        if self.partial_head != u32::MAX {
            let frame = self.partial_head as usize;
            let page = pmm::get_page(frame);
            let offset = page.freelist_offset.load(Ordering::Acquire);
            if offset != u32::MAX {
                let obj_virt = (hhdm + (frame * PAGE_SIZE) as u64 + offset as u64) as *mut u8;
                let next_offset = unsafe { *(obj_virt as *const u32) };
                page.freelist_offset.store(next_offset, Ordering::Release);
                page.inuse.fetch_add(1, Ordering::SeqCst);
                self.active_objects += 1;
                if next_offset == u32::MAX {
                    self.remove_partial(frame);
                }
                return NonNull::new(obj_virt);
            } else {
                self.remove_partial(frame);
            }
        }
        let frame = pmm::alloc_frame()?;
        let page = pmm::get_page(frame);
        page.set_state(PageState::Slab);
        page.slab_cache_idx
            .store(self.cache_idx as u8, Ordering::Release);
        page.inuse.store(1, Ordering::SeqCst);
        self.allocated_pages += 1;
        self.active_objects += 1;
        let page_virt = (hhdm + (frame * PAGE_SIZE) as u64) as *mut u8;
        let objects_per_page = PAGE_SIZE / self.object_size;
        if objects_per_page > 1 {
            for i in 1..(objects_per_page - 1) {
                let cur_offset = i * self.object_size;
                let nxt_offset = (i + 1) * self.object_size;
                unsafe {
                    *(page_virt.add(cur_offset) as *mut u32) = nxt_offset as u32;
                }
            }
            let last_offset = (objects_per_page - 1) * self.object_size;
            unsafe {
                *(page_virt.add(last_offset) as *mut u32) = u32::MAX;
            }
            page.freelist_offset
                .store(self.object_size as u32, Ordering::Release);
            self.add_partial(frame);
        } else {
            page.freelist_offset.store(u32::MAX, Ordering::Release);
        }
        NonNull::new(page_virt)
    }
    pub unsafe fn deallocate(&mut self, ptr: NonNull<u8>) {
        let hhdm = crate::memory::address::hhdm_offset().get();
        let virt = ptr.as_ptr() as u64;
        if virt < hhdm {
            return;
        }
        let phys = virt - hhdm;
        let frame = (phys / PAGE_SIZE as u64) as usize;
        let offset = (phys % PAGE_SIZE as u64) as u32;
        let page = pmm::get_page(frame);
        let old_freelist = page.freelist_offset.load(Ordering::Acquire);
        *(ptr.as_ptr() as *mut u32) = old_freelist;
        page.freelist_offset.store(offset, Ordering::Release);
        let old_inuse = page.inuse.fetch_sub(1, Ordering::SeqCst);
        if self.active_objects > 0 {
            self.active_objects -= 1;
        }
        if old_inuse == 1 {
            if old_freelist != u32::MAX {
                self.remove_partial(frame);
            }
            page.freelist_offset.store(u32::MAX, Ordering::Release);
            page.set_state(PageState::Allocated);
            page.order.store(0, Ordering::Relaxed);
            if self.allocated_pages > 0 {
                self.allocated_pages -= 1;
            }
            pmm::free_frame(frame);
        } else if old_freelist == u32::MAX {
            self.add_partial(frame);
        }
    }
    fn add_partial(&mut self, frame: usize) {
        let head = self.partial_head;
        let page = pmm::get_page(frame);
        page.next_idx.store(head, Ordering::Relaxed);
        page.prev_idx.store(u32::MAX, Ordering::Relaxed);
        if head != u32::MAX {
            let head_page = pmm::get_page(head as usize);
            head_page.prev_idx.store(frame as u32, Ordering::Release);
        }
        self.partial_head = frame as u32;
    }
    fn remove_partial(&mut self, frame: usize) {
        let page = pmm::get_page(frame);
        let next = page.next_idx.load(Ordering::Acquire);
        let prev = page.prev_idx.load(Ordering::Acquire);
        if prev == u32::MAX {
            self.partial_head = next;
        } else {
            let prev_page = pmm::get_page(prev as usize);
            prev_page.next_idx.store(next, Ordering::Release);
        }
        if next != u32::MAX {
            let next_page = pmm::get_page(next as usize);
            next_page.prev_idx.store(prev, Ordering::Release);
        }
        page.next_idx.store(u32::MAX, Ordering::Relaxed);
        page.prev_idx.store(u32::MAX, Ordering::Relaxed);
    }
}
unsafe impl Send for SlabCache {}
unsafe impl Send for SlabAllocator {}
fn slab_frame(ptr: NonNull<u8>) -> Option<usize> {
    let hhdm = crate::memory::address::hhdm_offset().get();
    let virt = ptr.as_ptr() as u64;
    if virt < hhdm {
        return None;
    }
    let frame = ((virt - hhdm) / PAGE_SIZE as u64) as usize;
    if frame >= pmm::total_frames() {
        return None;
    }
    if pmm::get_page(frame).state() != PageState::Slab {
        return None;
    }
    Some(frame)
}
pub struct SlabAllocator {
    pub caches: [SlabCache; 7],
}
impl SlabAllocator {
    pub const fn new() -> Self {
        Self {
            caches: [
                SlabCache::new(32, 0),
                SlabCache::new(64, 1),
                SlabCache::new(128, 2),
                SlabCache::new(256, 3),
                SlabCache::new(512, 4),
                SlabCache::new(1024, 5),
                SlabCache::new(2048, 6),
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
    pub unsafe fn deallocate(&mut self, ptr: NonNull<u8>, _layout: core::alloc::Layout) {
        let Some(frame) = slab_frame(ptr) else {
            return;
        };
        let page = pmm::get_page(frame);
        let cache_idx = page.slab_cache_idx.load(Ordering::Acquire) as usize;
        if cache_idx >= self.caches.len() {
            return;
        }
        self.caches[cache_idx].deallocate(ptr);
    }
}
pub static SLAB: crate::sync::IrqSafeSpinlock<SlabAllocator> =
    crate::sync::IrqSafeSpinlock::new(SlabAllocator::new());
