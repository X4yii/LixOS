use super::{COMMANDS, SYSTEM_BINARIES};
use alloc::string::String;
use sys_libc::syscalls::{
    sys_exit, sys_get_keymap, sys_power, sys_process_list, sys_read, sys_set_keymap,
    sys_terminal_set_scale, sys_terminal_zoom, sys_write, ProcessInfo,
};
pub fn help(_args: &[&str], _current_dir: &mut String) -> bool {
    sys_write("\x1b[33mComandos internos de Vanish (Ring 3):\x1b[0m\n");
    for command in COMMANDS {
        let mut label = String::from(command.name);
        if !command.usage.is_empty() {
            label.push(' ');
            label.push_str(command.usage);
        }
        sys_write(&alloc::format!("  {:<11} - {}\n", label, command.help));
    }
    sys_write("\n\x1b[33mBinarios del sistema (ejecutables ELF):\x1b[0m\n");
    for (name, description) in SYSTEM_BINARIES {
        sys_write(&alloc::format!("  {name:<11} - {description}\n"));
    }
    true
}
pub fn clear(_args: &[&str], _current_dir: &mut String) -> bool {
    sys_write("\x1b[2J\x1b[1;1H");
    true
}
pub fn echo(args: &[&str], _current_dir: &mut String) -> bool {
    for (idx, arg) in args.iter().enumerate() {
        if idx > 0 {
            sys_write(" ");
        }
        sys_write(arg);
    }
    sys_write("\n");
    true
}
pub fn zoom(args: &[&str], _current_dir: &mut String) -> bool {
    let target = args.first().copied().unwrap_or("in");
    match target {
        "+" | "in" | "plus" => {
            sys_terminal_zoom(1);
            sys_write("Zoom aumentado (+).\n");
        }
        "-" | "out" | "minus" => {
            sys_terminal_zoom(-1);
            sys_write("Zoom disminuido (-).\n");
        }
        "reset" | "0" | "default" => {
            sys_terminal_set_scale(1);
            sys_write("Escala restablecida a 1x.\n");
        }
        "1" => {
            sys_terminal_set_scale(1);
            sys_write("Escala fijada en 1x (alta densidad).\n");
        }
        "2" => {
            sys_terminal_set_scale(2);
            sys_write("Escala fijada en 2x (estándar).\n");
        }
        "3" => {
            sys_terminal_set_scale(3);
            sys_write("Escala fijada en 3x (grande).\n");
        }
        "4" => {
            sys_terminal_set_scale(4);
            sys_write("Escala fijada en 4x (extra grande).\n");
        }
        _ => {
            sys_write("Uso: zoom [in|out|reset|1|2|3|4] o atajos Ctrl + (-/+)\n");
        }
    }
    true
}
pub fn keymap(args: &[&str], _current_dir: &mut String) -> bool {
    const LAYOUTS: &[(u8, &str, &str)] = &[
        (0, "latam", "Español Latinoamérica (ISO-105)"),
        (1, "es", "Español España (ISO-105)"),
        (2, "us", "English US (ANSI-104)"),
        (3, "uk", "English UK (ISO-105)"),
        (4, "de", "Deutsch (ISO-105)"),
        (5, "fr", "Français AZERTY (ISO-105)"),
        (6, "colemak", "Colemak (ANSI-104)"),
        (7, "dvorak", "Dvorak (ANSI-104)"),
        (8, "dvp", "Dvorak Programmer (ANSI-104)"),
        (9, "se", "Svenska / Suomi (Nordic ISO-105)"),
        (10, "no", "Norsk (ISO-105)"),
        (11, "jp", "Japanese JIS (JIS-109)"),
    ];
    if args.is_empty() {
        let current_id = sys_get_keymap();
        let current_info = LAYOUTS.iter().find(|(id, _, _)| *id == current_id);
        let (code, desc) = match current_info {
            Some((_, c, d)) => (*c, *d),
            None => ("desconocido", "Desconocido"),
        };
        sys_write(&alloc::format!(
            "\x1b[32;1mDistribución activa:\x1b[0m {desc} [{code}]\n"
        ));
        sys_write("\x1b[33mDistribuciones disponibles:\x1b[0m\n");
        for (id, code, desc) in LAYOUTS {
            let marker = if *id == current_id { " *" } else { "  " };
            sys_write(&alloc::format!("{marker} {code:<8} - {desc}\n"));
        }
        sys_write("\nUso: keymap <nombre> (ej. 'keymap latam', 'keymap es', 'keymap us')\n");
    } else {
        let target = args[0];
        let matched = LAYOUTS
            .iter()
            .find(|(_, code, _)| code.eq_ignore_ascii_case(target))
            .or_else(|| match target {
                "la" | "es-latam" | "latin" => Some(&LAYOUTS[0]),
                "spain" | "es-es" => Some(&LAYOUTS[1]),
                "en" | "en-us" | "usa" => Some(&LAYOUTS[2]),
                "en-uk" | "en-gb" | "gb" => Some(&LAYOUTS[3]),
                "german" | "de-de" => Some(&LAYOUTS[4]),
                "azerty" | "french" => Some(&LAYOUTS[5]),
                "sv" | "fi" | "nordic" => Some(&LAYOUTS[9]),
                "norway" | "norsk" => Some(&LAYOUTS[10]),
                "jis" | "japan" => Some(&LAYOUTS[11]),
                _ => None,
            });
        if let Some((id, code, desc)) = matched {
            sys_set_keymap(*id);
            sys_write(&alloc::format!("Teclado configurado a: {desc} [{code}]\n"));
        } else {
            sys_write(&alloc::format!(
                "Distribución '{target}' no reconocida. Ejecuta 'keymap' para ver la lista.\n"
            ));
        }
    }
    true
}
pub fn shutdown(_args: &[&str], _current_dir: &mut String) -> bool {
    sys_write("Apagar el sistema ahora? [s/N] ");
    let mut confirmation = [0u8; 1];
    let answered = sys_read(0, &mut confirmation).unwrap_or(0);
    if answered == 1 && matches!(confirmation[0], b's' | b'S' | b'y' | b'Y') {
        sys_write("\x1b[31;1mIniciando secuencia de apagado seguro de LixOS...\x1b[0m\n");
        sys_power(0);
    }
    sys_write("\nApagado cancelado.\n");
    true
}
pub fn exit(_args: &[&str], _current_dir: &mut String) -> bool {
    sys_write("Saliendo de vanish...\n");
    sys_exit(0);
}
const fn state_label(state: u8) -> &'static str {
    match state {
        0 => "Ready",
        1 => "Running",
        2 => "Blocked",
        3 => "Sleep",
        4 => "Zombie",
        5 => "Dead",
        _ => "?",
    }
}
/// `ps`: lista los procesos del sistema.
///
/// Es la comprobación en vivo de la política de derechos: `sys_process_list`
/// exige `INSPECT`, que `rights::declared` sólo concede a los binarios de
/// sistema (`SYSTEM`, que la shell posee porque su binario declara `SESSION`).
/// Si la política se rompiera y `vanish` cayera a `BASE_USER`, este comando
/// imprimiría `ps: EACCES` en lugar de la tabla.
pub fn ps(_args: &[&str], _current_dir: &mut String) -> bool {
    const MAX: usize = 32;
    let mut infos = [ProcessInfo::default(); MAX];
    let count = sys_process_list(infos.as_mut_ptr(), infos.len());
    if count == 0 || count > MAX {
        sys_write("ps: EACCES o sin procesos visibles\n");
        return false;
    }
    sys_write("  PID ESTADO   PRIV   TICKS  MEM  NOMBRE\n");
    for info in infos[..count].iter() {
        let raw = &info.name;
        let len = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
        let name = core::str::from_utf8(&raw[..len]).unwrap_or("?");
        sys_write(&alloc::format!(
            "{:>4} {:<8} {:>4} {:>7} {:>5}  {}\n",
            info.pid,
            state_label(info.state),
            info.privilege_level,
            info.cpu_ticks,
            info.ram_bytes,
            name
        ));
    }
    true
}
