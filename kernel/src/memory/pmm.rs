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

        let hhdm_offset = crate::HHDM_REQUEST.get_response().map(|r| r.offset()).unwrap_or(0xffff800000000000);
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

struct BitmapAllocator {
    bitmap: &'static mut [u8],
    total_frames: usize,
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
}