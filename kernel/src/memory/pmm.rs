use crate::boot::limine::MemoryMapEntryType;
use crate::memory::buddy::BuddyAllocator;
use crate::sync::IrqSafeSpinlock;
use core::sync::atomic::{AtomicU16, AtomicU32, AtomicU8, Ordering};
pub const PAGE_SIZE: usize = 4096;
pub const MAX_ORDER: usize = 11;
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum PageState {
    Reserved = 0,
    Free = 1,
    Allocated = 2,
    Slab = 3,
}
#[repr(C, align(8))]
pub struct Page {
    pub flags: AtomicU8,
    pub order: AtomicU8,
    pub slab_cache_idx: AtomicU8,
    pub _padding: u8,
    pub refcount: AtomicU16,
    pub inuse: AtomicU16,
    pub next_idx: AtomicU32,
    pub prev_idx: AtomicU32,
    pub freelist_offset: AtomicU32,
    pub _reserved: u32,
}
impl Page {
    pub const fn new_reserved() -> Self {
        Self {
            flags: AtomicU8::new(PageState::Reserved as u8),
            order: AtomicU8::new(0),
            slab_cache_idx: AtomicU8::new(0),
            _padding: 0,
            refcount: AtomicU16::new(0),
            inuse: AtomicU16::new(0),
            next_idx: AtomicU32::new(u32::MAX),
            prev_idx: AtomicU32::new(u32::MAX),
            freelist_offset: AtomicU32::new(u32::MAX),
            _reserved: 0,
        }
    }
    #[inline(always)]
    pub fn state(&self) -> PageState {
        match self.flags.load(Ordering::Acquire) {
            1 => PageState::Free,
            2 => PageState::Allocated,
            3 => PageState::Slab,
            _ => PageState::Reserved,
        }
    }
    #[inline(always)]
    pub fn set_state(&self, state: PageState) {
        self.flags.store(state as u8, Ordering::Release);
    }
}
use core::sync::atomic::{AtomicPtr, AtomicUsize};
static PAGES_PTR: AtomicPtr<Page> = AtomicPtr::new(core::ptr::null_mut());
static TOTAL_FRAMES: AtomicUsize = AtomicUsize::new(0);
static BUDDY: IrqSafeSpinlock<BuddyAllocator> = IrqSafeSpinlock::new(BuddyAllocator::new());
#[inline(always)]
pub fn total_frames() -> usize {
    TOTAL_FRAMES.load(Ordering::Acquire)
}
#[inline(always)]
pub fn get_page(frame: usize) -> &'static Page {
    let total = TOTAL_FRAMES.load(Ordering::Acquire);
    let ptr = PAGES_PTR.load(Ordering::Acquire);
    if frame >= total || ptr.is_null() {
        panic!(
            "get_page fuera de limites: frame={}, total={}",
            frame, total
        );
    }
    unsafe { &*ptr.add(frame) }
}
pub fn inc_ref(frame: usize) -> u16 {
    get_page(frame).refcount.fetch_add(1, Ordering::SeqCst) + 1
}
pub fn dec_ref(frame: usize) -> u16 {
    get_page(frame).refcount.fetch_sub(1, Ordering::SeqCst) - 1
}
pub fn get_ref(frame: usize) -> u16 {
    get_page(frame).refcount.load(Ordering::SeqCst)
}
pub fn init() {
    crate::kprintln!("[pmm] Inicializando PMM (Buddy + Per-CPU Page Caches)...");
    if let Some(memmap_resp) = crate::MEMORY_MAP_REQUEST.get_response() {
        let entries = memmap_resp.entries();
        let mut total_usable: u64 = 0;
        let mut max_usable_phys: u64 = 0;
        for entry in entries.iter() {
            if entry.entry_type == MemoryMapEntryType::USABLE {
                total_usable += entry.length;
                let end = entry.base + entry.length;
                if end > max_usable_phys {
                    max_usable_phys = end;
                }
            }
        }
        let total_frames = (max_usable_phys as usize + PAGE_SIZE - 1) / PAGE_SIZE;
        let meta_bytes = total_frames * core::mem::size_of::<Page>();
        let meta_frames = (meta_bytes + PAGE_SIZE - 1) / PAGE_SIZE;
        crate::kprintln!("[pmm] Memoria Usable: {} MB", total_usable / 1024 / 1024);
        crate::kprintln!(
            "[pmm] Max Phys Addr: {:#x} ({} frames)",
            max_usable_phys,
            total_frames
        );
        crate::kprintln!(
            "[pmm] Metadata Size: {} KB ({} frames)",
            meta_bytes / 1024,
            meta_frames
        );
        let hhdm_offset = crate::memory::address::hhdm_offset().get();
        let mut meta_phys = 0u64;
        for entry in entries.iter() {
            if entry.entry_type == MemoryMapEntryType::USABLE
                && entry.length >= (meta_frames * PAGE_SIZE) as u64
            {
                meta_phys = entry.base;
                break;
            }
        }
        if meta_phys == 0 {
            panic!("No se encontro memoria contigua para la tabla de paginas PMM");
        }
        let meta_virt = hhdm_offset + meta_phys;
        let meta_start_frame = (meta_phys as usize) / PAGE_SIZE;
        let meta_end_frame = meta_start_frame + meta_frames;
        crate::kprintln!("[pmm] Ubicando Page descriptors en: {:#x}", meta_virt);
        let ptr = meta_virt as *mut Page;
        unsafe {
            for i in 0..total_frames {
                core::ptr::write(ptr.add(i), Page::new_reserved());
            }
        }
        PAGES_PTR.store(ptr, Ordering::Release);
        TOTAL_FRAMES.store(total_frames, Ordering::Release);
        let mut buddy = BUDDY.lock();
        buddy.init(total_frames, total_usable);
        for entry in entries.iter() {
            if entry.entry_type == MemoryMapEntryType::USABLE {
                let entry_start = (entry.base as usize) / PAGE_SIZE;
                let entry_count = (entry.length as usize) / PAGE_SIZE;
                let entry_end = entry_start + entry_count;
                if entry_start < meta_end_frame && entry_end > meta_start_frame {
                    if entry_start < meta_start_frame {
                        buddy.add_range(entry_start, meta_start_frame - entry_start);
                    }
                    if entry_end > meta_end_frame {
                        buddy.add_range(meta_end_frame, entry_end - meta_end_frame);
                    }
                } else {
                    buddy.add_range(entry_start, entry_count);
                }
            }
        }
        crate::kprintln!(
            "[pmm] PMM Inicializado. Frames libres en Buddy: {}",
            buddy.free_frames
        );
    } else {
        panic!("El bootloader no entrego el Memory Map.");
    }
}
pub fn alloc_frame() -> Option<usize> {
    let pcp_frame = x86_64::instructions::interrupts::without_interrupts(|| {
        let percpu = crate::arch::x86_64::percpu::current_percpu();
        if percpu.page_cache.count > 0 {
            percpu.page_cache.count -= 1;
            Some(percpu.page_cache.frames[percpu.page_cache.count] as usize)
        } else {
            None
        }
    });
    if let Some(frame) = pcp_frame {
        let page = get_page(frame);
        page.set_state(PageState::Allocated);
        page.order.store(0, Ordering::Relaxed);
        page.refcount.store(1, Ordering::SeqCst);
        return Some(frame);
    }
    let mut buddy = BUDDY.lock();
    let frame = buddy.alloc_block(0)?;
    let mut batch = [0u32; crate::arch::x86_64::percpu::PCP_BATCH];
    let mut batch_count = 0;
    while batch_count < crate::arch::x86_64::percpu::PCP_BATCH.saturating_sub(1) {
        if let Some(f) = buddy.alloc_block(0) {
            batch[batch_count] = f as u32;
            batch_count += 1;
        } else {
            break;
        }
    }
    if batch_count > 0 {
        x86_64::instructions::interrupts::without_interrupts(|| {
            let percpu = crate::arch::x86_64::percpu::current_percpu();
            for i in 0..batch_count {
                if percpu.page_cache.count < crate::arch::x86_64::percpu::PCP_CAPACITY {
                    percpu.page_cache.frames[percpu.page_cache.count] = batch[i];
                    percpu.page_cache.count += 1;
                } else {
                    buddy.free_block(batch[i] as usize, 0);
                }
            }
        });
    }
    Some(frame)
}
pub fn free_frame(frame_idx: usize) {
    if frame_idx >= total_frames() {
        return;
    }
    let page = get_page(frame_idx);
    let prev = page.refcount.fetch_sub(1, Ordering::SeqCst);
    if prev > 1 {
        return;
    }
    if prev == 0 {
        page.refcount.store(0, Ordering::SeqCst);
        return;
    }
    page.set_state(PageState::Free);
    let pushed = x86_64::instructions::interrupts::without_interrupts(|| {
        let percpu = crate::arch::x86_64::percpu::current_percpu();
        if percpu.page_cache.count < crate::arch::x86_64::percpu::PCP_CAPACITY {
            percpu.page_cache.frames[percpu.page_cache.count] = frame_idx as u32;
            percpu.page_cache.count += 1;
            true
        } else {
            false
        }
    });
    if pushed {
        return;
    }
    let mut drained = [0u32; crate::arch::x86_64::percpu::PCP_BATCH];
    let drained_count = x86_64::instructions::interrupts::without_interrupts(|| {
        let percpu = crate::arch::x86_64::percpu::current_percpu();
        let count = crate::arch::x86_64::percpu::PCP_BATCH.min(percpu.page_cache.count);
        for i in 0..count {
            percpu.page_cache.count -= 1;
            drained[i] = percpu.page_cache.frames[percpu.page_cache.count];
        }
        percpu.page_cache.frames[percpu.page_cache.count] = frame_idx as u32;
        percpu.page_cache.count += 1;
        count
    });
    let mut buddy = BUDDY.lock();
    for i in 0..drained_count {
        buddy.free_block(drained[i] as usize, 0);
    }
}
pub fn alloc_order(order: usize) -> Option<usize> {
    if order == 0 {
        return alloc_frame();
    }
    if order > MAX_ORDER {
        return None;
    }
    BUDDY.lock().alloc_block(order)
}
pub fn free_order(frame: usize, order: usize) {
    if order == 0 {
        free_frame(frame);
        return;
    }
    if order > MAX_ORDER || frame >= total_frames() {
        return;
    }
    let page = get_page(frame);
    let prev = page.refcount.fetch_sub(1, Ordering::SeqCst);
    if prev > 1 {
        return;
    }
    if prev == 0 {
        page.refcount.store(0, Ordering::SeqCst);
        return;
    }
    page.set_state(PageState::Free);
    BUDDY.lock().free_block(frame, order);
}
pub fn alloc_contiguous_frames(count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    if count == 1 {
        return alloc_frame();
    }
    let mut order = 0;
    while (1 << order) < count {
        order += 1;
    }
    alloc_order(order)
}
pub fn free_contiguous_frames(start_frame: usize, count: usize) {
    if count == 0 {
        return;
    }
    if count == 1 {
        free_frame(start_frame);
        return;
    }
    let mut order = 0;
    while (1 << order) < count {
        order += 1;
    }
    free_order(start_frame, order);
}
pub fn memory_stats() -> (u64, u64) {
    let mut free_frames_count;
    let usable_bytes;
    {
        let buddy = BUDDY.lock();
        free_frames_count = buddy.free_frames;
        usable_bytes = buddy.usable_bytes;
    }
    for cpu_id in 0..crate::arch::x86_64::percpu::MAX_CPUS {
        let percpu = crate::arch::x86_64::percpu::percpu_mut(cpu_id);
        free_frames_count += percpu.page_cache.count;
    }
    (usable_bytes, free_frames_count as u64 * PAGE_SIZE as u64)
}
