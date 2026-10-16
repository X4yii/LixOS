pub mod ansi;
pub mod batch;
pub mod console;
pub mod cursor;
pub mod grid;
pub mod hw;
pub mod render;
pub mod theme;
use crate::drivers::font::{FONT_HEIGHT, FONT_WIDTH};
use crate::klog::EarlyConsole;
use crate::sync::IrqSafeSpinlock;
use ansi::AnsiDecoder;
use cursor::SoftwareCursor;
use grid::{TextGridManager, MAX_COLS, MAX_ROWS};
use hw::GopDisplay;
pub const ZOOM_MIN: usize = 8;
pub const ZOOM_MAX: usize = 16;
pub use ansi::{
    ansi_256_to_rgb, AnsiConsumer, ANSI_PALETTE_16, COLOR_BG_DEFAULT, COLOR_FG_DEFAULT,
};
pub use theme::{TextAttrs, COLOR_PANIC_BG, COLOR_PANIC_FG};
pub struct FramebufferConsole {
    pub hw: GopDisplay,
    pub grid: TextGridManager,
    pub cursor: SoftwareCursor,
    pub ansi: AnsiDecoder,
    pub theme: TextAttrs,
    pub zoom_q: usize,
    pub cols: usize,
    pub rows: usize,
    batching_write: bool,
}
impl FramebufferConsole {
    pub fn new(addr: *mut u8, width: usize, height: usize, pitch: usize, bpp: u16) -> Self {
        let cols = (width / FONT_WIDTH).min(MAX_COLS).max(1);
        let rows = (height / FONT_HEIGHT).min(MAX_ROWS).max(1);
        let mut console = Self {
            hw: GopDisplay::new(addr, width, height, pitch, bpp),
            grid: TextGridManager::new(),
            cursor: SoftwareCursor::new(),
            ansi: AnsiDecoder::new(),
            theme: TextAttrs::new(),
            zoom_q: ZOOM_MIN,
            cols,
            rows,
            batching_write: false,
        };
        console.clear_all(COLOR_BG_DEFAULT);
        console
    }
    #[inline(always)]
    pub fn cell_width(&self) -> usize {
        FONT_WIDTH * self.zoom_q / ZOOM_MIN
    }
    #[inline(always)]
    pub fn cell_height(&self) -> usize {
        FONT_HEIGHT * self.zoom_q / ZOOM_MIN
    }
    pub fn update_geometry(&mut self) {
        self.cols = (self.hw.width / self.cell_width()).min(MAX_COLS).max(1);
        self.rows = (self.hw.height / self.cell_height()).min(MAX_ROWS).max(1);
    }
    pub fn enter_alt_screen(&mut self) {
        if !self.grid.is_alt_screen {
            self.erase_cursor();
            self.cursor.save_primary();
            self.grid.is_alt_screen = true;
            self.cursor.col = 0;
            self.cursor.row = 0;
            self.theme.reversed = false;
            self.theme.bold = false;
            self.grid.clear(self.theme.bg);
            self.hw.clear(self.theme.bg);
            self.cursor.blink_state = true;
            self.draw_cursor();
        }
    }
    pub fn leave_alt_screen(&mut self) {
        if self.grid.is_alt_screen {
            self.erase_cursor();
            self.grid.is_alt_screen = false;
            self.cursor.restore_primary(self.cols, self.rows);
            self.theme.reversed = false;
            self.theme.bold = false;
            self.hw.clear(COLOR_BG_DEFAULT);
            self.redraw_grid();
            self.cursor.blink_state = true;
            self.draw_cursor();
        }
    }
    pub fn set_zoom_q(&mut self, new_q: usize) {
        let q = new_q.clamp(ZOOM_MIN, ZOOM_MAX);
        if self.zoom_q == q {
            return;
        }
        self.erase_cursor();
        let old_cols = self.cols;
        let old_rows = self.rows;
        self.zoom_q = q;
        self.update_geometry();
        if self.grid.is_alt_screen {
            self.grid.clear(self.theme.bg);
            self.cursor.col = self.cursor.col.min(self.cols.saturating_sub(1));
            self.cursor.row = self.cursor.row.min(self.rows.saturating_sub(1));
        } else {
            let (c, r) = self.grid.reflow_primary(
                old_cols,
                old_rows,
                self.cols,
                self.rows,
                self.cursor.col,
                self.cursor.row,
                COLOR_BG_DEFAULT,
            );
            self.cursor.col = c;
            self.cursor.row = r;
        }
        self.hw.clear(self.theme.bg);
        self.redraw_grid();
        self.cursor.blink_state = true;
        self.draw_cursor();
    }
    pub fn set_scale(&mut self, new_scale: usize) {
        let q = match new_scale {
            1 => ZOOM_MIN,
            2 => 10,
            3 => 12,
            4 => ZOOM_MAX,
            _ => ZOOM_MIN,
        };
        self.set_zoom_q(q);
    }
    pub fn zoom_in(&mut self) {
        self.set_zoom_q(self.zoom_q + 1);
    }
    pub fn zoom_out(&mut self) {
        self.set_zoom_q(self.zoom_q.saturating_sub(1));
    }
    pub fn zoom_reset(&mut self) {
        self.set_zoom_q(ZOOM_MIN);
    }
    pub fn scroll(&mut self) {
        self.erase_cursor();
        let ch_h = self.cell_height();
        self.hw.scroll_up(ch_h, self.rows * ch_h, self.theme.bg);
        self.grid.scroll_up(self.rows, self.theme.bg);
    }
    pub fn scroll_history_up(&mut self, lines: usize) {
        if self.grid.scroll_history_up(lines) {
            self.erase_cursor();
            self.redraw_grid();
            if self.grid.scroll_offset() == 0 {
                self.draw_cursor();
            }
        }
    }
    pub fn scroll_history_down(&mut self, lines: usize) {
        if self.grid.scroll_history_down(lines) {
            self.erase_cursor();
            self.redraw_grid();
            if self.grid.scroll_offset() == 0 {
                self.draw_cursor();
            }
        }
    }
    pub fn clear_all(&mut self, color: u32) {
        self.erase_cursor();
        self.grid.clear(color);
        self.hw.clear(color);
        self.cursor.col = 0;
        self.cursor.row = 0;
        self.cursor.blink_state = true;
        self.draw_cursor();
    }
    pub fn setup_panic_screen(&mut self) {
        self.theme.bg = COLOR_PANIC_BG;
        self.theme.fg = COLOR_PANIC_FG;
        self.clear_all(COLOR_PANIC_BG);
        self.cursor.col = 0;
        self.cursor.row = 1;
    }
}
pub static FRAMEBUFFER_DEVICE: IrqSafeSpinlock<Option<FramebufferConsole>> =
    IrqSafeSpinlock::new(None);
