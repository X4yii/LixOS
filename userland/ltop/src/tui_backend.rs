use litui::backend::{Backend, WindowSize, ClearType};
use litui::buffer::Cell;
use litui::layout::{Size, Position};
use litui::style::{Color, Modifier};
use alloc::string::String;
use sys_libc::syscalls::{sys_write, sys_terminal_info, TerminalInfo};
use core::fmt;

pub struct LixosBackend {
    output_buf: String,
}

impl Default for LixosBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LixosBackend {
    pub fn new() -> Self {
        Self {
            output_buf: String::with_capacity(32768),
        }
    }

    fn write_color_fg(buf: &mut String, c: Color) {
        use core::fmt::Write;
        match c {
            Color::Reset => buf.push_str("\x1b[39m"),
            Color::Black => buf.push_str("\x1b[30m"),
            Color::Red => buf.push_str("\x1b[31m"),
            Color::Green => buf.push_str("\x1b[32m"),
            Color::Yellow => buf.push_str("\x1b[33m"),
            Color::Blue => buf.push_str("\x1b[34m"),
            Color::Magenta => buf.push_str("\x1b[35m"),
            Color::Cyan => buf.push_str("\x1b[36m"),
            Color::Gray => buf.push_str("\x1b[37m"),
            Color::DarkGray => buf.push_str("\x1b[90m"),
            Color::LightRed => buf.push_str("\x1b[91m"),
            Color::LightGreen => buf.push_str("\x1b[92m"),
            Color::LightYellow => buf.push_str("\x1b[93m"),
            Color::LightBlue => buf.push_str("\x1b[94m"),
            Color::LightMagenta => buf.push_str("\x1b[95m"),
            Color::LightCyan => buf.push_str("\x1b[96m"),
            Color::White => buf.push_str("\x1b[97m"),
            Color::Rgb(r, g, b) => {
                let _ = write!(buf, "\x1b[38;2;{};{};{}m", r, g, b);
            }
            Color::Indexed(n) => {
                let _ = write!(buf, "\x1b[38;5;{}m", n);
            }
        }
    }

    fn write_color_bg(buf: &mut String, c: Color) {
        use core::fmt::Write;
        match c {
            Color::Reset => buf.push_str("\x1b[49m"),
            Color::Black => buf.push_str("\x1b[40m"),
            Color::Red => buf.push_str("\x1b[41m"),
            Color::Green => buf.push_str("\x1b[42m"),
            Color::Yellow => buf.push_str("\x1b[43m"),
            Color::Blue => buf.push_str("\x1b[44m"),
            Color::Magenta => buf.push_str("\x1b[45m"),
            Color::Cyan => buf.push_str("\x1b[46m"),
            Color::Gray => buf.push_str("\x1b[47m"),
            Color::DarkGray => buf.push_str("\x1b[100m"),
            Color::LightRed => buf.push_str("\x1b[101m"),
            Color::LightGreen => buf.push_str("\x1b[102m"),
            Color::LightYellow => buf.push_str("\x1b[103m"),
            Color::LightBlue => buf.push_str("\x1b[104m"),
            Color::LightMagenta => buf.push_str("\x1b[105m"),
            Color::LightCyan => buf.push_str("\x1b[106m"),
            Color::White => buf.push_str("\x1b[107m"),
            Color::Rgb(r, g, b) => {
                let _ = write!(buf, "\x1b[48;2;{};{};{}m", r, g, b);
            }
            Color::Indexed(n) => {
                let _ = write!(buf, "\x1b[48;5;{}m", n);
            }
        }
    }

    fn write_modifier(buf: &mut String, m: Modifier, prev_m: Modifier) {
        if m.contains(Modifier::BOLD) && !prev_m.contains(Modifier::BOLD) {
            buf.push_str("\x1b[1m");
        } else if !m.contains(Modifier::BOLD) && prev_m.contains(Modifier::BOLD) {
            buf.push_str("\x1b[22m");
        }
        if m.contains(Modifier::REVERSED) && !prev_m.contains(Modifier::REVERSED) {
            buf.push_str("\x1b[7m");
        } else if !m.contains(Modifier::REVERSED) && prev_m.contains(Modifier::REVERSED) {
            buf.push_str("\x1b[27m");
        }
    }
}

impl Backend for LixosBackend {
    type Error = fmt::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        use core::fmt::Write;
        let mut cur_fg = Color::Reset;
        let mut cur_bg = Color::Reset;
        let mut cur_mod = Modifier::empty();
        let mut next_position: Option<(u16, u16)> = None;

        for (x, y, cell) in content {
            if next_position != Some((x, y)) {
                let _ = write!(self.output_buf, "\x1b[{};{}H", y + 1, x + 1);
            }

            if cell.modifier != cur_mod {
                Self::write_modifier(&mut self.output_buf, cell.modifier, cur_mod);
                cur_mod = cell.modifier;
            }

            if cell.fg != cur_fg {
                Self::write_color_fg(&mut self.output_buf, cell.fg);
                cur_fg = cell.fg;
            }
            if cell.bg != cur_bg {
                Self::write_color_bg(&mut self.output_buf, cell.bg);
                cur_bg = cell.bg;
            }

            self.output_buf.push_str(cell.symbol());
            next_position = Some((x.saturating_add(1), y));
        }

        if cur_mod != Modifier::empty() {
            self.output_buf.push_str("\x1b[0m");
        }

        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.output_buf.push_str("\x1b[?25l");
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.output_buf.push_str("\x1b[?25h");
        Ok(())
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        Ok(Position { x: 0, y: 0 })
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, pos: P) -> Result<(), Self::Error> {
        let p = pos.into();
        use core::fmt::Write;
        let _ = write!(self.output_buf, "\x1b[{};{}H", p.y + 1, p.x + 1);
        Ok(())
    }

    fn get_cursor(&mut self) -> Result<(u16, u16), Self::Error> {
        Ok((0, 0))
    }

    fn set_cursor(&mut self, x: u16, y: u16) -> Result<(), Self::Error> {
        use core::fmt::Write;
        let _ = write!(self.output_buf, "\x1b[{};{}H", y + 1, x + 1);
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.output_buf.push_str("\x1b[0m\x1b[2J\x1b[1;1H");
        Ok(())
    }

    fn clear_region(&mut self, _clear_type: ClearType) -> Result<(), Self::Error> {
        Ok(())
    }

    fn size(&self) -> Result<Size, Self::Error> {
        let mut info = TerminalInfo::default();
        if sys_terminal_info(&mut info) == 0 && info.cols > 0 && info.rows > 0 {
            Ok(Size { width: info.cols, height: info.rows })
        } else {
            Ok(Size { width: 80, height: 25 })
        }
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        let s = self.size()?;
        Ok(WindowSize {
            columns_rows: s,
            pixels: Size { width: 0, height: 0 },
        })
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        if !self.output_buf.is_empty() {
            sys_write(&self.output_buf);
            self.output_buf.clear();
        }
        Ok(())
    }
}
