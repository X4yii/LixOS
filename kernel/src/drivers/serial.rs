//! # Driver UART 16550 (Puerto Serie COM1)

use crate::arch::x86_64::io::{inb, outb};
use core::fmt;
use spin::Mutex;

const COM1_PORT: u16 = 0x3F8;

pub struct SerialPort {
    port: u16,
}

impl SerialPort {
    pub const fn new(port: u16) -> Self {
        Self { port }
    }

    /// Inicializa el controlador UART 16550 a 115200 baudios, 8N1.
    pub unsafe fn init(&self) {
        outb(self.port + 1, 0x00); // Desactivar interrupciones
        outb(self.port + 3, 0x80); // Activar DLAB (divisor de baudios)
        outb(self.port + 0, 0x01); // Divisor byte bajo (115200 baud)
        outb(self.port + 1, 0x00); // Divisor byte alto
        outb(self.port + 3, 0x03); // 8 bits, sin paridad, un bit de parada (8N1)
        outb(self.port + 2, 0xC7); // Habilitar FIFO, vaciar búferes, umbral 14 bytes
        outb(self.port + 4, 0x0B); // IRQs habilitadas, RTS/DSR activo
    }

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

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
        Ok(())
    }
}

pub static SERIAL1: Mutex<SerialPort> = Mutex::new(SerialPort::new(COM1_PORT));

/// Inicializa COM1 de manera segura.
pub fn init_serial() {
    unsafe {
        SERIAL1.lock().init();
    }
}
