pub struct KlogRingBuffer<const CAPACITY: usize> {
    buffer: [u8; CAPACITY],
    head: usize,
    count: usize,
    total_written: usize,
}
impl<const CAPACITY: usize> KlogRingBuffer<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            buffer: [0; CAPACITY],
            head: 0,
            count: 0,
            total_written: 0,
        }
    }
    pub fn write_byte(&mut self, byte: u8) {
        self.buffer[self.head] = byte;
        self.head = (self.head + 1) % CAPACITY;
        if self.count < CAPACITY {
            self.count += 1;
        }
        self.total_written += 1;
    }
    pub fn write_str(&mut self, s: &str) {
        for b in s.bytes() {
            self.write_byte(b);
        }
    }
    pub fn total_written(&self) -> usize {
        self.total_written
    }
}
