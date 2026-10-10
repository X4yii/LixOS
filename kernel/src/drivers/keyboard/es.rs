//! Distribución de Teclado Español España (es / ISO 105) para LixOS.
//! Implementa `KeyboardLayout` para pc-keyboard.

use pc_keyboard::{DecodedKey, HandleControl, KeyCode, KeyboardLayout, Modifiers, PhysicalKeyboard};

#[derive(Debug, Clone, Copy, Default)]
pub struct SpainLayout;

impl SpainLayout {
    fn handle_ascii(modifiers: &Modifiers, upper: char, handle_ctrl: HandleControl) -> DecodedKey {
        if handle_ctrl == HandleControl::MapLettersToUnicode && modifiers.is_ctrl() {
            DecodedKey::Unicode(((upper as u8) - 64) as char)
        } else if modifiers.is_caps() {
            DecodedKey::Unicode(upper)
        } else {
            DecodedKey::Unicode(((upper as u8) + 32) as char)
        }
    }

    fn handle_symbol(modifiers: &Modifiers, normal: char, shifted: char, altgr: Option<char>) -> DecodedKey {
        if modifiers.is_altgr() && altgr.is_some() {
            DecodedKey::Unicode(altgr.unwrap())
        } else if modifiers.is_shifted() {
            DecodedKey::Unicode(shifted)
        } else {
            DecodedKey::Unicode(normal)
        }
    }
}

