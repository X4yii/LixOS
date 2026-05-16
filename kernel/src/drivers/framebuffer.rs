use crate::drivers::font::{get_glyph, FONT_HEIGHT, FONT_WIDTH};
use crate::klog::EarlyConsole;

pub const COLOR_BG_DEFAULT: u32 = 0x00141A29;
pub const COLOR_FG_DEFAULT: u32 = 0x00E2E8F0;
pub const COLOR_PANIC_BG: u32   = 0x003B0D0D;
pub const COLOR_PANIC_FG: u32   = 0x00FCA5A5;

const MAX_COLS: usize = 160;
const MAX_ROWS: usize = 64;

#[derive(Copy, Clone)]
struct TextCell {
    ch: char,
    fg: u32,
    bg: u32,
}

impl TextCell {
    const fn empty(bg: u32) -> Self {
        Self {
            ch: ' ',
            fg: COLOR_FG_DEFAULT,
            bg,
        }
    }
}

pub struct FramebufferConsole {
    base_addr: *mut u8,
    width: usize,
    height: usize,
    pitch: usize,
    bpp: u16,

    grid: [TextCell; MAX_COLS * MAX_ROWS],
    cols: usize,
    rows: usize,
    cursor_col: usize,
    cursor_row: usize,

    pub fg_color: u32,
    pub bg_color: u32,

    ansi_state: u8,
    ansi_param: u32,
}

unsafe impl Send for FramebufferConsole {}
unsafe impl Sync for FramebufferConsole {}

impl FramebufferConsole {
    pub fn new(base_addr: *mut u8, width: usize, height: usize, pitch: usize, bpp: u16) -> Self {
        let cols = (width / FONT_WIDTH).min(MAX_COLS);
        let rows = (height / FONT_HEIGHT).min(MAX_ROWS);

        let mut console = Self {
            base_addr,
            width,
            height,
            pitch,
            bpp,
            grid: [TextCell::empty(COLOR_BG_DEFAULT); MAX_COLS * MAX_ROWS],
            cols,
            rows,
            cursor_col: 0,
            cursor_row: 0,
            fg_color: COLOR_FG_DEFAULT,
            bg_color: COLOR_BG_DEFAULT,
            ansi_state: 0,
            ansi_param: 0,
        };

        console.clear_all(COLOR_BG_DEFAULT);
        console
    }

