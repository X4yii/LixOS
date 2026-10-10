use x86_64::instructions::port::Port;
use pc_keyboard::{DecodedKey, KeyCode};

pub fn handle_interrupt() -> bool {
    let mut port = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };

    if let Some(key) = crate::drivers::keyboard::process_scancode(scancode) {
        match key {
            DecodedKey::Unicode(character) => {
                crate::drivers::tty::push_char(character)
            }
            DecodedKey::RawKey(raw) => {
                match raw {
                    KeyCode::Backspace => crate::drivers::tty::push_char('\x08'),
                    KeyCode::ArrowUp => crate::drivers::tty::push_str("\x1b[A"),
                    KeyCode::ArrowDown => crate::drivers::tty::push_str("\x1b[B"),
                    KeyCode::ArrowRight => crate::drivers::tty::push_str("\x1b[C"),
                    KeyCode::ArrowLeft => crate::drivers::tty::push_str("\x1b[D"),
                    KeyCode::Home => crate::drivers::tty::push_str("\x1b[H"),
                    KeyCode::End => crate::drivers::tty::push_str("\x1b[F"),
                    KeyCode::Delete => crate::drivers::tty::push_str("\x1b[3~"),
                    KeyCode::NumpadSubtract => crate::drivers::tty::push_char('-'),
                    KeyCode::NumpadAdd => crate::drivers::tty::push_char('+'),
                    KeyCode::NumpadMultiply => crate::drivers::tty::push_char('*'),
                    KeyCode::NumpadDivide => crate::drivers::tty::push_char('/'),
                    KeyCode::NumpadEnter => crate::drivers::tty::push_char('\n'),
                    _ => false,
                }
            }
        }
    } else {
        false
    }
}
