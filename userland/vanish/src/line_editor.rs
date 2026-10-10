use alloc::string::{String, ToString};
use core::fmt::Write;
use sys_libc::syscalls::{sys_terminal_set_scale, sys_terminal_zoom, sys_write};
use crate::autocomplete::handle_tab_completion;
use crate::history::{History, MAX_COMMAND_BYTES};

pub fn format_prompt(cwd: &str) -> String {
    alloc::format!("\x1b[32;1mvanish\x1b[0m:\x1b[34;1m{}\x1b[0m$ ", cwd)
}

pub fn refresh_line(prompt: &str, line: &str, cursor_idx: usize) {
    sys_write("\r\x1b[2K");
    sys_write(prompt);
    sys_write(line);
    let chars_after = line.get(cursor_idx..).unwrap_or("").chars().count();
    if chars_after > 0 {
        let mut s = String::new();
        let _ = write!(s, "\x1b[{}D", chars_after);
        sys_write(&s);
    }
}

pub fn previous_char_boundary(text: &str, index: usize) -> usize {
    text[..index.min(text.len())].char_indices().next_back().map_or(0, |(i, _)| i)
}

pub fn next_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    text[index..].char_indices().nth(1).map_or(text.len(), |(i, _)| index + i)
}

pub struct LineEditor {
    pub line: String,
    pub cursor: usize,
    esc_state: u8,
    esc_param: u8,
}

impl LineEditor {
    pub fn new() -> Self {
        Self {
            line: String::new(),
            cursor: 0,
            esc_state: 0,
            esc_param: 0,
        }
    }

    pub fn reset(&mut self) {
        self.line.clear();
        self.cursor = 0;
        self.esc_state = 0;
        self.esc_param = 0;
    }

    pub fn set_line(&mut self, text: &str) {
        self.line = text.to_string();
        self.cursor = self.line.len();
    }

    pub fn handle_byte(
        &mut self,
        byte: u8,
        current_dir: &str,
        history: &mut History,
    ) -> Option<String> {
        let prompt = format_prompt(current_dir);

        if self.esc_state == 1 {
            if byte == b'[' {
                self.esc_state = 2;
            } else {
                self.esc_state = 0;
            }
            return None;
        } else if self.esc_state == 2 {
            match byte {
                b'A' => {
                    self.esc_state = 0;
                    if let Some(prev) = history.up(&self.line) {
                        self.set_line(prev);
                        refresh_line(&prompt, &self.line, self.cursor);
                    }
                }
                b'B' => {
                    self.esc_state = 0;
                    if let Some(next) = history.down() {
                        self.set_line(next);
                        refresh_line(&prompt, &self.line, self.cursor);
                    }
                }
                b'C' => {
                    self.esc_state = 0;
                    if self.cursor < self.line.len() {
                        self.cursor = next_char_boundary(&self.line, self.cursor);
                        sys_write("\x1b[C");
                    }
                }
                b'D' => {
                    self.esc_state = 0;
                    if self.cursor > 0 {
                        self.cursor = previous_char_boundary(&self.line, self.cursor);
                        sys_write("\x1b[D");
                    }
                }
                b'H' => {
                    self.esc_state = 0;
                    self.cursor = 0;
                    refresh_line(&prompt, &self.line, self.cursor);
                }
                b'F' => {
                    self.esc_state = 0;
                    self.cursor = self.line.len();
                    refresh_line(&prompt, &self.line, self.cursor);
                }
                b'1' | b'3' | b'4' => {
                    self.esc_param = byte;
                    self.esc_state = 3;
                }
                _ => {
                    self.esc_state = 0;
                }
            }
            return None;
        } else if self.esc_state == 3 {
            self.esc_state = 0;
            if byte == b'~' {
                match self.esc_param {
                    b'1' => {
                        self.cursor = 0;
                        refresh_line(&prompt, &self.line, self.cursor);
                    }
                    b'4' => {
                        self.cursor = self.line.len();
                        refresh_line(&prompt, &self.line, self.cursor);
                    }
                    b'3' => {
                        if self.cursor < self.line.len() {
                            let next = next_char_boundary(&self.line, self.cursor);
                            self.line.drain(self.cursor..next);
                            refresh_line(&prompt, &self.line, self.cursor);
                        }
                    }
                    _ => {}
                }
            }
            return None;
        }

        if byte == 0x1B {
            self.esc_state = 1;
            return None;
        }

        match byte {
            b'\n' | b'\r' => {
                sys_write("\n");
                let cmd = self.line.trim().to_string();
                self.reset();
                Some(cmd)
            }
            0x08 | 0x7F => {
                if self.cursor > 0 {
                    self.cursor = previous_char_boundary(&self.line, self.cursor);
                    let next = next_char_boundary(&self.line, self.cursor);
                    self.line.drain(self.cursor..next);
                    refresh_line(&prompt, &self.line, self.cursor);
                }
                None
            }
            b'\t' => {
                handle_tab_completion(&mut self.line, &mut self.cursor, current_dir, &prompt, refresh_line);
                None
            }
            0x1F => {
                sys_terminal_zoom(-1);
                refresh_line(&prompt, &self.line, self.cursor);
                None
            }
            0x1E => {
                sys_terminal_zoom(1);
                refresh_line(&prompt, &self.line, self.cursor);
                None
            }
            0x1C => {
                sys_terminal_set_scale(1);
                refresh_line(&prompt, &self.line, self.cursor);
                None
            }
            0x03 => {
                sys_write("^C\n");
                self.reset();
                history.reset_navigation();
                sys_write(&prompt);
                None
            }
            b if (0x20..=0x7E).contains(&b) => {
                let ch = b as char;
                if self.line.len() >= MAX_COMMAND_BYTES {
                    return None;
                }
                if self.cursor == self.line.len() {
                    self.line.push(ch);
                    self.cursor += 1;
                    let s = alloc::format!("{}", ch);
                    sys_write(&s);
                } else {
                    self.line.insert(self.cursor, ch);
                    self.cursor += 1;
                    refresh_line(&prompt, &self.line, self.cursor);
                }
                None
            }
            _ => None,
        }
    }
}
