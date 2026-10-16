use super::ansi::{COLOR_BG_DEFAULT, COLOR_FG_DEFAULT};
pub const COLOR_PANIC_BG: u32 = 0x00801111;
pub const COLOR_PANIC_FG: u32 = 0x00FFFFFF;
pub struct TextAttrs {
    pub fg: u32,
    pub bg: u32,
    pub bold: bool,
    pub reversed: bool,
}
impl TextAttrs {
    pub const fn new() -> Self {
        Self {
            fg: COLOR_FG_DEFAULT,
            bg: COLOR_BG_DEFAULT,
            bold: false,
            reversed: false,
        }
    }
    #[inline(always)]
    pub fn draw_fg(&self) -> u32 {
        if self.reversed {
            self.bg
        } else {
            self.fg
        }
    }
    #[inline(always)]
    pub fn draw_bg(&self) -> u32 {
        if self.reversed {
            self.fg
        } else {
            self.bg
        }
    }
    pub fn reset(&mut self) {
        self.fg = COLOR_FG_DEFAULT;
        self.bg = COLOR_BG_DEFAULT;
    }
}
