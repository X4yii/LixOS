use litui::backend::{Backend, WindowSize, ClearType};
use litui::buffer::Cell;
use litui::layout::{Rect, Size, Position};
use alloc::string::String;
use sys_libc::syscalls::sys_write;
use core::fmt;

// LixOS Ratatui Backend
pub struct LixosBackend;

impl Backend for LixosBackend {
    type Error = fmt::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        for (x, y, cell) in content {
            let row = y + 1;
            let col = x + 1;
            let mut buf = String::new();
            buf.push_str("\x1b[");
            let mut r_str = alloc::format!("{};{}H", row, col);
            buf.push_str(&r_str);
            buf.push_str(cell.symbol());
            sys_write(&buf);
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        sys_write("\x1b[?25l");
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        sys_write("\x1b[?25h");
        Ok(())
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        Ok(Position { x: 0, y: 0 })
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, pos: P) -> Result<(), Self::Error> {
        let p = pos.into();
        let buf = alloc::format!("\x1b[{};{}H", p.y + 1, p.x + 1);
        sys_write(&buf);
        Ok(())
    }

    fn get_cursor(&mut self) -> Result<(u16, u16), Self::Error> {
        Ok((0, 0))
    }

    fn set_cursor(&mut self, x: u16, y: u16) -> Result<(), Self::Error> {
        let buf = alloc::format!("\x1b[{};{}H", y + 1, x + 1);
        sys_write(&buf);
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        sys_write("\x1b[2J\x1b[1;1H");
        Ok(())
    }

    fn clear_region(&mut self, _clear_type: ClearType) -> Result<(), Self::Error> {
        Ok(())
    }

    fn size(&self) -> Result<Size, Self::Error> {
        Ok(Size { width: 80, height: 25 })
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        Ok(WindowSize {
            columns_rows: Size { width: 80, height: 25 },
            pixels: Size { width: 0, height: 0 },
        })
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
