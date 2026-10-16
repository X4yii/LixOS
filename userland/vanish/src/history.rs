use alloc::string::{String, ToString};
use alloc::vec::Vec;
pub const MAX_COMMAND_BYTES: usize = 4096;
const MAX_HISTORY_ENTRIES: usize = 64;
pub struct History {
    entries: Vec<String>,
    index: usize,
    draft: String,
}
impl History {
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            index: 0,
            draft: String::new(),
        }
    }
    pub fn push(&mut self, cmd: &str) {
        let trimmed = cmd.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_COMMAND_BYTES {
            return;
        }
        if self.entries.is_empty()
            || self.entries.last().map(alloc::string::String::as_str) != Some(trimmed)
        {
            self.entries.push(trimmed.to_string());
            if self.entries.len() > MAX_HISTORY_ENTRIES {
                self.entries.remove(0);
            }
        }
        self.index = self.entries.len();
        self.draft.clear();
    }
    pub fn up(&mut self, current: &str) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        if self.index == self.entries.len() {
            self.draft = current.to_string();
        }
        if self.index > 0 {
            self.index -= 1;
            Some(&self.entries[self.index])
        } else {
            Some(&self.entries[0])
        }
    }
    pub fn down(&mut self) -> Option<&str> {
        if self.entries.is_empty() || self.index >= self.entries.len() {
            return None;
        }
        self.index += 1;
        if self.index < self.entries.len() {
            Some(&self.entries[self.index])
        } else {
            Some(&self.draft)
        }
    }
    pub fn reset_navigation(&mut self) {
        self.index = self.entries.len();
        self.draft.clear();
    }
}