pub struct FramebufferEarlySink;
impl EarlyConsole for FramebufferEarlySink {
    fn write_str(&mut self, s: &str) {
        if let Some(ref mut dev) = *FRAMEBUFFER_DEVICE.lock() {
            EarlyConsole::write_str(dev, s);
        }
    }
}
static mut FB_EARLY_SINK: FramebufferEarlySink = FramebufferEarlySink;
pub fn init(
    addr: *mut u8,
    width: usize,
    height: usize,
    pitch: usize,
    bpp: u16,
) -> Option<&'static mut dyn EarlyConsole> {
    *FRAMEBUFFER_DEVICE.lock() = Some(FramebufferConsole::new(addr, width, height, pitch, bpp));
    Some(unsafe { &mut *core::ptr::addr_of_mut!(FB_EARLY_SINK) })
}
pub fn emergency_panic_screen() {
    let mut dev_lock = FRAMEBUFFER_DEVICE.try_lock();
    if dev_lock.is_none() {
        unsafe {
            FRAMEBUFFER_DEVICE.force_unlock();
        }
        dev_lock = FRAMEBUFFER_DEVICE.try_lock();
    }
    if let Some(mut guard) = dev_lock {
        if let Some(ref mut dev) = *guard {
            dev.setup_panic_screen();
        }
    }
}
pub fn cursor_blink_tick() {
    if let Some(mut guard) = FRAMEBUFFER_DEVICE.try_lock() {
        if let Some(ref mut dev) = *guard {
            dev.cursor.blink_state = !dev.cursor.blink_state;
            dev.draw_cursor();
        }
    }
}
pub fn clear_screen() {
    if let Some(ref mut dev) = *FRAMEBUFFER_DEVICE.lock() {
        dev.clear_all(COLOR_BG_DEFAULT);
        dev.cursor.row = 0;
        dev.cursor.col = 0;
    }
}
pub fn scroll_history_up(lines: usize) {
    if let Some(ref mut dev) = *FRAMEBUFFER_DEVICE.lock() {
        dev.scroll_history_up(lines);
    }
}
pub fn scroll_history_down(lines: usize) {
    if let Some(ref mut dev) = *FRAMEBUFFER_DEVICE.lock() {
        dev.scroll_history_down(lines);
    }
}
pub fn get_terminal_info() -> Option<lix_sys::TerminalInfo> {
    FRAMEBUFFER_DEVICE
        .lock()
        .as_ref()
        .map(|fb| lix_sys::TerminalInfo {
            cols: fb.cols as u16,
            rows: fb.rows as u16,
            pixel_width: fb.hw.width as u16,
            pixel_height: fb.hw.height as u16,
            font_width: fb.cell_width() as u8,
            font_height: fb.cell_height() as u8,
            scale: fb.zoom_q.div_ceil(ZOOM_MIN) as u8,
            _reserved: 0,
        })
}
pub fn set_scale(scale: usize) {
    if let Some(ref mut dev) = *FRAMEBUFFER_DEVICE.lock() {
        dev.set_scale(scale);
    }
}
pub fn zoom_in() {
    if let Some(ref mut dev) = *FRAMEBUFFER_DEVICE.lock() {
        dev.zoom_in();
    }
}
pub fn zoom_out() {
    if let Some(ref mut dev) = *FRAMEBUFFER_DEVICE.lock() {
        dev.zoom_out();
    }
}
pub fn zoom_reset() {
    if let Some(ref mut dev) = *FRAMEBUFFER_DEVICE.lock() {
        dev.zoom_reset();
    }
}
pub fn leave_alt_screen_if_active() {
    if let Some(ref mut dev) = *FRAMEBUFFER_DEVICE.lock() {
        if dev.grid.is_alt_screen {
            dev.leave_alt_screen();
        }
    }
}
