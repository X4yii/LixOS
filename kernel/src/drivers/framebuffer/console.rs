use super::ansi::{AnsiConsumer, COLOR_FG_DEFAULT};
use super::grid::TextCell;
use super::FramebufferConsole;
impl FramebufferConsole {
    pub fn write_char(&mut self, ch: char) {
        if self.grid.reset_scroll_offset() {
            self.redraw_grid();
            self.draw_cursor();
        }
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
        let bottom_margin = if self.grid.is_alt_screen { 0 } else { 1 };
        let limit = self.rows.saturating_sub(bottom_margin);
        if self.cursor.row >= limit {
            self.scroll();
            self.cursor.row = limit.saturating_sub(1);
        }
        let (draw_fg, draw_bg) = (self.theme.draw_fg(), self.theme.draw_bg());
        self.grid.set_cell(
            self.cursor.col,
            self.cursor.row,
            TextCell {
                ch: c,
                fg: draw_fg,
                bg: draw_bg,
            },
        );
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
        self.grid.set_cell(
            self.cursor.col,
            self.cursor.row,
            TextCell::empty(self.theme.bg),
        );
        self.render_char_at(
            self.cursor.col,
            self.cursor.row,
            ' ',
            self.theme.fg,
            self.theme.bg,
        );
    }
    fn newline(&mut self) {
        self.grid.set_wrap(self.cursor.row, false);
        self.cursor.col = 0;
        self.cursor.row += 1;
        let bottom_margin = if self.grid.is_alt_screen { 0 } else { 1 };
        let limit = self.rows.saturating_sub(bottom_margin);
        if self.cursor.row >= limit {
            self.scroll();
            self.cursor.row = limit.saturating_sub(1);
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
            let bottom_margin = if self.grid.is_alt_screen { 0 } else { 1 };
            let limit = self.rows.saturating_sub(bottom_margin);
            if self.cursor.row >= limit {
                self.scroll();
                self.cursor.row = limit.saturating_sub(1);
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
        self.clear_all(self.theme.bg);
    }
    fn clear_line(&mut self, mode: u8) {
        self.erase_cursor();
        let start_col = if mode == 1 || mode == 2 {
            0
        } else {
            self.cursor.col
        };
        let end_col = if mode == 1 {
            self.cursor.col
        } else {
            self.cols
        };
        let row = self.cursor.row;
        let bg = self.theme.bg;
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
        self.theme.fg = color;
    }
    fn set_bg_color(&mut self, color: u32) {
        self.theme.bg = color;
    }
    fn reset_colors(&mut self) {
        self.theme.reset();
    }
    fn set_bold(&mut self, bold: bool) {
        self.theme.bold = bold;
    }
    fn set_reversed(&mut self, reversed: bool) {
        self.theme.reversed = reversed;
    }
}
