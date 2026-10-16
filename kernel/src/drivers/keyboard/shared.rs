use pc_keyboard::{DecodedKey, HandleControl, KeyCode, Modifiers};
#[derive(Clone, Copy)]
pub struct Symbol {
    pub normal: char,
    pub shifted: char,
    pub altgr: Option<char>,
}
pub struct Tables {
    pub symbols: &'static [(KeyCode, Symbol)],
    pub altgr_letters: &'static [(KeyCode, char)],
    pub numlock_period: char,
}
pub const fn symbol(normal: char, shifted: char) -> Symbol {
    Symbol {
        normal,
        shifted,
        altgr: None,
    }
}
pub const fn symbol_gr(normal: char, shifted: char, altgr: char) -> Symbol {
    Symbol {
        normal,
        shifted,
        altgr: Some(altgr),
    }
}
pub fn handle_ascii(modifiers: &Modifiers, upper: char, handle_ctrl: HandleControl) -> DecodedKey {
    if handle_ctrl == HandleControl::MapLettersToUnicode && modifiers.is_ctrl() {
        DecodedKey::Unicode(((upper as u8) - 64) as char)
    } else if modifiers.is_caps() {
        DecodedKey::Unicode(upper)
    } else {
        DecodedKey::Unicode(((upper as u8) + 32) as char)
    }
}
pub fn handle_symbol(modifiers: &Modifiers, symbol: Symbol) -> DecodedKey {
    if modifiers.is_altgr() {
        if let Some(altgr) = symbol.altgr {
            return DecodedKey::Unicode(altgr);
        }
    }
    if modifiers.is_shifted() {
        DecodedKey::Unicode(symbol.shifted)
    } else {
        DecodedKey::Unicode(symbol.normal)
    }
}
const COMMON_SYMBOLS: &[(KeyCode, Symbol)] = &[
    (KeyCode::Key7, symbol('7', '/')),
    (KeyCode::Key8, symbol('8', '(')),
    (KeyCode::Key9, symbol('9', ')')),
    (KeyCode::Key0, symbol('0', '=')),
    (KeyCode::OemComma, symbol_gr(',', ';', '<')),
    (KeyCode::OemPeriod, symbol_gr('.', ':', '>')),
    (KeyCode::Oem2, symbol('-', '_')),
    (KeyCode::Oem5, symbol_gr('<', '>', '|')),
];
const LETTERS: &[(KeyCode, char)] = &[
    (KeyCode::A, 'A'),
    (KeyCode::B, 'B'),
    (KeyCode::C, 'C'),
    (KeyCode::D, 'D'),
    (KeyCode::E, 'E'),
    (KeyCode::F, 'F'),
    (KeyCode::G, 'G'),
    (KeyCode::H, 'H'),
    (KeyCode::I, 'I'),
    (KeyCode::J, 'J'),
    (KeyCode::K, 'K'),
    (KeyCode::L, 'L'),
    (KeyCode::M, 'M'),
    (KeyCode::N, 'N'),
    (KeyCode::O, 'O'),
    (KeyCode::P, 'P'),
    (KeyCode::Q, 'Q'),
    (KeyCode::R, 'R'),
    (KeyCode::S, 'S'),
    (KeyCode::T, 'T'),
    (KeyCode::U, 'U'),
    (KeyCode::V, 'V'),
    (KeyCode::W, 'W'),
    (KeyCode::X, 'X'),
    (KeyCode::Y, 'Y'),
    (KeyCode::Z, 'Z'),
];
fn find_symbol(symbols: &[(KeyCode, Symbol)], keycode: KeyCode) -> Option<Symbol> {
    symbols
        .iter()
        .find(|(key, _)| *key == keycode)
        .map(|(_, symbol)| *symbol)
}
const fn numlock_char(modifiers: &Modifiers, digit: char, fallback: KeyCode) -> DecodedKey {
    if modifiers.numlock {
        DecodedKey::Unicode(digit)
    } else {
        DecodedKey::RawKey(fallback)
    }
}
pub fn map_keycode(
    keycode: KeyCode,
    modifiers: &Modifiers,
    handle_ctrl: HandleControl,
    tables: &Tables,
) -> DecodedKey {
    match keycode {
        KeyCode::Escape => return DecodedKey::Unicode('\u{001B}'),
        KeyCode::Backspace => return DecodedKey::Unicode('\u{0008}'),
        KeyCode::Tab => return DecodedKey::Unicode('\u{0009}'),
        KeyCode::Return => return DecodedKey::Unicode('\u{000A}'),
        KeyCode::Spacebar => return DecodedKey::Unicode(' '),
        KeyCode::Oem1 => {
            return if modifiers.is_caps() {
                DecodedKey::Unicode('Ñ')
            } else {
                DecodedKey::Unicode('ñ')
            };
        }
        KeyCode::NumpadEnter => return DecodedKey::Unicode('\u{000A}'),
        KeyCode::NumpadPeriod => {
            return if modifiers.numlock {
                DecodedKey::Unicode(tables.numlock_period)
            } else {
                DecodedKey::RawKey(KeyCode::Delete)
            };
        }
        KeyCode::Numpad0 => return numlock_char(modifiers, '0', KeyCode::Insert),
        KeyCode::Numpad1 => return numlock_char(modifiers, '1', KeyCode::End),
        KeyCode::Numpad2 => return numlock_char(modifiers, '2', KeyCode::ArrowDown),
        KeyCode::Numpad3 => return numlock_char(modifiers, '3', KeyCode::PageDown),
        KeyCode::Numpad4 => return numlock_char(modifiers, '4', KeyCode::ArrowLeft),
        KeyCode::Numpad5 => return DecodedKey::Unicode('5'),
        KeyCode::Numpad6 => return numlock_char(modifiers, '6', KeyCode::ArrowRight),
        KeyCode::Numpad7 => return numlock_char(modifiers, '7', KeyCode::Home),
        KeyCode::Numpad8 => return numlock_char(modifiers, '8', KeyCode::ArrowUp),
        KeyCode::Numpad9 => return numlock_char(modifiers, '9', KeyCode::PageUp),
        _ => {}
    }
    if let Some(symbol) =
        find_symbol(tables.symbols, keycode).or_else(|| find_symbol(COMMON_SYMBOLS, keycode))
    {
        return handle_symbol(modifiers, symbol);
    }
    if modifiers.is_altgr() {
        if let Some((_, letter)) = tables.altgr_letters.iter().find(|(key, _)| *key == keycode) {
            return DecodedKey::Unicode(*letter);
        }
    }
    if let Some((_, upper)) = LETTERS.iter().find(|(key, _)| *key == keycode) {
        return handle_ascii(modifiers, *upper, handle_ctrl);
    }
    DecodedKey::RawKey(keycode)
}
