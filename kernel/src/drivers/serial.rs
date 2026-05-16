use crate::arch::x86_64::io::{inb, outb};
use crate::klog::EarlyConsole;

const COM1_PORT: u16 = 0x3F8;

pub struct SerialPort {
    port: u16,
}

impl SerialPort {
    pub const fn new(port: u16) -> Self {
        Self { port }
    }

    pub unsafe fn init_hardware(&self) {
        outb(self.port + 1, 0x00);
        outb(self.port + 3, 0x80);
        outb(self.port + 0, 0x01);
        outb(self.port + 1, 0x00);
        outb(self.port + 3, 0x03);
        outb(self.port + 2, 0xC7);
        outb(self.port + 4, 0x0B);
    }

    #[inline(always)]
    fn is_transmit_empty(&self) -> bool {
        unsafe { (inb(self.port + 5) & 0x20) != 0 }
    }

    pub fn write_byte(&self, byte: u8) {
        while !self.is_transmit_empty() {
            core::hint::spin_loop();
        }
        unsafe {
            outb(self.port, byte);
        }
    }
}

impl EarlyConsole for SerialPort {
    fn write_str(&mut self, s: &str) {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
    }
}

use core::cell::UnsafeCell;

struct SerialDevice(UnsafeCell<SerialPort>);
unsafe impl Sync for SerialDevice {}

static COM1_DEVICE: SerialDevice = SerialDevice(UnsafeCell::new(SerialPort::new(COM1_PORT)));

pub fn init() -> &'static mut SerialPort {
    unsafe {
        let port = &mut *COM1_DEVICE.0.get();
        port.init_hardware();
        port
    }
}
