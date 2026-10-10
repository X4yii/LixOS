pub mod hw;
pub mod grid;
pub mod cursor;
pub mod ansi;

use hw::GopDisplay;
use grid::{TextCell, TextGridManager, MAX_COLS, MAX_ROWS};
use cursor::{SoftwareCursor, CURSOR_COLOR};
use ansi::AnsiDecoder;

use crate::klog::EarlyConsole;
use crate::drivers::font::{get_glyph, FONT_WIDTH, FONT_HEIGHT};
use crate::sync::IrqSafeSpinlock;

pub const ZOOM_MIN: usize = 8;
pub const ZOOM_MAX: usize = 16;

pub use ansi::{COLOR_BG_DEFAULT, COLOR_FG_DEFAULT, ANSI_PALETTE_16, ansi_256_to_rgb, AnsiConsumer};
pub const COLOR_PANIC_BG: u32   = 0x00801111;
pub const COLOR_PANIC_FG: u32   = 0x00FFFFFF;

pub struct FramebufferConsole {
    pub hw: GopDisplay,
    pub grid: TextGridManager,
    pub cursor: SoftwareCursor,
    pub ansi: AnsiDecoder,

    pub is_reversed: bool,
    pub is_bold: bool,

    pub zoom_q: usize,
    pub cols: usize,
    pub rows: usize,

    pub fg_color: u32,
    pub bg_color: u32,
    batching_write: bool,
}

