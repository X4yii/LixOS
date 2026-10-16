use crate::memory::pmm::{self, PageState, MAX_ORDER};
use core::sync::atomic::Ordering;
pub struct BuddyAllocator {
    pub free_lists: [u32; MAX_ORDER + 1],
    pub total_frames: usize,
    pub usable_bytes: u64,
    pub free_frames: usize,
}
impl BuddyAllocator {
    pub const fn new() -> Self {
        Self {
            free_lists: [u32::MAX; MAX_ORDER + 1],
            total_frames: 0,
            usable_bytes: 0,
            free_frames: 0,
        }
    }
    pub fn init(&mut self, total_frames: usize, usable_bytes: u64) {
        self.free_lists = [u32::MAX; MAX_ORDER + 1];
        self.total_frames = total_frames;
        self.usable_bytes = usable_bytes;
        self.free_frames = 0;
    }
    pub fn add_range(&mut self, mut start: usize, mut count: usize) {
        while count > 0 {
            let mut order = MAX_ORDER;
            while (1 << order) > count || (start & ((1 << order) - 1)) != 0 {
                order -= 1;
            }
            self.free_block(start, order);
            start += 1 << order;
            count -= 1 << order;
        }
    }
    pub fn alloc_block(&mut self, order: usize) -> Option<usize> {
        if order > MAX_ORDER {
            return None;
        }
        let mut current_order = order;
        while current_order <= MAX_ORDER && self.free_lists[current_order] == u32::MAX {
            current_order += 1;
        }
        if current_order > MAX_ORDER {
            return None;
        }
        let block = self.free_lists[current_order] as usize;
        self.remove_from_free_list(block, current_order);
        while current_order > order {
            current_order -= 1;
            let buddy = block + (1 << current_order);
            self.insert_into_free_list(buddy, current_order);
        }
        let page = pmm::get_page(block);
        page.order.store(order as u8, Ordering::Relaxed);
        page.set_state(PageState::Allocated);
        page.refcount.store(1, Ordering::SeqCst);
        self.free_frames -= 1 << order;
        Some(block)
    }
    pub fn free_block(&mut self, frame: usize, order: usize) {
        if order > MAX_ORDER || frame >= self.total_frames {
            return;
        }
        let mut current_frame = frame;
        let mut current_order = order;
        while current_order < MAX_ORDER {
            let buddy = current_frame ^ (1 << current_order);
            if buddy >= self.total_frames {
                break;
            }
            let buddy_page = pmm::get_page(buddy);
            if buddy_page.state() != PageState::Free
                || buddy_page.order.load(Ordering::Acquire) as usize != current_order
            {
                break;
            }
            self.remove_from_free_list(buddy, current_order);
            current_frame &= !(1 << current_order);
            current_order += 1;
        }
        self.insert_into_free_list(current_frame, current_order);
        self.free_frames += 1 << order;
    }
    fn remove_from_free_list(&mut self, frame: usize, order: usize) {
        let page = pmm::get_page(frame);
        let next = page.next_idx.load(Ordering::Acquire);
        let prev = page.prev_idx.load(Ordering::Acquire);
        if prev == u32::MAX {
            self.free_lists[order] = next;
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
    fn insert_into_free_list(&mut self, frame: usize, order: usize) {
        let head = self.free_lists[order];
        let page = pmm::get_page(frame);
        page.next_idx.store(head, Ordering::Relaxed);
        page.prev_idx.store(u32::MAX, Ordering::Relaxed);
        page.order.store(order as u8, Ordering::Relaxed);
        page.set_state(PageState::Free);
        if head != u32::MAX {
            let head_page = pmm::get_page(head as usize);
            head_page.prev_idx.store(frame as u32, Ordering::Release);
        }
        self.free_lists[order] = frame as u32;
    }
}
