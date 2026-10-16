use super::FramebufferConsole;
use crate::klog::EarlyConsole;
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
