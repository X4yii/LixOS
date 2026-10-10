pub const COLOR_BG_DEFAULT: u32 = 0x001A1B26;
pub const COLOR_FG_DEFAULT: u32 = 0x00C0CAF5;

pub const ANSI_PALETTE_16: [u32; 16] = [
    0x0015161E,
    0x00F7768E,
    0x009ECE6A,
    0x00E0AF68,
    0x007AA2F7,
    0x00BB9AF7,
    0x007DCFFF,
    0x00A9B1D6,
    0x00414868,
    0x00FF899D,
    0x009FE044,
    0x00FABA4A,
    0x0089DDFF,
    0x00C099FF,
    0x000DB9D7,
    0x00C0CAF5,
];

#[inline(always)]
pub fn ansi_256_to_rgb(idx: u8) -> u32 {
    if idx < 16 {
        ANSI_PALETTE_16[idx as usize]
    } else if idx < 232 {
        let n = idx - 16;
        let b = (n % 6) * 51;
        let g = ((n / 6) % 6) * 51;
        let r = (n / 36) * 51;
        ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
    } else {
        let gray = (idx - 232) * 10 + 8;
        ((gray as u32) << 16) | ((gray as u32) << 8) | (gray as u32)
    }
}

pub trait AnsiConsumer {
    fn print_char(&mut self, ch: char);
    fn backspace(&mut self);
    fn newline(&mut self);
    fn carriage_return(&mut self);
    fn tab(&mut self);
    fn set_cursor(&mut self, row: usize, col: usize);
    fn move_cursor_up(&mut self, count: usize);
    fn move_cursor_down(&mut self, count: usize);
    fn move_cursor_right(&mut self, count: usize);
    fn move_cursor_left(&mut self, count: usize);
    fn clear_screen(&mut self);
    fn clear_line(&mut self, mode: u8);
    fn enter_alt_screen(&mut self);
    fn leave_alt_screen(&mut self);
    fn set_cursor_visible(&mut self, visible: bool);
    fn set_fg_color(&mut self, color: u32);
    fn set_bg_color(&mut self, color: u32);
    fn reset_colors(&mut self);
    fn set_bold(&mut self, bold: bool);
    fn set_reversed(&mut self, reversed: bool);
}

#[derive(Clone, Copy)]
pub struct AnsiDecoder {
    pub state: u8,
    pub params: [i32; 8],
    pub param_idx: usize,
    pub is_private: bool,
}

impl AnsiDecoder {
    pub const fn new() -> Self {
        Self {
            state: 0,
            params: [0; 8],
            param_idx: 0,
            is_private: false,
        }
    }

    pub fn reset(&mut self) {
        self.state = 0;
        self.params = [0; 8];
        self.param_idx = 0;
        self.is_private = false;
    }

    pub fn feed<C: AnsiConsumer>(&mut self, ch: char, consumer: &mut C) {
        match self.state {
            0 => match ch {
                '\x1b' => {
                    self.state = 1;
                }
                '\n' => consumer.newline(),
                '\r' => consumer.carriage_return(),
                '\t' => consumer.tab(),
                '\x08' => consumer.backspace(),
                c => consumer.print_char(c),
            },
            1 => {
                if ch == '[' {
                    self.state = 2;
                    self.params = [0; 8];
                    self.param_idx = 0;
                    self.is_private = false;
                } else {
                    self.state = 0;
                }
            }
            2 => {
                if ch == '?' && self.param_idx == 0 && self.params[0] == 0 {
                    self.is_private = true;
                } else if ch.is_ascii_digit() {
                    self.params[self.param_idx] = self.params[self.param_idx] * 10 + (ch as i32 - '0' as i32);
                } else if ch == ';' {
                    if self.param_idx < 7 {
                        self.param_idx += 1;
                    }
                } else {
                    self.dispatch_csi(ch, consumer);
                    self.reset();
                }
            }
            _ => self.reset(),
        }
    }