impl FramebufferConsole {
    pub fn new(addr: *mut u8, width: usize, height: usize, pitch: usize, bpp: u16) -> Self {
        let cols = (width / FONT_WIDTH).min(MAX_COLS);
        let rows = (height / FONT_HEIGHT).min(MAX_ROWS);

        let mut console = Self {
            hw: GopDisplay::new(addr, width, height, pitch, bpp),
            grid: TextGridManager::new(),
            cursor: SoftwareCursor::new(),
            ansi: AnsiDecoder::new(),
            is_reversed: false,
            is_bold: false,
            zoom_q: ZOOM_MIN,
            cols,
            rows,
            fg_color: COLOR_FG_DEFAULT,
            bg_color: COLOR_BG_DEFAULT,
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
        self.cols = (self.hw.width / self.cell_width()).min(MAX_COLS);
        self.rows = (self.hw.height / self.cell_height()).min(MAX_ROWS);
    }

    pub fn erase_cursor(&mut self) {
        if self.cursor.is_drawn {
            let col = self.cursor.last_drawn_col;
            let row = self.cursor.last_drawn_row;
            if col < self.cols && row < self.rows {
                let cell = self.grid.get_cell(col, row);
                self.render_char_at(col, row, cell.ch, cell.fg, cell.bg);
            }
            self.cursor.is_drawn = false;
        }
    }

    pub fn draw_cursor(&mut self) {
        if self.batching_write {
            return;
        }
        if !self.cursor.visible {
            self.erase_cursor();
            return;
        }

        let col = self.cursor.col.min(self.cols.saturating_sub(1));
        let row = self.cursor.row.min(self.rows.saturating_sub(1));

        if self.cursor.is_drawn && (self.cursor.last_drawn_col != col || self.cursor.last_drawn_row != row) {
            self.erase_cursor();
        }

        if self.cursor.blink_state {
            let cell = self.grid.get_cell(col, row);
            let cursor_fg = cell.bg;
            let cursor_bg = if cell.fg == COLOR_FG_DEFAULT || cell.fg == cell.bg {
                CURSOR_COLOR
            } else {
                cell.fg
            };
            self.render_char_at(col, row, cell.ch, cursor_fg, cursor_bg);
            self.cursor.last_drawn_col = col;
            self.cursor.last_drawn_row = row;
            self.cursor.is_drawn = true;
        } else {
            self.erase_cursor();
        }
    }

    pub fn enter_alt_screen(&mut self) {
        if !self.grid.is_alt_screen {
            self.erase_cursor();
            self.cursor.save_primary();
            self.grid.is_alt_screen = true;
            self.cursor.col = 0;
            self.cursor.row = 0;
            self.is_reversed = false;
            self.is_bold = false;
            self.grid.clear(self.bg_color);
            self.hw.clear(self.bg_color);
            self.cursor.blink_state = true;
            self.draw_cursor();
        }
    }

    pub fn leave_alt_screen(&mut self) {
        if self.grid.is_alt_screen {
            self.erase_cursor();
            self.grid.is_alt_screen = false;
            self.cursor.restore_primary(self.cols, self.rows);
            self.is_reversed = false;
            self.is_bold = false;
            self.hw.clear(COLOR_BG_DEFAULT);
            self.redraw_grid();
            self.cursor.blink_state = true;
            self.draw_cursor();
        }
    }

    pub fn redraw_grid(&mut self) {
        for r in 0..self.rows {
            for c in 0..self.cols {
                let cell = self.grid.get_cell(c, r);
                self.render_char_at(c, r, cell.ch, cell.fg, cell.bg);
            }
        }
    }

    /// Fija el zoom en cuartos de unidad (4 = 1.0x .. 16 = 4.0x).
    /// En la pantalla primaria el texto se reorganiza (reflow) a la nueva rejilla.
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
            self.grid.clear(self.bg_color);
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

        self.hw.clear(self.bg_color);
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

    fn render_char_at(&mut self, col: usize, row: usize, ch: char, fg: u32, bg: u32) {
        let cw = self.cell_width();
        let ch_h = self.cell_height();
        let start_x = col * cw;
        let start_y = row * ch_h;
        let glyph = get_glyph(ch);

        for py in 0..ch_h {
            let byte = glyph[py * FONT_HEIGHT / ch_h];
            for px in 0..cw {
                let glyph_col = px * FONT_WIDTH / cw;
                let is_set = (byte & (0x80 >> glyph_col)) != 0;
                self.hw.draw_pixel(start_x + px, start_y + py, if is_set { fg } else { bg });
            }
        }
    }

    pub fn scroll(&mut self) {
        self.erase_cursor();
        let ch_h = self.cell_height();
        self.hw.scroll_up(ch_h, self.rows * ch_h, self.bg_color);
        self.grid.scroll_up(self.rows, self.bg_color);
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
    pub fn write_char(&mut self, ch: char) {
        let mut decoder = self.ansi;
        decoder.feed(ch, self);
        self.ansi = decoder;
    }
}

impl AnsiConsumer for FramebufferConsole {
    fn print_char(&mut self, c: char) {
        if self.cursor.col >= self.cols {
            self.grid.set_wrap(self.cursor.row, true);
            self.cursor.col = 0;
            self.cursor.row += 1;
        }
        if self.cursor.row >= self.rows {
            self.scroll();
            self.cursor.row = self.rows - 1;
        }

        let (draw_fg, draw_bg) = if self.is_reversed {
            (self.bg_color, self.fg_color)
        } else {
            (self.fg_color, self.bg_color)
        };

        self.grid.set_cell(self.cursor.col, self.cursor.row, TextCell {
            ch: c,
            fg: draw_fg,
            bg: draw_bg,
        });

        self.render_char_at(self.cursor.col, self.cursor.row, c, draw_fg, draw_bg);
        self.cursor.col += 1;
    }

    fn backspace(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.cols - 1;
        }
        self.grid.set_cell(self.cursor.col, self.cursor.row, TextCell::empty(self.bg_color));
        self.render_char_at(self.cursor.col, self.cursor.row, ' ', self.fg_color, self.bg_color);
    }

    fn newline(&mut self) {
        self.grid.set_wrap(self.cursor.row, false);
        self.cursor.col = 0;
        self.cursor.row += 1;
        if self.cursor.row >= self.rows {
            self.scroll();
            self.cursor.row = self.rows - 1;
        }
    }

    fn carriage_return(&mut self) {
        self.cursor.col = 0;
    }

    fn tab(&mut self) {
        self.cursor.col = (self.cursor.col + 4) & !3;
        if self.cursor.col >= self.cols {
            self.cursor.col = 0;
            self.cursor.row += 1;
            if self.cursor.row >= self.rows {
                self.scroll();
                self.cursor.row = self.rows - 1;
            }
        }
    }

    fn set_cursor(&mut self, row: usize, col: usize) {
        self.erase_cursor();
        self.cursor.row = row.min(self.rows.saturating_sub(1));
        self.cursor.col = col.min(self.cols.saturating_sub(1));
        self.cursor.blink_state = true;
        self.draw_cursor();
    }

    fn move_cursor_up(&mut self, count: usize) {
        self.erase_cursor();
        self.cursor.row = self.cursor.row.saturating_sub(count);
        self.cursor.blink_state = true;
        self.draw_cursor();
    }

    fn move_cursor_down(&mut self, count: usize) {
        self.erase_cursor();
        self.cursor.row = (self.cursor.row + count).min(self.rows.saturating_sub(1));
        self.cursor.blink_state = true;
        self.draw_cursor();
    }

    fn move_cursor_right(&mut self, count: usize) {
        self.erase_cursor();
        self.cursor.col = (self.cursor.col + count).min(self.cols.saturating_sub(1));
        self.cursor.blink_state = true;
        self.draw_cursor();
    }

    fn move_cursor_left(&mut self, count: usize) {
        self.erase_cursor();
        self.cursor.col = self.cursor.col.saturating_sub(count);
        self.cursor.blink_state = true;
        self.draw_cursor();
    }

    fn clear_screen(&mut self) {
        self.clear_all(self.bg_color);
    }

    fn clear_line(&mut self, mode: u8) {
        self.erase_cursor();
        let start_col = if mode == 1 || mode == 2 { 0 } else { self.cursor.col };
        let end_col = if mode == 1 { self.cursor.col } else { self.cols };
        let row = self.cursor.row;
        let bg = self.bg_color;
        if mode == 2 || start_col == 0 {
            self.grid.set_wrap(row, false);
        }
        for c in start_col..end_col {
            self.grid.set_cell(c, row, TextCell::empty(bg));
            self.render_char_at(c, row, ' ', COLOR_FG_DEFAULT, bg);
        }
        self.cursor.blink_state = true;
        self.draw_cursor();
    }

    fn enter_alt_screen(&mut self) {
        self.enter_alt_screen();
    }

    fn leave_alt_screen(&mut self) {
        self.leave_alt_screen();
    }

    fn set_cursor_visible(&mut self, visible: bool) {
        self.cursor.visible = visible;
        if visible {
            self.draw_cursor();
        } else {
            self.erase_cursor();
        }
    }

    fn set_fg_color(&mut self, color: u32) {
        self.fg_color = color;
    }

    fn set_bg_color(&mut self, color: u32) {
        self.bg_color = color;
    }

    fn reset_colors(&mut self) {
        self.fg_color = COLOR_FG_DEFAULT;
        self.bg_color = COLOR_BG_DEFAULT;
    }

    fn set_bold(&mut self, bold: bool) {
        self.is_bold = bold;
    }

    fn set_reversed(&mut self, reversed: bool) {
        self.is_reversed = reversed;
    }
}

impl FramebufferConsole {

