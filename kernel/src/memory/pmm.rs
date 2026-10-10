use crate::boot::limine::MemoryMapEntryType;
use crate::sync::IrqSafeSpinlock;

pub const PAGE_SIZE: usize = 4096;

static ALLOCATOR: IrqSafeSpinlock<Option<BitmapAllocator>> = IrqSafeSpinlock::new(None);


pub fn init() {
    crate::kprintln!("[pmm] Inicializando PMM...");
    if let Some(memmap_resp) = crate::MEMORY_MAP_REQUEST.get_response() {
        let entries = memmap_resp.entries();
        
        let mut total_usable: u64 = 0;
        let mut max_phys_addr: u64 = 0;

        for entry in entries.iter() {
            if entry.entry_type == MemoryMapEntryType::USABLE {
                total_usable += entry.length;
            }
            let region_end = entry.base + entry.length;
            if region_end > max_phys_addr {
                max_phys_addr = region_end;
            }
            
            crate::kprintln!(
                "[pmm] Región: {:#018x} - {:#018x} | Longitud: {} KB | Tipo: {:?}",
                entry.base,
                entry.base + entry.length,
                entry.length / 1024,
                entry.entry_type == MemoryMapEntryType::USABLE
            );
        }

        let total_frames = (max_phys_addr as usize + PAGE_SIZE - 1) / PAGE_SIZE;
        let bitmap_size = (total_frames + 7) / 8;
        let bitmap_frames = (bitmap_size + PAGE_SIZE - 1) / PAGE_SIZE;

        crate::kprintln!("[pmm] Memoria Usable: {} MB", total_usable / 1024 / 1024);
        crate::kprintln!("[pmm] Max Phys Addr: {:#x} ({} frames)", max_phys_addr, total_frames);
        crate::kprintln!("[pmm] Bitmap Size: {} KB ({} frames)", bitmap_size / 1024, bitmap_frames);

        let hhdm_offset = crate::memory::address::hhdm_offset().get();
        let mut bitmap_addr = 0;

        for entry in entries.iter() {
            if entry.entry_type == MemoryMapEntryType::USABLE && entry.length >= (bitmap_frames * PAGE_SIZE) as u64 {
                bitmap_addr = hhdm_offset + entry.base;
                break;
            }
        }

        if bitmap_addr == 0 {
            panic!("No se encontró memoria contigua suficiente para el PMM bitmap");
        }

        crate::kprintln!("[pmm] Ubicando bitmap en la dirección virtual: {:#x}", bitmap_addr);

        let bitmap_slice = unsafe {
            core::slice::from_raw_parts_mut(bitmap_addr as *mut u8, bitmap_size)
        };

        bitmap_slice.fill(0xFF);

        let mut allocator = BitmapAllocator {
            bitmap: bitmap_slice,
            total_frames,
            usable_bytes: total_usable,
            free_frames: 0,
            last_idx: 0,
        };

        for entry in entries.iter() {
            if entry.entry_type == MemoryMapEntryType::USABLE {
                let start_frame = entry.base as usize / PAGE_SIZE;
                let num_frames = entry.length as usize / PAGE_SIZE;
                allocator.free_region(start_frame, num_frames);
            }
        }

        let bitmap_phys_frame = (bitmap_addr - hhdm_offset) as usize / PAGE_SIZE;
        allocator.reserve_region(bitmap_phys_frame, bitmap_frames);

        crate::kprintln!("[pmm] PMM Inicializado. Frames libres: {}", allocator.free_frames);

        *ALLOCATOR.lock() = Some(allocator);
    } else {
        panic!("El bootloader no entregó el Memory Map.");
    }
}

pub fn alloc_frame() -> Option<usize> {
    ALLOCATOR.lock().as_mut().and_then(|a| a.alloc_frame())
}

pub fn free_frame(frame_idx: usize) {
    if let Some(a) = ALLOCATOR.lock().as_mut() {
        a.free_frame(frame_idx);
    }
}

pub fn memory_stats() -> (u64, u64) {
    ALLOCATOR.lock().as_ref().map_or((0, 0), |allocator| {
        (allocator.usable_bytes, allocator.free_frames as u64 * PAGE_SIZE as u64)
    })
}

pub fn alloc_contiguous_frames(count: usize) -> Option<usize> {
    ALLOCATOR.lock().as_mut().and_then(|a| a.alloc_contiguous_frames(count))
}

pub fn free_contiguous_frames(start_frame: usize, count: usize) {
    if let Some(a) = ALLOCATOR.lock().as_mut() {
        a.free_contiguous_frames(start_frame, count);
    }
}

struct BitmapAllocator {
    bitmap: &'static mut [u8],
    total_frames: usize,
    usable_bytes: u64,
    free_frames: usize,
    last_idx: usize,
}

impl BitmapAllocator {
    fn set_bit(&mut self, bit: usize) {
        let byte = bit / 8;
        let offset = bit % 8;
        self.bitmap[byte] |= 1 << offset;
    }

    fn clear_bit(&mut self, bit: usize) {
        let byte = bit / 8;
        let offset = bit % 8;
        self.bitmap[byte] &= !(1 << offset);
    }

    fn test_bit(&self, bit: usize) -> bool {
        let byte = bit / 8;
        let offset = bit % 8;
        (self.bitmap[byte] & (1 << offset)) != 0
    }

    fn free_region(&mut self, start_frame: usize, count: usize) {
        for i in 0..count {
            if self.test_bit(start_frame + i) {
                self.clear_bit(start_frame + i);
                self.free_frames += 1;
            }
        }
    }

    fn reserve_region(&mut self, start_frame: usize, count: usize) {
        for i in 0..count {
            if !self.test_bit(start_frame + i) {
                self.set_bit(start_frame + i);
                self.free_frames -= 1;
            }
        }
    }

    fn alloc_frame(&mut self) -> Option<usize> {
        if self.free_frames == 0 {
            return None;
        }

        for i in self.last_idx..self.total_frames {
            if !self.test_bit(i) {
                self.set_bit(i);
                self.free_frames -= 1;
                self.last_idx = i + 1;
                return Some(i);
            }
        }

        for i in 0..self.last_idx {
            if !self.test_bit(i) {
                self.set_bit(i);
                self.free_frames -= 1;
                self.last_idx = i + 1;
                return Some(i);
            }
        }

        None
    }

    fn free_frame(&mut self, frame: usize) {
        if frame < self.total_frames && self.test_bit(frame) {
            self.clear_bit(frame);
            self.free_frames += 1;
            if frame < self.last_idx {
                self.last_idx = frame;
            }
        }
    }

    fn alloc_contiguous_frames(&mut self, count: usize) -> Option<usize> {
        if count == 0 {
            return None;
        }
        if count == 1 {
            return self.alloc_frame();
        }
        if self.free_frames < count {
            return None;
        }

        let mut consecutive = 0;
        let mut start_idx = 0;

        for i in 0..self.total_frames {
            if !self.test_bit(i) {
                if consecutive == 0 {
                    start_idx = i;
                }
                consecutive += 1;
                if consecutive == count {
                    for f in start_idx..start_idx + count {
                        self.set_bit(f);
                    }
                    self.free_frames -= count;
                    return Some(start_idx);
                }
            } else {
                consecutive = 0;
            }
        }

        None
    }

    fn free_contiguous_frames(&mut self, start_frame: usize, count: usize) {
        for i in 0..count {
            self.free_frame(start_frame + i);
        }
    }
}
