#![allow(unsafe_code)]
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicBool, Ordering};
struct Spinlock<T> {
    lock: AtomicBool,
    data: core::cell::UnsafeCell<T>,
}
unsafe impl<T: Send> Sync for Spinlock<T> {}
unsafe impl<T: Send> Send for Spinlock<T> {}
impl<T> Spinlock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            lock: AtomicBool::new(false),
            data: core::cell::UnsafeCell::new(data),
        }
    }
    pub fn lock(&self) -> SpinlockGuard<'_, T> {
        while self.lock.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        SpinlockGuard { lock: self }
    }
}
struct SpinlockGuard<'a, T> {
    lock: &'a Spinlock<T>,
}
impl<'a, T> core::ops::Deref for SpinlockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}
impl<'a, T> core::ops::DerefMut for SpinlockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}
impl<'a, T> Drop for SpinlockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.lock.store(false, Ordering::Release);
    }
}
#[repr(C, align(16))]
struct BlockHeader {
    size: usize,
    next: *mut BlockHeader,
    free: bool,
}
struct HeapState {
    head: *mut BlockHeader,
}
unsafe impl Send for HeapState {}
impl HeapState {
    const fn new() -> Self {
        Self {
            head: core::ptr::null_mut(),
        }
    }
    unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let size = (layout.size() + 15) & !15;
        let total_needed = size + core::mem::size_of::<BlockHeader>();
        let mut curr = self.head;
        while !curr.is_null() {
            let block = &mut *curr;
            if block.free && block.size >= total_needed {
                if block.size >= total_needed + core::mem::size_of::<BlockHeader>() + 32 {
                    let next_block_ptr = (curr as *mut u8).add(core::mem::size_of::<BlockHeader>() + size) as *mut BlockHeader;
                    next_block_ptr.write(BlockHeader {
                        size: block.size - total_needed,
                        next: block.next,
                        free: true,
                    });
                    block.size = size;
                    block.next = next_block_ptr;
                }
                block.free = false;
                return (curr as *mut u8).add(core::mem::size_of::<BlockHeader>());
            }
            curr = block.next;
        }
        let chunk_size = total_needed.max(64 * 1024);
        let mmap_res = crate::syscalls::sys_mmap(
            0,
            chunk_size as u64,
            crate::syscalls::PROT_READ | crate::syscalls::PROT_WRITE,
            crate::syscalls::MAP_ANONYMOUS,
        );
        let addr = match mmap_res {
            Ok(addr) => addr,
            Err(_) => return core::ptr::null_mut(),
        };
        let new_block = addr as *mut BlockHeader;
        new_block.write(BlockHeader {
            size: chunk_size - core::mem::size_of::<BlockHeader>(),
            next: self.head,
            free: true,
        });
        self.head = new_block;
        let block = &mut *new_block;
        if block.size >= total_needed + core::mem::size_of::<BlockHeader>() + 32 {
            let next_block_ptr = (new_block as *mut u8).add(core::mem::size_of::<BlockHeader>() + size) as *mut BlockHeader;
            next_block_ptr.write(BlockHeader {
                size: block.size - total_needed,
                next: block.next,
                free: true,
            });
            block.size = size;
            block.next = next_block_ptr;
        }
        block.free = false;
        (new_block as *mut u8).add(core::mem::size_of::<BlockHeader>())
    }
    unsafe fn dealloc(&mut self, ptr: *mut u8, _layout: Layout) {
        if ptr.is_null() {
            return;
        }
        let block_ptr = (ptr as *mut u8).sub(core::mem::size_of::<BlockHeader>()) as *mut BlockHeader;
        (*block_ptr).free = true;
        let mut curr = self.head;
        while !curr.is_null() {
            let next = (*curr).next;
            if !next.is_null() && (*curr).free && (*next).free {
                let curr_end = (curr as *mut u8).add(core::mem::size_of::<BlockHeader>() + (*curr).size);
                if curr_end == next as *mut u8 {
                    (*curr).size += core::mem::size_of::<BlockHeader>() + (*next).size;
                    (*curr).next = (*next).next;
                    continue;
                }
            }
            curr = (*curr).next;
        }
    }
}
pub struct UserAllocator {
    state: Spinlock<HeapState>,
}
unsafe impl Sync for UserAllocator {}
impl UserAllocator {
    pub const fn new() -> Self {
        Self {
            state: Spinlock::new(HeapState::new()),
        }
    }
}
unsafe impl GlobalAlloc for UserAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut guard = self.state.lock();
        guard.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let mut guard = self.state.lock();
        guard.dealloc(ptr, layout)
    }
}
