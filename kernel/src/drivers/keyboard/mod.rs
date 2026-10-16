pub mod es;
pub mod latam;
pub mod shared;
use crate::sync::IrqSafeSpinlock;
use lazy_static::lazy_static;
use pc_keyboard::{
    layouts, DecodedKey, HandleControl, KeyCode, KeyState, KeyboardLayout, Modifiers, PS2Keyboard,
    PhysicalKeyboard, ScancodeSet1,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum KeyboardLayoutKind {
    LatinAmerica = 0,
    Spain = 1,
    Us104 = 2,
    Uk105 = 3,
    De105 = 4,
    Azerty = 5,
    Colemak = 6,
    Dvorak104 = 7,
    Dvp104 = 8,
    FiSe105 = 9,
    No105 = 10,
    Jis109 = 11,
}
impl KeyboardLayoutKind {
    pub const ALL: &'static [KeyboardLayoutKind] = &[
        KeyboardLayoutKind::LatinAmerica,
        KeyboardLayoutKind::Spain,
        KeyboardLayoutKind::Us104,
        KeyboardLayoutKind::Uk105,
        KeyboardLayoutKind::De105,
        KeyboardLayoutKind::Azerty,
        KeyboardLayoutKind::Colemak,
        KeyboardLayoutKind::Dvorak104,
        KeyboardLayoutKind::Dvp104,
        KeyboardLayoutKind::FiSe105,
        KeyboardLayoutKind::No105,
        KeyboardLayoutKind::Jis109,
    ];
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::LatinAmerica),
            1 => Some(Self::Spain),
            2 => Some(Self::Us104),
            3 => Some(Self::Uk105),
            4 => Some(Self::De105),
            5 => Some(Self::Azerty),
            6 => Some(Self::Colemak),
            7 => Some(Self::Dvorak104),
            8 => Some(Self::Dvp104),
            9 => Some(Self::FiSe105),
            10 => Some(Self::No105),
            11 => Some(Self::Jis109),
            _ => None,
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        let n = name.trim();
        if n.eq_ignore_ascii_case("latam")
            || n.eq_ignore_ascii_case("la")
            || n.eq_ignore_ascii_case("es-latam")
            || n.eq_ignore_ascii_case("latin")
        {
            Some(Self::LatinAmerica)
        } else if n.eq_ignore_ascii_case("es")
            || n.eq_ignore_ascii_case("spain")
            || n.eq_ignore_ascii_case("es-es")
        {
            Some(Self::Spain)
        } else if n.eq_ignore_ascii_case("us")
            || n.eq_ignore_ascii_case("en-us")
            || n.eq_ignore_ascii_case("usa")
        {
            Some(Self::Us104)
        } else if n.eq_ignore_ascii_case("uk")
            || n.eq_ignore_ascii_case("en-uk")
            || n.eq_ignore_ascii_case("en-gb")
            || n.eq_ignore_ascii_case("gb")
        {
            Some(Self::Uk105)
        } else if n.eq_ignore_ascii_case("de")
            || n.eq_ignore_ascii_case("de-de")
            || n.eq_ignore_ascii_case("german")
        {
            Some(Self::De105)
        } else if n.eq_ignore_ascii_case("fr")
            || n.eq_ignore_ascii_case("fr-fr")
            || n.eq_ignore_ascii_case("azerty")
            || n.eq_ignore_ascii_case("french")
        {
            Some(Self::Azerty)
        } else if n.eq_ignore_ascii_case("colemak") {
            Some(Self::Colemak)
        } else if n.eq_ignore_ascii_case("dvorak") {
            Some(Self::Dvorak104)
        } else if n.eq_ignore_ascii_case("dvp") || n.eq_ignore_ascii_case("dvorak-programmer") {
            Some(Self::Dvp104)
        } else if n.eq_ignore_ascii_case("se")
            || n.eq_ignore_ascii_case("sv")
            || n.eq_ignore_ascii_case("fi")
            || n.eq_ignore_ascii_case("nordic")
        {
            Some(Self::FiSe105)
        } else if n.eq_ignore_ascii_case("no")
            || n.eq_ignore_ascii_case("norsk")
            || n.eq_ignore_ascii_case("norway")
        {
            Some(Self::No105)
        } else if n.eq_ignore_ascii_case("jp")
            || n.eq_ignore_ascii_case("jis")
            || n.eq_ignore_ascii_case("japan")
        {
            Some(Self::Jis109)
        } else {
            None
        }
    }
    pub fn code(&self) -> &'static str {
        match self {
            Self::LatinAmerica => "latam",
            Self::Spain => "es",
            Self::Us104 => "us",
            Self::Uk105 => "uk",
            Self::De105 => "de",
            Self::Azerty => "fr",
            Self::Colemak => "colemak",
            Self::Dvorak104 => "dvorak",
            Self::Dvp104 => "dvp",
            Self::FiSe105 => "se",
            Self::No105 => "no",
            Self::Jis109 => "jp",
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::LatinAmerica => "Español Latinoamérica (ISO-105)",
            Self::Spain => "Español España (ISO-105)",
            Self::Us104 => "English US (ANSI-104)",
            Self::Uk105 => "English UK (ISO-105)",
            Self::De105 => "Deutsch (ISO-105)",
            Self::Azerty => "Français AZERTY (ISO-105)",
            Self::Colemak => "Colemak (ANSI-104)",
            Self::Dvorak104 => "Dvorak (ANSI-104)",
            Self::Dvp104 => "Dvorak Programmer (ANSI-104)",
            Self::FiSe105 => "Svenska / Suomi (Nordic ISO-105)",
            Self::No105 => "Norsk (ISO-105)",
            Self::Jis109 => "Japanese JIS (JIS-109)",
        }
    }
}
pub enum LixLayout {
    Latam(latam::LatinAmericaLayout),
    Spain(es::SpainLayout),
    Builtin(layouts::AnyLayout),
}
impl LixLayout {
    pub fn from_kind(kind: KeyboardLayoutKind) -> Self {
        match kind {
            KeyboardLayoutKind::LatinAmerica => LixLayout::Latam(latam::LatinAmericaLayout),
            KeyboardLayoutKind::Spain => LixLayout::Spain(es::SpainLayout),
            KeyboardLayoutKind::Us104 => {
                LixLayout::Builtin(layouts::AnyLayout::Us104Key(layouts::Us104Key))
            }
            KeyboardLayoutKind::Uk105 => {
                LixLayout::Builtin(layouts::AnyLayout::Uk105Key(layouts::Uk105Key))
            }
            KeyboardLayoutKind::De105 => {
                LixLayout::Builtin(layouts::AnyLayout::De105Key(layouts::De105Key))
            }
            KeyboardLayoutKind::Azerty => {
                LixLayout::Builtin(layouts::AnyLayout::Azerty(layouts::Azerty))
            }
            KeyboardLayoutKind::Colemak => {
                LixLayout::Builtin(layouts::AnyLayout::Colemak(layouts::Colemak))
            }
            KeyboardLayoutKind::Dvorak104 => {
                LixLayout::Builtin(layouts::AnyLayout::Dvorak104Key(layouts::Dvorak104Key))
            }
            KeyboardLayoutKind::Dvp104 => {
                LixLayout::Builtin(layouts::AnyLayout::DVP104Key(layouts::DVP104Key))
            }
            KeyboardLayoutKind::FiSe105 => {
                LixLayout::Builtin(layouts::AnyLayout::FiSe105Key(layouts::FiSe105Key))
            }
            KeyboardLayoutKind::No105 => {
                LixLayout::Builtin(layouts::AnyLayout::No105Key(layouts::No105Key))
            }
            KeyboardLayoutKind::Jis109 => {
                LixLayout::Builtin(layouts::AnyLayout::Jis109Key(layouts::Jis109Key))
            }
        }
    }
}
impl KeyboardLayout for LixLayout {
    fn map_keycode(
        &self,
        keycode: KeyCode,
        modifiers: &Modifiers,
        handle_ctrl: HandleControl,
    ) -> DecodedKey {
        if modifiers.is_ctrl() {
            match keycode {
                KeyCode::Oem2 | KeyCode::OemMinus | KeyCode::NumpadSubtract => {
                    crate::drivers::framebuffer::zoom_out();
                    return DecodedKey::RawKey(KeyCode::ScrollLock);
                }
                KeyCode::Oem6 | KeyCode::OemPlus | KeyCode::NumpadAdd => {
                    crate::drivers::framebuffer::zoom_in();
                    return DecodedKey::RawKey(KeyCode::ScrollLock);
                }
                KeyCode::Key0 | KeyCode::Numpad0 => {
                    crate::drivers::framebuffer::zoom_reset();
                    return DecodedKey::RawKey(KeyCode::ScrollLock);
                }
                KeyCode::ArrowUp => {
                    crate::drivers::framebuffer::scroll_history_up(3);
                    return DecodedKey::RawKey(KeyCode::ScrollLock);
                }
                KeyCode::ArrowDown => {
                    crate::drivers::framebuffer::scroll_history_down(3);
                    return DecodedKey::RawKey(KeyCode::ScrollLock);
                }
                _ => {}
            }
        }
        match self {
            LixLayout::Latam(l) => l.map_keycode(keycode, modifiers, handle_ctrl),
            LixLayout::Spain(l) => l.map_keycode(keycode, modifiers, handle_ctrl),
            LixLayout::Builtin(l) => l.map_keycode(keycode, modifiers, handle_ctrl),
        }
    }
    fn get_physical(&self) -> PhysicalKeyboard {
        match self {
            LixLayout::Latam(l) => l.get_physical(),
            LixLayout::Spain(l) => l.get_physical(),
            LixLayout::Builtin(l) => l.get_physical(),
        }
    }
}
struct KeyboardSubsystem {
    kind: KeyboardLayoutKind,
    keyboard: PS2Keyboard<LixLayout, ScancodeSet1>,
}
lazy_static! {
    static ref KEYBOARD_SUBSYSTEM: IrqSafeSpinlock<KeyboardSubsystem> =
        IrqSafeSpinlock::new(KeyboardSubsystem {
            kind: KeyboardLayoutKind::LatinAmerica,
            keyboard: PS2Keyboard::new(
                ScancodeSet1::new(),
                LixLayout::from_kind(KeyboardLayoutKind::LatinAmerica),
                HandleControl::MapLettersToUnicode,
            ),
        });
}
pub fn set_layout(kind: KeyboardLayoutKind) {
    let mut sub = KEYBOARD_SUBSYSTEM.lock();
    sub.kind = kind;
    sub.keyboard = PS2Keyboard::new(
        ScancodeSet1::new(),
        LixLayout::from_kind(kind),
        HandleControl::MapLettersToUnicode,
    );
    crate::kprintln!(
        "[keyboard] Distribución de teclado configurada: {} ({})",
        kind.name(),
        kind.code()
    );
}
pub fn get_layout() -> KeyboardLayoutKind {
    KEYBOARD_SUBSYSTEM.lock().kind
}
pub fn process_scancode(scancode: u8) -> Option<(Option<DecodedKey>, u32, bool)> {
    let mut sub = KEYBOARD_SUBSYSTEM.lock();
    if let Ok(Some(ev)) = sub.keyboard.add_byte(scancode) {
        let code = hid_usage(ev.code);
        if code == 0 {
            return None;
        }
        let pressed = matches!(ev.state, KeyState::Down | KeyState::SingleShot);
        let decoded = sub.keyboard.process_keyevent(ev);
        Some((decoded, code, pressed))
    } else {
        None
    }
}
pub fn keycode_from_hid(usage: u32) -> Option<KeyCode> {
    match usage {
        0x04 => Some(KeyCode::A),
        0x05 => Some(KeyCode::B),
        0x06 => Some(KeyCode::C),
        0x07 => Some(KeyCode::D),
        0x08 => Some(KeyCode::E),
        0x09 => Some(KeyCode::F),
        0x0a => Some(KeyCode::G),
        0x0b => Some(KeyCode::H),
        0x0c => Some(KeyCode::I),
        0x0d => Some(KeyCode::J),
        0x0e => Some(KeyCode::K),
        0x0f => Some(KeyCode::L),
        0x10 => Some(KeyCode::M),
        0x11 => Some(KeyCode::N),
        0x12 => Some(KeyCode::O),
        0x13 => Some(KeyCode::P),
        0x14 => Some(KeyCode::Q),
        0x15 => Some(KeyCode::R),
        0x16 => Some(KeyCode::S),
        0x17 => Some(KeyCode::T),
        0x18 => Some(KeyCode::U),
        0x19 => Some(KeyCode::V),
        0x1a => Some(KeyCode::W),
        0x1b => Some(KeyCode::X),
        0x1c => Some(KeyCode::Y),
        0x1d => Some(KeyCode::Z),
        0x1e => Some(KeyCode::Key1),
        0x1f => Some(KeyCode::Key2),
        0x20 => Some(KeyCode::Key3),
        0x21 => Some(KeyCode::Key4),
        0x22 => Some(KeyCode::Key5),
        0x23 => Some(KeyCode::Key6),
        0x24 => Some(KeyCode::Key7),
        0x25 => Some(KeyCode::Key8),
        0x26 => Some(KeyCode::Key9),
        0x27 => Some(KeyCode::Key0),
        0x28 => Some(KeyCode::Return),
        0x29 => Some(KeyCode::Escape),
        0x2a => Some(KeyCode::Backspace),
        0x2b => Some(KeyCode::Tab),
        0x2c => Some(KeyCode::Spacebar),
        0x2d => Some(KeyCode::OemMinus),
        0x2e => Some(KeyCode::OemPlus),
        0x2f => Some(KeyCode::Oem4),
        0x30 => Some(KeyCode::Oem6),
        0x31 => Some(KeyCode::Oem5),
        0x33 => Some(KeyCode::Oem1),
        0x34 => Some(KeyCode::Oem7),
        0x35 => Some(KeyCode::Oem3),
        0x36 => Some(KeyCode::OemComma),
        0x37 => Some(KeyCode::OemPeriod),
        0x38 => Some(KeyCode::Oem2),
        0x39 => Some(KeyCode::CapsLock),
        0x3a => Some(KeyCode::F1),
        0x3b => Some(KeyCode::F2),
        0x3c => Some(KeyCode::F3),
        0x3d => Some(KeyCode::F4),
        0x3e => Some(KeyCode::F5),
        0x3f => Some(KeyCode::F6),
        0x40 => Some(KeyCode::F7),
        0x41 => Some(KeyCode::F8),
        0x42 => Some(KeyCode::F9),
        0x43 => Some(KeyCode::F10),
        0x44 => Some(KeyCode::F11),
        0x45 => Some(KeyCode::F12),
        0x46 => Some(KeyCode::PrintScreen),
        0x47 => Some(KeyCode::ScrollLock),
        0x48 => Some(KeyCode::PauseBreak),
        0x49 => Some(KeyCode::Insert),
        0x4a => Some(KeyCode::Home),
        0x4b => Some(KeyCode::PageUp),
        0x4c => Some(KeyCode::Delete),
        0x4d => Some(KeyCode::End),
        0x4e => Some(KeyCode::PageDown),
        0x4f => Some(KeyCode::ArrowRight),
        0x50 => Some(KeyCode::ArrowLeft),
        0x51 => Some(KeyCode::ArrowDown),
        0x52 => Some(KeyCode::ArrowUp),
        0x53 => Some(KeyCode::NumpadLock),
        0x54 => Some(KeyCode::NumpadDivide),
        0x55 => Some(KeyCode::NumpadMultiply),
        0x56 => Some(KeyCode::NumpadSubtract),
        0x57 => Some(KeyCode::NumpadAdd),
        0x58 => Some(KeyCode::NumpadEnter),
        0x59 => Some(KeyCode::Numpad1),
        0x5a => Some(KeyCode::Numpad2),
        0x5b => Some(KeyCode::Numpad3),
        0x5c => Some(KeyCode::Numpad4),
        0x5d => Some(KeyCode::Numpad5),
        0x5e => Some(KeyCode::Numpad6),
        0x5f => Some(KeyCode::Numpad7),
        0x60 => Some(KeyCode::Numpad8),
        0x61 => Some(KeyCode::Numpad9),
        0x62 => Some(KeyCode::Numpad0),
        0x63 => Some(KeyCode::NumpadPeriod),
        0x64 => Some(KeyCode::Oem5),
        0xe0 => Some(KeyCode::LControl),
        0xe1 => Some(KeyCode::LShift),
        0xe2 => Some(KeyCode::LAlt),
        0xe3 => Some(KeyCode::LWin),
        0xe4 => Some(KeyCode::RControl),
        0xe5 => Some(KeyCode::RShift),
        0xe6 => Some(KeyCode::RAltGr),
        0xe7 => Some(KeyCode::RWin),
        _ => None,
    }
}
pub fn process_hid_key(usage: u32, pressed: bool) -> Option<DecodedKey> {
    let keycode = keycode_from_hid(usage)?;
    let mut sub = KEYBOARD_SUBSYSTEM.lock();
    let state = if pressed {
        KeyState::Down
    } else {
        KeyState::Up
    };
    let event = pc_keyboard::KeyEvent::new(keycode, state);
    sub.keyboard.process_keyevent(event)
}
pub fn dispatch_decoded_key(key: DecodedKey) {
    match key {
        DecodedKey::Unicode(character) => {
            crate::drivers::tty::push_char(character);
        }
        DecodedKey::RawKey(raw) => match raw {
            KeyCode::Backspace => {
                crate::drivers::tty::push_char('\x08');
            }
            KeyCode::ArrowUp => {
                crate::drivers::tty::push_str("\x1b[A");
            }
            KeyCode::ArrowDown => {
                crate::drivers::tty::push_str("\x1b[B");
            }
            KeyCode::ArrowRight => {
                crate::drivers::tty::push_str("\x1b[C");
            }
            KeyCode::ArrowLeft => {
                crate::drivers::tty::push_str("\x1b[D");
            }
            KeyCode::Home => {
                crate::drivers::tty::push_str("\x1b[H");
            }
            KeyCode::End => {
                crate::drivers::tty::push_str("\x1b[F");
            }
            KeyCode::Delete => {
                crate::drivers::tty::push_str("\x1b[3~");
            }
            KeyCode::NumpadSubtract => {
                crate::drivers::tty::push_char('-');
            }
            KeyCode::NumpadAdd => {
                crate::drivers::tty::push_char('+');
            }
            KeyCode::NumpadMultiply => {
                crate::drivers::tty::push_char('*');
            }
            KeyCode::NumpadDivide => {
                crate::drivers::tty::push_char('/');
            }
            KeyCode::NumpadEnter => {
                crate::drivers::tty::push_char('\n');
            }
            _ => {}
        },
    }
}
fn hid_usage(code: KeyCode) -> u32 {
    match code {
        KeyCode::A => 0x04,
        KeyCode::B => 0x05,
        KeyCode::C => 0x06,
        KeyCode::D => 0x07,
        KeyCode::E => 0x08,
        KeyCode::F => 0x09,
        KeyCode::G => 0x0a,
        KeyCode::H => 0x0b,
        KeyCode::I => 0x0c,
        KeyCode::J => 0x0d,
        KeyCode::K => 0x0e,
        KeyCode::L => 0x0f,
        KeyCode::M => 0x10,
        KeyCode::N => 0x11,
        KeyCode::O => 0x12,
        KeyCode::P => 0x13,
        KeyCode::Q => 0x14,
        KeyCode::R => 0x15,
        KeyCode::S => 0x16,
        KeyCode::T => 0x17,
        KeyCode::U => 0x18,
        KeyCode::V => 0x19,
        KeyCode::W => 0x1a,
        KeyCode::X => 0x1b,
        KeyCode::Y => 0x1c,
        KeyCode::Z => 0x1d,
        KeyCode::Key1 => 0x1e,
        KeyCode::Key2 => 0x1f,
        KeyCode::Key3 => 0x20,
        KeyCode::Key4 => 0x21,
        KeyCode::Key5 => 0x22,
        KeyCode::Key6 => 0x23,
        KeyCode::Key7 => 0x24,
        KeyCode::Key8 => 0x25,
        KeyCode::Key9 => 0x26,
        KeyCode::Key0 => 0x27,
        KeyCode::Return | KeyCode::NumpadEnter => 0x28,
        KeyCode::Escape => 0x29,
        KeyCode::Backspace => 0x2a,
        KeyCode::Tab => 0x2b,
        KeyCode::Spacebar => 0x2c,
        KeyCode::OemMinus => 0x2d,
        KeyCode::OemPlus => 0x2e,
        KeyCode::Oem4 => 0x2f,
        KeyCode::Oem6 => 0x30,
        KeyCode::Oem5 => 0x31,
        KeyCode::Oem1 => 0x33,
        KeyCode::Oem7 => 0x34,
        KeyCode::Oem3 => 0x35,
        KeyCode::OemComma => 0x36,
        KeyCode::OemPeriod => 0x37,
        KeyCode::Oem2 => 0x38,
        KeyCode::CapsLock => 0x39,
        KeyCode::F1 => 0x3a,
        KeyCode::F2 => 0x3b,
        KeyCode::F3 => 0x3c,
        KeyCode::F4 => 0x3d,
        KeyCode::F5 => 0x3e,
        KeyCode::F6 => 0x3f,
        KeyCode::F7 => 0x40,
        KeyCode::F8 => 0x41,
        KeyCode::F9 => 0x42,
        KeyCode::F10 => 0x43,
        KeyCode::F11 => 0x44,
        KeyCode::F12 => 0x45,
        KeyCode::PrintScreen => 0x46,
        KeyCode::ScrollLock => 0x47,
        KeyCode::PauseBreak => 0x48,
        KeyCode::Insert => 0x49,
        KeyCode::Home => 0x4a,
        KeyCode::PageUp => 0x4b,
        KeyCode::Delete => 0x4c,
        KeyCode::End => 0x4d,
        KeyCode::PageDown => 0x4e,
        KeyCode::ArrowRight => 0x4f,
        KeyCode::ArrowLeft => 0x50,
        KeyCode::ArrowDown => 0x51,
        KeyCode::ArrowUp => 0x52,
        KeyCode::NumpadLock => 0x53,
        KeyCode::NumpadDivide => 0x54,
        KeyCode::NumpadMultiply => 0x55,
        KeyCode::NumpadSubtract => 0x56,
        KeyCode::NumpadAdd => 0x57,
        KeyCode::Numpad1 => 0x59,
        KeyCode::Numpad2 => 0x5a,
        KeyCode::Numpad3 => 0x5b,
        KeyCode::Numpad4 => 0x5c,
        KeyCode::Numpad5 => 0x5d,
        KeyCode::Numpad6 => 0x5e,
        KeyCode::Numpad7 => 0x5f,
        KeyCode::Numpad8 => 0x60,
        KeyCode::Numpad9 => 0x61,
        KeyCode::Numpad0 => 0x62,
        KeyCode::NumpadPeriod => 0x63,
        KeyCode::LControl => 0xe0,
        KeyCode::LShift => 0xe1,
        KeyCode::LAlt => 0xe2,
        KeyCode::LWin => 0xe3,
        KeyCode::RControl => 0xe4,
        KeyCode::RShift => 0xe5,
        KeyCode::RAltGr => 0xe6,
        KeyCode::RWin => 0xe7,
        _ => 0,
    }
}
pub fn load_config_from_vfs() {
    let content_opt = crate::fs::vfs::read_file("/etc/keyboard.conf")
        .or_else(|| crate::fs::vfs::read_file("/etc/vconsole.conf"));
    if let Some(bytes) = content_opt {
        if let Ok(text) = core::str::from_utf8(&bytes) {
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('#') || trimmed.is_empty() {
                    continue;
                }
                let val_opt = trimmed
                    .strip_prefix("KEYMAP=")
                    .or_else(|| trimmed.strip_prefix("keymap="))
                    .or_else(|| trimmed.strip_prefix("LAYOUT="))
                    .or_else(|| trimmed.strip_prefix("layout="));
                if let Some(val) = val_opt {
                    let keymap_name = val.trim().trim_matches('"').trim_matches('\'');
                    if let Some(kind) = KeyboardLayoutKind::from_name(keymap_name) {
                        crate::kprintln!(
                            "[keyboard] Configuración cargada desde /etc/keyboard.conf: {}",
                            keymap_name
                        );
                        set_layout(kind);
                        return;
                    } else {
                        crate::kprintln!(
                            "[keyboard] AVISO: Distribución '{}' desconocida en config",
                            keymap_name
                        );
                    }
                }
            }
        }
    }
}
