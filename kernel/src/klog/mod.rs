pub mod buffer;
pub mod sink;

pub use sink::EarlyConsole;

use crate::sync::IrqSafeSpinlock;
use buffer::KlogRingBuffer;
use core::fmt;

const MAX_SINKS: usize = 4;
const RING_BUFFER_CAPACITY: usize = 65536;

pub struct KlogRouter {
    ring_buffer: KlogRingBuffer<RING_BUFFER_CAPACITY>,
    sinks: [Option<&'static mut dyn EarlyConsole>; MAX_SINKS],
    sink_count: usize,
}

impl KlogRouter {
    pub const fn new() -> Self {
        Self {
            ring_buffer: KlogRingBuffer::new(),
            sinks: [None, None, None, None],
            sink_count: 0,
        }
    }

    pub fn register_sink(&mut self, sink: &'static mut dyn EarlyConsole) {
        if self.sink_count < MAX_SINKS {
            self.sinks[self.sink_count] = Some(sink);
            self.sink_count += 1;
        }
    }

    pub fn write_str_all(&mut self, s: &str) {
        self.ring_buffer.write_str(s);

        for i in 0..self.sink_count {
            if let Some(ref mut sink) = self.sinks[i] {
                sink.write_str(s);
            }
        }
    }
}

impl fmt::Write for KlogRouter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_str_all(s);
        Ok(())
    }
}

static LOGGER: IrqSafeSpinlock<KlogRouter> = IrqSafeSpinlock::new(KlogRouter::new());
pub fn register_console(sink: &'static mut dyn EarlyConsole) {
    LOGGER.lock().register_sink(sink);
}

pub unsafe fn force_unlock() {
    LOGGER.force_unlock();
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    let mut logger = LOGGER.lock();
    let _ = logger.write_fmt(args);
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => ($crate::klog::_print(core::format_args!($($arg)*)));
}

#[macro_export]
macro_rules! kprintln {
    () => ($crate::kprint!("\n"));
    ($fmt:expr) => ($crate::kprint!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::kprint!(concat!($fmt, "\n"), $($arg)*));
}