    fn dispatch_csi<C: AnsiConsumer>(&mut self, ch: char, consumer: &mut C) {
        if self.is_private {
            match ch {
                'h' => {
                    if self.params[0] == 1049 || self.params[0] == 47 {
                        consumer.enter_alt_screen();
                    } else if self.params[0] == 25 {
                        consumer.set_cursor_visible(true);
                    }
                }
                'l' => {
                    if self.params[0] == 1049 || self.params[0] == 47 {
                        consumer.leave_alt_screen();
                    } else if self.params[0] == 25 {
                        consumer.set_cursor_visible(false);
                    }
                }
                _ => {}
            }
            return;
        }

        match ch {
            'm' => self.dispatch_sgr(consumer),
            'H' | 'f' => {
                let row = if self.params[0] > 0 { (self.params[0] - 1) as usize } else { 0 };
                let col = if self.param_idx >= 1 && self.params[1] > 0 { (self.params[1] - 1) as usize } else { 0 };
                consumer.set_cursor(row, col);
            }
            'J' => {
                if self.params[0] == 2 || self.params[0] == 3 {
                    consumer.clear_screen();
                }
            }
            'K' => {
                consumer.clear_line(self.params[0] as u8);
            }
            'A' => {
                let count = if self.params[0] > 0 { self.params[0] as usize } else { 1 };
                consumer.move_cursor_up(count);
            }
            'B' => {
                let count = if self.params[0] > 0 { self.params[0] as usize } else { 1 };
                consumer.move_cursor_down(count);
            }
            'C' => {
                let count = if self.params[0] > 0 { self.params[0] as usize } else { 1 };
                consumer.move_cursor_right(count);
            }
            'D' => {
                let count = if self.params[0] > 0 { self.params[0] as usize } else { 1 };
                consumer.move_cursor_left(count);
            }
            _ => {}
        }
    }

    fn dispatch_sgr<C: AnsiConsumer>(&mut self, consumer: &mut C) {
        let total = self.param_idx + 1;
        let mut i = 0;
        while i < total {
            match self.params[i] {
                0 => {
                    consumer.reset_colors();
                    consumer.set_bold(false);
                    consumer.set_reversed(false);
                    i += 1;
                }
                1 => {
                    consumer.set_bold(true);
                    i += 1;
                }
                7 => {
                    consumer.set_reversed(true);
                    i += 1;
                }
                27 => {
                    consumer.set_reversed(false);
                    i += 1;
                }
                30..=37 => {
                    let idx = (self.params[i] - 30) as usize;
                    consumer.set_fg_color(ANSI_PALETTE_16[idx]);
                    i += 1;
                }
                38 => {
                    if i + 4 < total && self.params[i + 1] == 2 {
                        let r = (self.params[i + 2] & 0xFF) as u32;
                        let g = (self.params[i + 3] & 0xFF) as u32;
                        let b = (self.params[i + 4] & 0xFF) as u32;
                        consumer.set_fg_color((r << 16) | (g << 8) | b);
                        i += 5;
                    } else if i + 2 < total && self.params[i + 1] == 5 {
                        let n = (self.params[i + 2] & 0xFF) as u8;
                        consumer.set_fg_color(ansi_256_to_rgb(n));
                        i += 3;
                    } else {
                        i += 1;
                    }
                }
                39 => {
                    consumer.set_fg_color(COLOR_FG_DEFAULT);
                    i += 1;
                }
                40..=47 => {
                    let idx = (self.params[i] - 40) as usize;
                    consumer.set_bg_color(ANSI_PALETTE_16[idx]);
                    i += 1;
                }
                48 => {
                    if i + 4 < total && self.params[i + 1] == 2 {
                        let r = (self.params[i + 2] & 0xFF) as u32;
                        let g = (self.params[i + 3] & 0xFF) as u32;
                        let b = (self.params[i + 4] & 0xFF) as u32;
                        consumer.set_bg_color((r << 16) | (g << 8) | b);
                        i += 5;
                    } else if i + 2 < total && self.params[i + 1] == 5 {
                        let n = (self.params[i + 2] & 0xFF) as u8;
                        consumer.set_bg_color(ansi_256_to_rgb(n));
                        i += 3;
                    } else {
                        i += 1;
                    }
                }
                49 => {
                    consumer.set_bg_color(COLOR_BG_DEFAULT);
                    i += 1;
                }
                90..=97 => {
                    let idx = (self.params[i] - 90 + 8) as usize;
                    consumer.set_fg_color(ANSI_PALETTE_16[idx]);
                    i += 1;
                }
                100..=107 => {
                    let idx = (self.params[i] - 100 + 8) as usize;
                    consumer.set_bg_color(ANSI_PALETTE_16[idx]);
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }
    }
}