    #[inline(always)]
    fn draw_pixel_direct(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            let offset = y * self.pitch + x * (self.bpp as usize / 8);
            unsafe {
                let pixel = self.base_addr.add(offset) as *mut u32;
                pixel.write_volatile(color);
            }
        }
    }

    fn render_char_at(&mut self, col: usize, row: usize, ch: char, fg: u32, bg: u32) {
        let start_x = col * FONT_WIDTH;
        let start_y = row * FONT_HEIGHT;
        let glyph = get_glyph(ch);

        for (glyph_row, &byte) in glyph.iter().enumerate() {
            for glyph_col in 0..FONT_WIDTH {
                let is_set = (byte & (0x80 >> glyph_col)) != 0;
                let color = if is_set { fg } else { bg };
                self.draw_pixel_direct(start_x + glyph_col, start_y + glyph_row, color);
            }
        }
    }

    pub fn scroll(&mut self) {
        let row_bytes = FONT_HEIGHT * self.pitch;
        let copy_bytes = (self.height - FONT_HEIGHT) * self.pitch;
        
        unsafe {
            core::ptr::copy(
                self.base_addr.add(row_bytes),
                self.base_addr,
                copy_bytes,
            );
        }

        let last_y = (self.rows - 1) * FONT_HEIGHT;
        for y in last_y..self.height {
            for x in 0..self.width {
                self.draw_pixel_direct(x, y, self.bg_color);
            }
        }

        for r in 1..self.rows {
            let src_start = r * self.cols;
            let dst_start = (r - 1) * self.cols;
            for c in 0..self.cols {
                self.grid[dst_start + c] = self.grid[src_start + c];
            }
        }

        let last_row_start = (self.rows - 1) * self.cols;
        for c in 0..self.cols {
            self.grid[last_row_start + c] = TextCell::empty(self.bg_color);
        }
    }

    pub fn clear_all(&mut self, color: u32) {
        for cell in self.grid.iter_mut() {
            *cell = TextCell::empty(color);
        }
        for y in 0..self.height {
            for x in 0..self.width {
                self.draw_pixel_direct(x, y, color);
            }
        }
        self.cursor_col = 0;
        self.cursor_row = 0;
    }

    pub fn write_char(&mut self, ch: char) {
        if self.ansi_state == 1 {
            if ch == '[' {
                self.ansi_state = 2;
                self.ansi_param = 0;
            } else {
                self.ansi_state = 0;
            }
            return;
        } else if self.ansi_state == 2 {
            if ch >= '0' && ch <= '9' {
                self.ansi_param = self.ansi_param * 10 + (ch as u32 - '0' as u32);
            } else if ch == 'm' {
                match self.ansi_param {
                    0 => self.fg_color = COLOR_FG_DEFAULT,
                    30 => self.fg_color = 0x00000000,
                    31 => self.fg_color = 0x00FF5555, // Red
                    32 => self.fg_color = 0x0055FF55, // Green
                    33 => self.fg_color = 0x00FFFF55, // Yellow
                    34 => self.fg_color = 0x005555FF, // Blue
                    35 => self.fg_color = 0x00FF55FF, // Magenta
                    36 => self.fg_color = 0x0055FFFF, // Cyan
                    37 => self.fg_color = 0x00EEEEEE, // White
                    _ => {}
                }
                self.ansi_state = 0;
            } else {
                self.ansi_state = 0;
            }
            return;
        }
        if ch == '\x1b' {
            self.ansi_state = 1;
            return;
        }

        match ch {
            '\n' => {
                self.cursor_col = 0;
                self.cursor_row += 1;
            }
            '\r' => {
                self.cursor_col = 0;
            }
            '\t' => {
                self.cursor_col = (self.cursor_col + 4) & !3;
                if self.cursor_col >= self.cols {
                    self.cursor_col = 0;
                    self.cursor_row += 1;
                }
            }
            '\x08' => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                } else if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                    self.cursor_col = self.cols - 1;
                }
                
                let idx = self.cursor_row * self.cols + self.cursor_col;
                self.grid[idx] = TextCell::empty(self.bg_color);
                self.render_char_at(self.cursor_col, self.cursor_row, ' ', self.fg_color, self.bg_color);
            }
            c => {
                if self.cursor_col >= self.cols {
                    self.cursor_col = 0;
                    self.cursor_row += 1;
                }
                if self.cursor_row >= self.rows {
                    self.scroll();
                    self.cursor_row = self.rows - 1;
                }

                let idx = self.cursor_row * self.cols + self.cursor_col;
                self.grid[idx] = TextCell {
                    ch: c,
                    fg: self.fg_color,
                    bg: self.bg_color,
                };

                self.render_char_at(self.cursor_col, self.cursor_row, c, self.fg_color, self.bg_color);
                self.cursor_col += 1;
            }
        }

        if self.cursor_row >= self.rows {
            self.scroll();
            self.cursor_row = self.rows - 1;
        }
    }

    pub fn setup_panic_screen(&mut self) {
        self.bg_color = COLOR_PANIC_BG;
        self.fg_color = COLOR_PANIC_FG;
        self.clear_all(COLOR_PANIC_BG);
        self.cursor_col = 0;
        self.cursor_row = 1;
    }
}

impl EarlyConsole for FramebufferConsole {
    fn write_str(&mut self, s: &str) {
        for c in s.chars() {
            self.write_char(c);
        }
    }
}

use core::cell::UnsafeCell;

struct FbDevice(UnsafeCell<Option<FramebufferConsole>>);
unsafe impl Sync for FbDevice {}

static FRAMEBUFFER_DEVICE: FbDevice = FbDevice(UnsafeCell::new(None));

pub fn init(addr: *mut u8, width: usize, height: usize, pitch: usize, bpp: u16) -> Option<&'static mut FramebufferConsole> {
    unsafe {
        let slot = &mut *FRAMEBUFFER_DEVICE.0.get();
        *slot = Some(FramebufferConsole::new(addr, width, height, pitch, bpp));
        slot.as_mut()
    }
}

pub unsafe fn emergency_panic_screen() {
    let slot = &mut *FRAMEBUFFER_DEVICE.0.get();
    if let Some(ref mut dev) = slot {
        dev.setup_panic_screen();
    }
}

pub unsafe fn clear_screen() {
    let slot = &mut *FRAMEBUFFER_DEVICE.0.get();
    if let Some(ref mut dev) = slot {
        dev.clear_all(COLOR_BG_DEFAULT);
        dev.cursor_row = 0;
        dev.cursor_col = 0;
    }
}
