#![no_std]
#![allow(unsafe_code)]
extern crate alloc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;
use sys_libc::strerror;
use sys_libc::syscalls::{
    TerminalInfo, sys_read_file_all, sys_read_nonblocking, sys_terminal_info, sys_write,
    sys_write_file,
};
pub fn restore_terminal() {
    sys_write("\x1b[?1049l\x1b[?25h\x1b[0m");
}
struct Utf8Input {
    buf: [u8; 4],
    len: usize,
    need: usize,
}
impl Utf8Input {
    const fn new() -> Self {
        Self {
            buf: [0; 4],
            len: 0,
            need: 0,
        }
    }
    fn push(&mut self, byte: u8) -> Option<char> {
        if self.need > 0 {
            if (byte & 0xC0) == 0x80 {
                self.buf[self.len] = byte;
                self.len += 1;
                if self.len == self.need {
                    let decoded = core::str::from_utf8(&self.buf[..self.len])
                        .ok()
                        .and_then(|text| text.chars().next());
                    self.len = 0;
                    self.need = 0;
                    return decoded;
                }
                return None;
            }
            self.len = 0;
            self.need = 0;
        }
        self.need = match byte {
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => return None,
        };
        self.buf[0] = byte;
        self.len = 1;
        None
    }
}
fn apply_insert(
    lines: &mut Vec<String>,
    cursor_row: usize,
    cursor_col: &mut usize,
    sel_range: Option<((usize, usize), (usize, usize))>,
    sel_anchor: &mut Option<(usize, usize)>,
    modified: &mut bool,
    ch: char,
) {
    if let Some((start, end)) = sel_range {
        if start.0 == end.0 {
            let mut chars: Vec<char> = lines[start.0].chars().collect();
            let from = start.1.min(chars.len());
            let to = end.1.min(chars.len()).max(from);
            chars.drain(from..to);
            lines[start.0] = chars.iter().collect();
            *cursor_col = from;
        }
        *sel_anchor = None;
    }
    *modified = true;
    let mut chars: Vec<char> = lines[cursor_row].chars().collect();
    let ins_idx = (*cursor_col).min(chars.len());
    chars.insert(ins_idx, ch);
    lines[cursor_row] = chars.iter().collect();
    *cursor_col += 1;
}
pub fn run_editor(file_path: &str) {
    let mut lines: Vec<String> = Vec::new();
    if let Ok(bytes) = sys_read_file_all(file_path)
        && let Ok(content) = core::str::from_utf8(&bytes)
    {
        for line in content.split('\n') {
            let clean = line.strip_suffix('\r').unwrap_or(line);
            lines.push(clean.to_string());
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    let mut cursor_row = 0usize;
    let mut cursor_col = 0usize;
    let mut scroll_row = 0usize;
    let mut scroll_col = 0usize;
    let mut modified = false;
    let mut clipboard = String::new();
    let mut sel_anchor: Option<(usize, usize)> = None;
    let mut status_msg = String::from("Listo. Presiona ^S para guardar, ^X para salir.");
    let mut confirm_exit = false;
    sys_write("\x1b[?1049h\x1b[?25h");
    let mut esc_state = 0u8;
    let mut esc_buf = [0u8; 8];
    let mut esc_len = 0usize;
    let mut utf8 = Utf8Input::new();
    loop {
        let mut term_info = TerminalInfo::default();
        let _ = sys_terminal_info(&mut term_info);
        let term_cols = if term_info.cols >= 20 {
            term_info.cols as usize
        } else {
            80
        };
        let term_rows = if term_info.rows >= 8 {
            term_info.rows as usize
        } else {
            25
        };
        let text_rows = term_rows.saturating_sub(2);
        let gutter_width = 5usize;
        let content_cols = term_cols.saturating_sub(gutter_width);
        if cursor_row < scroll_row {
            scroll_row = cursor_row;
        } else if cursor_row >= scroll_row + text_rows {
            scroll_row = cursor_row.saturating_sub(text_rows - 1);
        }
        if cursor_col < scroll_col {
            scroll_col = cursor_col;
        } else if cursor_col >= scroll_col + content_cols {
            scroll_col = cursor_col.saturating_sub(content_cols - 1);
        }
        let mut frame = String::with_capacity(8192);
        frame.push_str("\x1b[H\x1b[?25l");
        frame.push_str("\x1b[7m");
        let mod_flag = if modified { " [*Modificado*]" } else { "" };
        let header_left = alloc::format!(" [ LixOS Edit ]  {file_path}  {mod_flag} ");
        let header_right = alloc::format!(
            " Ln {}, Col {}   (Total: {}) ",
            cursor_row + 1,
            cursor_col + 1,
            lines.len()
        );
        frame.push_str(&header_left);
        let remaining_pad = term_cols.saturating_sub(header_left.len() + header_right.len());
        for _ in 0..remaining_pad {
            frame.push(' ');
        }
        frame.push_str(&header_right);
        frame.push_str("\x1b[0m\r\n");
        let sel_range = sel_anchor.map(|(ar, ac)| {
            if ar < cursor_row || (ar == cursor_row && ac <= cursor_col) {
                ((ar, ac), (cursor_row, cursor_col))
            } else {
                ((cursor_row, cursor_col), (ar, ac))
            }
        });
        for row_idx in 0..text_rows {
            let line_idx = scroll_row + row_idx;
            frame.push_str("\x1b[2K");
            if line_idx < lines.len() {
                let _ = write!(frame, "\x1b[90m{:4} │\x1b[0m", line_idx + 1);
                let line_str = &lines[line_idx];
                let chars: Vec<char> = line_str.chars().collect();
                let mut col_pos = 0usize;
                let mut screen_col = 0usize;
                for &ch in &chars {
                    let is_selected = sel_range.is_some_and(|(start, end)| {
                        if line_idx < start.0 || line_idx > end.0 {
                            false
                        } else if line_idx == start.0 && line_idx == end.0 {
                            col_pos >= start.1 && col_pos < end.1
                        } else if line_idx == start.0 {
                            col_pos >= start.1
                        } else if line_idx == end.0 {
                            col_pos < end.1
                        } else {
                            true
                        }
                    });
                    if col_pos >= scroll_col && screen_col < content_cols {
                        if is_selected {
                            frame.push_str("\x1b[7m");
                            frame.push(ch);
                            frame.push_str("\x1b[0m");
                        } else {
                            frame.push(ch);
                        }
                        screen_col += 1;
                    }
                    col_pos += 1;
                }
            } else {
                frame.push_str("\x1b[90m   ~ │\x1b[0m");
            }
            frame.push_str("\r\n");
        }
        frame.push_str("\x1b[2K\x1b[7m");
        let help_text =
            " ^S Guardar   ^X Salir   ^C Copiar   ^V Pegar   Shift+Flechas Sel   ^Flechas Palabra ";
        frame.push_str(help_text);
        let help_pad = term_cols.saturating_sub(help_text.len());
        for _ in 0..help_pad {
            frame.push(' ');
        }
        frame.push_str("\x1b[0m\r\n");
        frame.push_str("\x1b[2K");
        if confirm_exit {
            frame.push_str(
                "\x1b[33;1m¿Hay cambios sin guardar. Salir de todas formas? (y/n):\x1b[0m ",
            );
        } else {
            frame.push_str(&status_msg);
        }
        let render_screen_col = gutter_width + cursor_col.saturating_sub(scroll_col) + 1;
        let render_screen_row = 2 + cursor_row.saturating_sub(scroll_row);
        let _ = write!(
            frame,
            "\x1b[{render_screen_row};{render_screen_col}H\x1b[?25h"
        );
        sys_write(&frame);
        let mut read_buf = [0u8; 16];
        let count = sys_read_nonblocking(0, &mut read_buf).unwrap_or(0);
        if count == 0 {
            core::hint::spin_loop();
            continue;
        }
        for i in 0..count {
            let b = read_buf[i];
            let typed = utf8.push(b);
            if confirm_exit {
                if b == b'y' || b == b'Y' {
                    restore_terminal();
                    return;
                }
                confirm_exit = false;
                status_msg = String::from("Salida cancelada.");
                continue;
            }
            if esc_state == 1 {
                if b == b'[' {
                    esc_state = 2;
                    esc_len = 0;
                } else {
                    esc_state = 0;
                }
                continue;
            } else if esc_state == 2 {
                if (b'0'..=b'9').contains(&b) || b == b';' {
                    if esc_len < esc_buf.len() {
                        esc_buf[esc_len] = b;
                        esc_len += 1;
                    }
                    continue;
                }
                esc_state = 0;
                let param_str = core::str::from_utf8(&esc_buf[..esc_len]).unwrap_or("");
                if param_str == "1;2" {
                    if sel_anchor.is_none() {
                        sel_anchor = Some((cursor_row, cursor_col));
                    }
                    if b == b'D' {
                        if cursor_col > 0 {
                            cursor_col -= 1;
                        } else if cursor_row > 0 {
                            cursor_row -= 1;
                            cursor_col = lines[cursor_row].chars().count();
                        }
                    } else if b == b'C' {
                        let cur_len = lines[cursor_row].chars().count();
                        if cursor_col < cur_len {
                            cursor_col += 1;
                        } else if cursor_row + 1 < lines.len() {
                            cursor_row += 1;
                            cursor_col = 0;
                        }
                    }
                    continue;
                }
                if param_str == "1;5" || param_str == "5" {
                    sel_anchor = None;
                    if b == b'D' {
                        let chars: Vec<char> = lines[cursor_row].chars().collect();
                        while cursor_col > 0
                            && cursor_col <= chars.len()
                            && chars
                                .get(cursor_col.saturating_sub(1))
                                .is_some_and(|c| c.is_whitespace())
                        {
                            cursor_col -= 1;
                        }
                        while cursor_col > 0
                            && cursor_col <= chars.len()
                            && chars
                                .get(cursor_col.saturating_sub(1))
                                .is_some_and(|c| !c.is_whitespace())
                        {
                            cursor_col -= 1;
                        }
                    } else if b == b'C' {
                        let chars: Vec<char> = lines[cursor_row].chars().collect();
                        while cursor_col < chars.len()
                            && chars.get(cursor_col).is_some_and(|c| !c.is_whitespace())
                        {
                            cursor_col += 1;
                        }
                        while cursor_col < chars.len()
                            && chars.get(cursor_col).is_some_and(|c| c.is_whitespace())
                        {
                            cursor_col += 1;
                        }
                    }
                    continue;
                }
                match b {
                    b'A' => {
                        sel_anchor = None;
                        if cursor_row > 0 {
                            cursor_row -= 1;
                            let max_col = lines[cursor_row].chars().count();
                            if cursor_col > max_col {
                                cursor_col = max_col;
                            }
                        }
                    }
                    b'B' => {
                        sel_anchor = None;
                        if cursor_row + 1 < lines.len() {
                            cursor_row += 1;
                            let max_col = lines[cursor_row].chars().count();
                            if cursor_col > max_col {
                                cursor_col = max_col;
                            }
                        }
                    }
                    b'C' => {
                        sel_anchor = None;
                        let max_col = lines[cursor_row].chars().count();
                        if cursor_col < max_col {
                            cursor_col += 1;
                        } else if cursor_row + 1 < lines.len() {
                            cursor_row += 1;
                            cursor_col = 0;
                        }
                    }
                    b'D' => {
                        sel_anchor = None;
                        if cursor_col > 0 {
                            cursor_col -= 1;
                        } else if cursor_row > 0 {
                            cursor_row -= 1;
                            cursor_col = lines[cursor_row].chars().count();
                        }
                    }
                    b'H' => {
                        sel_anchor = None;
                        cursor_col = 0;
                    }
                    b'F' => {
                        sel_anchor = None;
                        cursor_col = lines[cursor_row].chars().count();
                    }
                    _ => {}
                }
                continue;
            }
            if b == 0x1B {
                esc_state = 1;
                continue;
            }
            if let Some(ch) = typed {
                apply_insert(
                    &mut lines,
                    cursor_row,
                    &mut cursor_col,
                    sel_range,
                    &mut sel_anchor,
                    &mut modified,
                    ch,
                );
                continue;
            }
            match b {
                0x13 => {
                    let mut file_content = String::new();
                    for (idx, line) in lines.iter().enumerate() {
                        if idx > 0 {
                            file_content.push('\n');
                        }
                        file_content.push_str(line);
                    }
                    match sys_write_file(file_path, file_content.as_bytes()) {
                        Ok(bytes) => {
                            modified = false;
                            status_msg =
                                alloc::format!("Guardado: Escritos {bytes} bytes en {file_path}");
                        }
                        Err(e) => {
                            status_msg = alloc::format!(
                                "Error al guardar en {}: {} (código {})",
                                file_path,
                                strerror(e),
                                e
                            );
                        }
                    }
                }
                0x18 => {
                    if modified {
                        confirm_exit = true;
                    } else {
                        restore_terminal();
                        return;
                    }
                }
                0x03 => {
                    if let Some(range) = sel_range {
                        let (start, end) = range;
                        clipboard.clear();
                        if start.0 == end.0 {
                            let chars: Vec<char> = lines[start.0].chars().collect();
                            let from = start.1.min(chars.len());
                            let to = end.1.min(chars.len()).max(from);
                            let sub: String = chars[from..to].iter().collect();
                            clipboard.push_str(&sub);
                        } else {
                            for r in start.0..=end.0 {
                                let chars: Vec<char> = lines[r].chars().collect();
                                if r == start.0 {
                                    let sub: String =
                                        chars[start.1.min(chars.len())..].iter().collect();
                                    clipboard.push_str(&sub);
                                    clipboard.push('\n');
                                } else if r == end.0 {
                                    let sub: String =
                                        chars[..end.1.min(chars.len())].iter().collect();
                                    clipboard.push_str(&sub);
                                } else {
                                    let sub: String = chars.iter().collect();
                                    clipboard.push_str(&sub);
                                    clipboard.push('\n');
                                }
                            }
                        }
                        status_msg = alloc::format!(
                            "Copiado: {} caracteres al portapapeles.",
                            clipboard.len()
                        );
                    } else {
                        clipboard = lines[cursor_row].clone();
                        status_msg = String::from("Copiada la línea completa al portapapeles.");
                    }
                }
                0x16 => {
                    if !clipboard.is_empty() {
                        modified = true;
                        if clipboard.contains('\n') {
                            let clip_lines: Vec<&str> = clipboard.split('\n').collect();
                            let current_line = &lines[cursor_row];
                            let chars: Vec<char> = current_line.chars().collect();
                            let before: String =
                                chars[..cursor_col.min(chars.len())].iter().collect();
                            let after: String =
                                chars[cursor_col.min(chars.len())..].iter().collect();
                            let mut first = before;
                            first.push_str(clip_lines[0]);
                            lines[cursor_row] = first;
                            for (idx, clip_line) in clip_lines[1..].iter().enumerate() {
                                let target_idx = cursor_row + idx + 1;
                                if idx + 1 == clip_lines.len() - 1 {
                                    let mut last = clip_line.to_string();
                                    last.push_str(&after);
                                    lines.insert(target_idx, last);
                                } else {
                                    lines.insert(target_idx, clip_line.to_string());
                                }
                            }
                            cursor_row += clip_lines.len() - 1;
                            cursor_col = clip_lines.last().map_or(0, |l| l.chars().count());
                        } else {
                            let mut chars: Vec<char> = lines[cursor_row].chars().collect();
                            let clip_chars: Vec<char> = clipboard.chars().collect();
                            let ins_idx = cursor_col.min(chars.len());
                            for (offset, &ch) in clip_chars.iter().enumerate() {
                                chars.insert(ins_idx + offset, ch);
                            }
                            lines[cursor_row] = chars.iter().collect();
                            cursor_col += clip_chars.len();
                        }
                        status_msg = String::from("Texto pegado.");
                    }
                }
                0x08 | 0x7F => {
                    if let Some(range) = sel_range {
                        let (start, end) = range;
                        modified = true;
                        if start.0 == end.0 {
                            let mut chars: Vec<char> = lines[start.0].chars().collect();
                            let from = start.1.min(chars.len());
                            let to = end.1.min(chars.len()).max(from);
                            chars.drain(from..to);
                            lines[start.0] = chars.iter().collect();
                            cursor_col = from;
                        } else {
                            let start_chars: Vec<char> = lines[start.0].chars().collect();
                            let end_chars: Vec<char> = lines[end.0].chars().collect();
                            let mut merged: String = start_chars[..start.1.min(start_chars.len())]
                                .iter()
                                .collect();
                            let right: String =
                                end_chars[end.1.min(end_chars.len())..].iter().collect();
                            merged.push_str(&right);
                            lines[start.0] = merged;
                            for _ in (start.0 + 1)..=end.0 {
                                lines.remove(start.0 + 1);
                            }
                            cursor_row = start.0;
                            cursor_col = start.1.min(start_chars.len());
                        }
                        sel_anchor = None;
                    } else if cursor_col > 0 {
                        modified = true;
                        let mut chars: Vec<char> = lines[cursor_row].chars().collect();
                        let idx = (cursor_col - 1).min(chars.len().saturating_sub(1));
                        if idx < chars.len() {
                            chars.remove(idx);
                            lines[cursor_row] = chars.iter().collect();
                            cursor_col = idx;
                        } else {
                            cursor_col = 0;
                        }
                    } else if cursor_row > 0 {
                        modified = true;
                        let prev_len = lines[cursor_row - 1].chars().count();
                        let curr_line = lines.remove(cursor_row);
                        lines[cursor_row - 1].push_str(&curr_line);
                        cursor_row -= 1;
                        cursor_col = prev_len;
                    }
                }
                b'\r' | b'\n' => {
                    sel_anchor = None;
                    modified = true;
                    let current = &lines[cursor_row];
                    let chars: Vec<char> = current.chars().collect();
                    let before: String = chars[..cursor_col.min(chars.len())].iter().collect();
                    let after: String = chars[cursor_col.min(chars.len())..].iter().collect();
                    lines[cursor_row] = before;
                    lines.insert(cursor_row + 1, after);
                    cursor_row += 1;
                    cursor_col = 0;
                }
                b'\t' => {
                    sel_anchor = None;
                    modified = true;
                    let mut chars: Vec<char> = lines[cursor_row].chars().collect();
                    let ins_idx = cursor_col.min(chars.len());
                    chars.insert(ins_idx, ' ');
                    chars.insert(ins_idx + 1, ' ');
                    chars.insert(ins_idx + 2, ' ');
                    chars.insert(ins_idx + 3, ' ');
                    lines[cursor_row] = chars.iter().collect();
                    cursor_col += 4;
                }
                b if (0x20..=0x7E).contains(&b) => {
                    apply_insert(
                        &mut lines,
                        cursor_row,
                        &mut cursor_col,
                        sel_range,
                        &mut sel_anchor,
                        &mut modified,
                        b as char,
                    );
                }
                _ => {}
            }
        }
    }
}
