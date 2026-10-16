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
        let bpp = match bpp {
            16 | 24 | 32 => bpp,
            _ => 32,
        };
        Self {
            base_addr,
            width,
            height,
            pitch,
            bpp,
        }
    }
    #[inline(always)]
    fn bytes_per_pixel(&self) -> usize {
        match self.bpp {
            16 => 2,
            24 => 3,
            _ => 4,
        }
    }
    #[inline(always)]
    fn store(&self, byte_offset: usize, color: u32) {
        unsafe {
            match self.bytes_per_pixel() {
                2 => (self.base_addr.add(byte_offset) as *mut u16).write_volatile(color as u16),
                3 => {
                    let pixel = self.base_addr.add(byte_offset);
                    pixel.write(color as u8);
                    pixel.add(1).write((color >> 8) as u8);
                    pixel.add(2).write((color >> 16) as u8);
                }
                _ => (self.base_addr.add(byte_offset) as *mut u32).write_volatile(color),
            }
        }
    }
    #[inline(always)]
    pub fn draw_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            let offset = y * self.pitch + x * self.bytes_per_pixel();
            self.store(offset, color);
        }
    }
    pub fn clear(&mut self, color: u32) {
        for y in 0..self.height {
            let row = y * self.pitch;
            for x in 0..self.width {
                self.store(row + x * self.bytes_per_pixel(), color);
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
            core::ptr::copy(self.base_addr.add(row_bytes), self.base_addr, copy_bytes);
        }
        let width = self.width;
        let bpp = self.bytes_per_pixel();
        for y in 0..pixel_height {
            let row = copy_bytes + y * self.pitch;
            for x in 0..width {
                self.store(row + x * bpp, fill_color);
            }
        }
    }
}