    pub fn setup_panic_screen(&mut self) {
        self.bg_color = COLOR_PANIC_BG;
        self.fg_color = COLOR_PANIC_FG;
        self.clear_all(COLOR_PANIC_BG);
        self.cursor.col = 0;
        self.cursor.row = 1;
    }
}

impl EarlyConsole for FramebufferConsole {
    fn write_str(&mut self, s: &str) {
        self.erase_cursor();
        self.batching_write = true;
        for c in s.chars() {
            self.write_char(c);
        }
        self.batching_write = false;
        self.draw_cursor();
    }
}

pub static FRAMEBUFFER_DEVICE: IrqSafeSpinlock<Option<FramebufferConsole>> = IrqSafeSpinlock::new(None);

pub struct FramebufferEarlySink;
impl EarlyConsole for FramebufferEarlySink {
    fn write_str(&mut self, s: &str) {
        if let Some(ref mut dev) = *FRAMEBUFFER_DEVICE.lock() {
            EarlyConsole::write_str(dev, s);
        }
    }
}
static mut FB_EARLY_SINK: FramebufferEarlySink = FramebufferEarlySink;

pub fn init(addr: *mut u8, width: usize, height: usize, pitch: usize, bpp: u16) -> Option<&'static mut dyn EarlyConsole> {
    *FRAMEBUFFER_DEVICE.lock() = Some(FramebufferConsole::new(addr, width, height, pitch, bpp));
    Some(unsafe { &mut *core::ptr::addr_of_mut!(FB_EARLY_SINK) })
}

pub fn emergency_panic_screen() {
    let mut dev_lock = FRAMEBUFFER_DEVICE.try_lock();
    if dev_lock.is_none() {
        unsafe { FRAMEBUFFER_DEVICE.force_unlock(); }
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

pub fn get_terminal_info() -> Option<lix_sys::TerminalInfo> {
    FRAMEBUFFER_DEVICE.lock().as_ref().map(|fb| lix_sys::TerminalInfo {
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
