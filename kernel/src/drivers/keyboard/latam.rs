use super::shared::{self, symbol, symbol_gr, Symbol, Tables};
use pc_keyboard::{
    DecodedKey, HandleControl, KeyCode, KeyboardLayout, Modifiers, PhysicalKeyboard,
};
const SYMBOLS: &[(KeyCode, Symbol)] = &[
    (KeyCode::Oem8, symbol_gr('|', '°', '¬')),
    (KeyCode::Key1, symbol('1', '!')),
    (KeyCode::Key2, symbol('2', '"')),
    (KeyCode::Key3, symbol('3', '#')),
    (KeyCode::Key4, symbol('4', '$')),
    (KeyCode::Key5, symbol('5', '%')),
    (KeyCode::Key6, symbol('6', '&')),
    (KeyCode::OemMinus, symbol_gr('\'', '?', '\\')),
    (KeyCode::OemPlus, symbol('¿', '¡')),
    (KeyCode::Oem4, symbol('´', '¨')),
    (KeyCode::Oem6, symbol_gr('+', '*', '~')),
    (KeyCode::Oem3, symbol_gr('{', '[', '^')),
    (KeyCode::Oem7, symbol_gr('}', ']', '`')),
    (KeyCode::Oem5, symbol_gr('<', '>', '|')),
    (KeyCode::OemComma, symbol_gr(',', ';', '<')),
    (KeyCode::OemPeriod, symbol_gr('.', ':', '>')),
];
const ALTGR_LETTERS: &[(KeyCode, char)] = &[];
const TABLES: Tables = Tables {
    symbols: SYMBOLS,
    altgr_letters: ALTGR_LETTERS,
    numlock_period: '.',
};
#[derive(Debug, Clone, Copy, Default)]
pub struct LatinAmericaLayout;
impl KeyboardLayout for LatinAmericaLayout {
    fn map_keycode(
        &self,
        keycode: KeyCode,
        modifiers: &Modifiers,
        handle_ctrl: HandleControl,
    ) -> DecodedKey {
        shared::map_keycode(keycode, modifiers, handle_ctrl, &TABLES)
    }
    fn get_physical(&self) -> PhysicalKeyboard {
        PhysicalKeyboard::Iso
    }
}