impl KeyboardLayout for SpainLayout {
    fn map_keycode(
        &self,
        keycode: KeyCode,
        modifiers: &Modifiers,
        handle_ctrl: HandleControl,
    ) -> DecodedKey {
        match keycode {
            KeyCode::Escape => DecodedKey::Unicode('\u{001B}'),

            KeyCode::Oem8 => Self::handle_symbol(modifiers, 'º', 'ª', Some('\\')),
            KeyCode::Key1 => Self::handle_symbol(modifiers, '1', '!', Some('|')),
            KeyCode::Key2 => Self::handle_symbol(modifiers, '2', '"', Some('@')),
            KeyCode::Key3 => Self::handle_symbol(modifiers, '3', '·', Some('#')),
            KeyCode::Key4 => Self::handle_symbol(modifiers, '4', '$', Some('~')),
            KeyCode::Key5 => Self::handle_symbol(modifiers, '5', '%', Some('€')),
            KeyCode::Key6 => Self::handle_symbol(modifiers, '6', '&', Some('¬')),
            KeyCode::Key7 => Self::handle_symbol(modifiers, '7', '/', None),
            KeyCode::Key8 => Self::handle_symbol(modifiers, '8', '(', None),
            KeyCode::Key9 => Self::handle_symbol(modifiers, '9', ')', None),
            KeyCode::Key0 => Self::handle_symbol(modifiers, '0', '=', None),
            KeyCode::OemMinus => Self::handle_symbol(modifiers, '\'', '?', None),
            KeyCode::OemPlus => Self::handle_symbol(modifiers, '¡', '¿', None),
            KeyCode::Backspace => DecodedKey::Unicode('\u{0008}'),

            KeyCode::Tab => DecodedKey::Unicode('\u{0009}'),
            KeyCode::Q => Self::handle_ascii(modifiers, 'Q', handle_ctrl),
            KeyCode::W => Self::handle_ascii(modifiers, 'W', handle_ctrl),
            KeyCode::E => {
                if modifiers.is_altgr() {
                    DecodedKey::Unicode('€')
                } else {
                    Self::handle_ascii(modifiers, 'E', handle_ctrl)
                }
            }
            KeyCode::R => Self::handle_ascii(modifiers, 'R', handle_ctrl),
            KeyCode::T => Self::handle_ascii(modifiers, 'T', handle_ctrl),
            KeyCode::Y => Self::handle_ascii(modifiers, 'Y', handle_ctrl),
            KeyCode::U => Self::handle_ascii(modifiers, 'U', handle_ctrl),
            KeyCode::I => Self::handle_ascii(modifiers, 'I', handle_ctrl),
            KeyCode::O => Self::handle_ascii(modifiers, 'O', handle_ctrl),
            KeyCode::P => Self::handle_ascii(modifiers, 'P', handle_ctrl),
            KeyCode::Oem4 => Self::handle_symbol(modifiers, '`', '^', Some('[')),
            KeyCode::Oem6 => Self::handle_symbol(modifiers, '+', '*', Some(']')),

            KeyCode::A => Self::handle_ascii(modifiers, 'A', handle_ctrl),
            KeyCode::S => Self::handle_ascii(modifiers, 'S', handle_ctrl),
            KeyCode::D => Self::handle_ascii(modifiers, 'D', handle_ctrl),
            KeyCode::F => Self::handle_ascii(modifiers, 'F', handle_ctrl),
            KeyCode::G => Self::handle_ascii(modifiers, 'G', handle_ctrl),
            KeyCode::H => Self::handle_ascii(modifiers, 'H', handle_ctrl),
            KeyCode::J => Self::handle_ascii(modifiers, 'J', handle_ctrl),
            KeyCode::K => Self::handle_ascii(modifiers, 'K', handle_ctrl),
            KeyCode::L => Self::handle_ascii(modifiers, 'L', handle_ctrl),
            KeyCode::Oem1 => {
                if modifiers.is_caps() {
                    DecodedKey::Unicode('Ñ')
                } else {
                    DecodedKey::Unicode('ñ')
                }
            }
            KeyCode::Oem3 => Self::handle_symbol(modifiers, '´', '¨', Some('{')),
            KeyCode::Oem7 => Self::handle_symbol(modifiers, 'ç', 'Ç', Some('}')),
            KeyCode::Return => DecodedKey::Unicode('\u{000A}'),

            KeyCode::Oem5 => Self::handle_symbol(modifiers, '<', '>', None),
            KeyCode::Z => Self::handle_ascii(modifiers, 'Z', handle_ctrl),
            KeyCode::X => Self::handle_ascii(modifiers, 'X', handle_ctrl),
            KeyCode::C => Self::handle_ascii(modifiers, 'C', handle_ctrl),
            KeyCode::V => Self::handle_ascii(modifiers, 'V', handle_ctrl),
            KeyCode::B => Self::handle_ascii(modifiers, 'B', handle_ctrl),
            KeyCode::N => Self::handle_ascii(modifiers, 'N', handle_ctrl),
            KeyCode::M => Self::handle_ascii(modifiers, 'M', handle_ctrl),
            KeyCode::OemComma => Self::handle_symbol(modifiers, ',', ';', None),
            KeyCode::OemPeriod => Self::handle_symbol(modifiers, '.', ':', None),
            KeyCode::Oem2 => Self::handle_symbol(modifiers, '-', '_', None),

            KeyCode::Spacebar => DecodedKey::Unicode(' '),
            KeyCode::Delete => DecodedKey::RawKey(KeyCode::Delete),
            KeyCode::Insert => DecodedKey::RawKey(KeyCode::Insert),
            KeyCode::Home => DecodedKey::RawKey(KeyCode::Home),
            KeyCode::End => DecodedKey::RawKey(KeyCode::End),
            KeyCode::PageUp => DecodedKey::RawKey(KeyCode::PageUp),
            KeyCode::PageDown => DecodedKey::RawKey(KeyCode::PageDown),
            KeyCode::ArrowUp => DecodedKey::RawKey(KeyCode::ArrowUp),
            KeyCode::ArrowDown => DecodedKey::RawKey(KeyCode::ArrowDown),
            KeyCode::ArrowLeft => DecodedKey::RawKey(KeyCode::ArrowLeft),
            KeyCode::ArrowRight => DecodedKey::RawKey(KeyCode::ArrowRight),

            KeyCode::NumpadDivide => DecodedKey::Unicode('/'),
            KeyCode::NumpadMultiply => DecodedKey::Unicode('*'),
            KeyCode::NumpadSubtract => DecodedKey::Unicode('-'),
            KeyCode::NumpadAdd => DecodedKey::Unicode('+'),
            KeyCode::NumpadEnter => DecodedKey::Unicode('\u{000A}'),
            KeyCode::NumpadPeriod => {
                if modifiers.numlock {
                    DecodedKey::Unicode(',')
                } else {
                    DecodedKey::RawKey(KeyCode::Delete)
                }
            }
            KeyCode::Numpad0 => {
                if modifiers.numlock { DecodedKey::Unicode('0') } else { DecodedKey::RawKey(KeyCode::Insert) }
            }
            KeyCode::Numpad1 => {
                if modifiers.numlock { DecodedKey::Unicode('1') } else { DecodedKey::RawKey(KeyCode::End) }
            }
            KeyCode::Numpad2 => {
                if modifiers.numlock { DecodedKey::Unicode('2') } else { DecodedKey::RawKey(KeyCode::ArrowDown) }
            }
            KeyCode::Numpad3 => {
                if modifiers.numlock { DecodedKey::Unicode('3') } else { DecodedKey::RawKey(KeyCode::PageDown) }
            }
            KeyCode::Numpad4 => {
                if modifiers.numlock { DecodedKey::Unicode('4') } else { DecodedKey::RawKey(KeyCode::ArrowLeft) }
            }
            KeyCode::Numpad5 => DecodedKey::Unicode('5'),
            KeyCode::Numpad6 => {
                if modifiers.numlock { DecodedKey::Unicode('6') } else { DecodedKey::RawKey(KeyCode::ArrowRight) }
            }
            KeyCode::Numpad7 => {
                if modifiers.numlock { DecodedKey::Unicode('7') } else { DecodedKey::RawKey(KeyCode::Home) }
            }
            KeyCode::Numpad8 => {
                if modifiers.numlock { DecodedKey::Unicode('8') } else { DecodedKey::RawKey(KeyCode::ArrowUp) }
            }
            KeyCode::Numpad9 => {
                if modifiers.numlock { DecodedKey::Unicode('9') } else { DecodedKey::RawKey(KeyCode::PageUp) }
            }

            raw => DecodedKey::RawKey(raw),
        }
    }

    fn get_physical(&self) -> PhysicalKeyboard {
        PhysicalKeyboard::Iso
    }
}
