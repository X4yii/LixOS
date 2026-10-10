pub struct GopDisplay {
    pub base_addr: *mut u8,
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bpp: u16,
}

unsafe impl Send for GopDisplay {}
unsafe impl Sync for GopDisplay {}

impl GopDisplay {
    pub fn new(base_addr: *mut u8, width: usize, height: usize, pitch: usize, bpp: u16) -> Self {
        Self {
            base_addr,
            width,
            height,
            pitch,
            bpp,
        }
    }

    #[inline(always)]
    pub fn draw_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            let offset = y * self.pitch + x * (self.bpp as usize / 8);
            unsafe {
                let pixel = self.base_addr.add(offset) as *mut u32;
                pixel.write_volatile(color);
            }
        }
    }

    pub fn clear(&mut self, color: u32) {
        let bytes_per_pixel = (self.bpp as usize / 8).max(1);
        let pixels_per_row = self.pitch / bytes_per_pixel;
        let total_pixels = pixels_per_row * self.height;
        unsafe {
            let ptr = self.base_addr as *mut u32;
            for i in 0..total_pixels {
                ptr.add(i).write_volatile(color);
            }
        }
    }

    pub fn scroll_up(&mut self, pixel_height: usize, region_height: usize, fill_color: u32) {
        let region = region_height.min(self.height);
        if pixel_height >= region {
            return;
        }
        let row_bytes = pixel_height * self.pitch;
        let copy_bytes = (region - pixel_height) * self.pitch;

        unsafe {
            core::ptr::copy(
                self.base_addr.add(row_bytes),
                self.base_addr,
                copy_bytes,
            );
            let clear_bytes = pixel_height * self.pitch;
            let ptr = self.base_addr.add(copy_bytes) as *mut u32;
            let words = clear_bytes / 4;
            for i in 0..words {
                ptr.add(i).write_volatile(fill_color);
            }
        }
    }
}
