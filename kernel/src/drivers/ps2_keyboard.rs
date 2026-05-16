use pc_keyboard::{layouts, DecodedKey, HandleControl, PS2Keyboard, ScancodeSet1};
use crate::sync::IrqSafeSpinlock;
use x86_64::instructions::port::Port;
use lazy_static::lazy_static;

lazy_static! {
    static ref KEYBOARD: IrqSafeSpinlock<PS2Keyboard<layouts::Us104Key, ScancodeSet1>> =
        IrqSafeSpinlock::new(PS2Keyboard::new(
            ScancodeSet1::new(),
            layouts::Us104Key,
            HandleControl::MapLettersToUnicode
        ));
}

pub fn handle_interrupt() {
    let mut keyboard = crate::sync::IrqSafeSpinlock::lock(&KEYBOARD);
    let mut port = Port::new(0x60);
    
    let scancode: u8 = unsafe { port.read() };
    
    if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
        if let Some(key) = keyboard.process_keyevent(key_event) {
            match key {
                DecodedKey::Unicode(character) => {
                    if character == '\x03' { // Ctrl+C
                        let fg = crate::shell::FOREGROUND_TASK.load(core::sync::atomic::Ordering::SeqCst);
                        if fg != 0 {
                            crate::task::scheduler::send_signal(crate::task::process::ProcessId(fg), 2); // SIGINT
                        }
                        crate::shell::CANCEL_REQUEST.store(true, core::sync::atomic::Ordering::SeqCst);
                    } else {
                        crate::drivers::tty::STDIN.lock().push_back(character);
                        crate::task::scheduler::wake_process(2);
                    }
                }
                DecodedKey::RawKey(raw) => {
                    use pc_keyboard::KeyCode;
                    let ch = match raw {
                        KeyCode::Backspace => Some('\x08'),
                        KeyCode::ArrowUp => Some('\x11'),
                        KeyCode::ArrowDown => Some('\x12'),
                        KeyCode::ArrowLeft => Some('\x13'),
                        KeyCode::ArrowRight => Some('\x14'),
                        _ => None,
                    };
                    if let Some(c) = ch {
                        crate::drivers::tty::STDIN.lock().push_back(c);
                        crate::task::scheduler::wake_process(2);
                    }
                }
            }
        }
    }
}
