use super::ansi::COLOR_FG_DEFAULT;
use super::cursor::CURSOR_COLOR;
use super::FramebufferConsole;
use crate::drivers::font::{get_glyph, FONT_HEIGHT, FONT_WIDTH};
impl FramebufferConsole {
    pub fn render_char_at(&mut self, col: usize, row: usize, ch: char, fg: u32, bg: u32) {
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
                self.hw
                    .draw_pixel(start_x + px, start_y + py, if is_set { fg } else { bg });
            }
        }
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
        if self.cursor.is_drawn
            && (self.cursor.last_drawn_col != col || self.cursor.last_drawn_row != row)
        {
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
    pub fn redraw_grid(&mut self) {
        for r in 0..self.rows {
            for c in 0..self.cols {
                let cell = self.grid.get_cell(c, r);
                self.render_char_at(c, r, cell.ch, cell.fg, cell.bg);
            }
        }
    }
}
