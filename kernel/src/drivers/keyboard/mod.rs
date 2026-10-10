pub mod latam;
pub mod es;

use pc_keyboard::{
    layouts, DecodedKey, HandleControl, KeyCode, KeyboardLayout, Modifiers, PS2Keyboard,
    PhysicalKeyboard, ScancodeSet1,
};
use crate::sync::IrqSafeSpinlock;
use lazy_static::lazy_static;

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
        } else if n.eq_ignore_ascii_case("dvp")
            || n.eq_ignore_ascii_case("dvorak-programmer")
        {
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
                    return DecodedKey::Unicode('\x1f');
                }
                KeyCode::Oem6 | KeyCode::OemPlus | KeyCode::NumpadAdd => {
                    return DecodedKey::Unicode('\x1e');
                }
                KeyCode::Key0 | KeyCode::Numpad0 => {
                    return DecodedKey::Unicode('\x1c');
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

pub fn process_scancode(scancode: u8) -> Option<DecodedKey> {
    let mut sub = KEYBOARD_SUBSYSTEM.lock();
    if let Ok(Some(ev)) = sub.keyboard.add_byte(scancode) {
        sub.keyboard.process_keyevent(ev)
    } else {
        None
    }
}

/// Lee la configuración de teclado desde el VFS (`/etc/keyboard.conf` o `/etc/vconsole.conf`).
/// Si existe una clave `KEYMAP=nombre`, aplica la distribución de forma transparente.
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
