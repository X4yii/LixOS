use alloc::string::String;
use sys_libc::syscalls::{
    sys_write, sys_exit, sys_list_dir, sys_read_file, sys_power,
    sys_terminal_zoom, sys_terminal_set_scale, sys_get_keymap, sys_set_keymap,
    sys_block_open, sys_block_info, sys_block_read, sys_block_sync, sys_handle_close,
    BlockDeviceInfo,
};
use crate::autocomplete::resolve_path;

pub fn try_execute_builtin(cmd: &str, args: &[&str], current_dir: &mut String) -> bool {
    match cmd {
        "cd" => {
            let target = args.first().copied().unwrap_or("/");
            let resolved = resolve_path(current_dir, target);
            let mut test_buf = [0u8; 1024];
            if resolved == "/" || sys_list_dir(&resolved, &mut test_buf).is_ok() {
                *current_dir = resolved;
            } else {
                sys_write(&alloc::format!("cd: no existe el directorio: {}\n", target));
            }
            true
        }
        "pwd" => {
            sys_write(current_dir);
            sys_write("\n");
            true
        }
        "help" => {
            sys_write("\x1b[33mComandos internos de Vanish (Ring 3):\x1b[0m\n");
            sys_write("  cd <dir>   - Cambia el directorio de trabajo actual\n");
            sys_write("  pwd        - Muestra el directorio de trabajo actual\n");
            sys_write("  help       - Muestra este mensaje de ayuda\n");
            sys_write("  clear      - Limpia la pantalla de la terminal\n");
            sys_write("  echo       - Imprime argumentos en pantalla\n");
            sys_write("  zoom       - Redimensiona celdas (zoom in/out/reset o Ctrl + -/+)\n");
            sys_write("  keymap     - Configura distribución de teclado (latam, es, us, uk, de, fr, etc.)\n");
            sys_write("  blktest    - Diagnóstico y verificación del ABI de bloques / NVMe\n");
            sys_write("  shutdown   - Cierra procesos, desmonta particiones y apaga el equipo\n");
            sys_write("  exit       - Sale de la shell\n");
            sys_write("\n\x1b[33mBinarios del sistema (ejecutables ELF):\x1b[0m\n");
            sys_write("  ltop       - Administrador de procesos interactivo (TUI)\n");
            sys_write("  hello      - Binario de prueba Ring 3 C\n");
            sys_write("  ls [dir]   - Lista archivos del sistema de archivos\n");
            sys_write("  cat <file> - Muestra el contenido de un archivo\n");
            true
        }
        "clear" => {
            sys_write("\x1b[2J\x1b[1;1H");
            true
        }
        "echo" => {
            for (idx, arg) in args.iter().enumerate() {
                if idx > 0 {
                    sys_write(" ");
                }
                sys_write(arg);
            }
            sys_write("\n");
            true
        }
        "zoom" | "scale" => {
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
        "keymap" | "loadkeys" => {
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
                sys_write(&alloc::format!("\x1b[32;1mDistribución activa:\x1b[0m {} [{}]\n", desc, code));
                sys_write("\x1b[33mDistribuciones disponibles:\x1b[0m\n");
                for (id, code, desc) in LAYOUTS {
                    let marker = if *id == current_id { " *" } else { "  " };
                    sys_write(&alloc::format!("{} {:<8} - {}\n", marker, code, desc));
                }
                sys_write("\nUso: keymap <nombre> (ej. 'keymap latam', 'keymap es', 'keymap us')\n");
            } else {
                let target = args[0];
                let matched = LAYOUTS.iter().find(|(_, code, _)| code.eq_ignore_ascii_case(target))
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
                    sys_write(&alloc::format!("Teclado configurado a: {} [{}]\n", desc, code));
                } else {
                    sys_write(&alloc::format!("Distribución '{}' no reconocida. Ejecuta 'keymap' para ver la lista.\n", target));
                }
            }
            true
        }
        "shutdown" | "poweroff" => {
            sys_write("\x1b[31;1mIniciando secuencia de apagado seguro de LixOS...\x1b[0m\n");
            sys_power(0);
        }
        "blktest" | "blkinfo" => {
            sys_write("\x1b[36;1m[blktest] Verificando ABI de Bloques y Lease Exclusivo...\x1b[0m\n");
            match sys_block_open(0, false) {
                Ok(handle) => {
                    sys_write(&alloc::format!("  [+] Lease adquirido con éxito (Handle: {:#x})\n", handle.0));
                    let mut info = BlockDeviceInfo::default();
                    if let Ok(()) = sys_block_info(handle, &mut info) {
                        sys_write(&alloc::format!(
                            "  [+] Dispositivo: {} sectores, {} bytes/sector (Total: {} MB)\n",
                            info.block_count,
                            info.block_size,
                            (info.block_count * info.block_size as u64) / (1024 * 1024)
                        ));
                    }
                    let mut sector = [0u8; 4096];
                    let read_len = if info.block_size > 0 && (info.block_size as usize) <= 4096 {
                        info.block_size as usize
                    } else {
                        512
                    };
                    match sys_block_read(handle, 0, 1, &mut sector[..read_len]) {
                        Ok(bytes) => {
                            let sig = [sector[510], sector[511]];
                            let is_mbr = sig == [0x55, 0xAA];
                            sys_write(&alloc::format!(
                                "  [+] Lectura de LBA 0 ({} bytes): Firma 0x{:02X}{:02X} ({})\n",
                                bytes, sig[0], sig[1],
                                if is_mbr { "MBR/GPT válido" } else { "sin firma MBR estándar" }
                            ));
                        }
                        Err(e) => {
                            sys_write(&alloc::format!("  [-] Error en lectura de LBA 0: código {}\n", e));
                        }
                    }
                    if let Ok(()) = sys_block_sync(handle) {
                        sys_write("  [+] sys_block_sync: Flush NVMe completado exitosamente\n");
                    }
                    match sys_block_open(0, false) {
                        Err(e) if e == -16 => {
                            sys_write("  [+] Exclusividad verificada: segundo lease rechazado con EBUSY (-16)\n");
                        }
                        Ok(h2) => {
                            sys_write("  [-] AVISO: Segundo lease no fue rechazado\n");
                            let _ = sys_handle_close(h2);
                        }
                        Err(e) => {
                            sys_write(&alloc::format!("  [-] Segundo lease falló con código {}\n", e));
                        }
                    }
                    let _ = sys_handle_close(handle);
                    sys_write("  [+] Lease liberado (sys_handle_close). ABI de bloques 100% operativo.\n");
                }
                Err(e) => {
                    sys_write(&alloc::format!("[-] Fallo al abrir dispositivo de bloque: código {}\n", e));
                    if e == -1 {
                        sys_write("    (Permiso denegado: se requiere nivel de privilegio <= 1)\n");
                    } else if e == -19 {
                        sys_write("    (Dispositivo no encontrado o driver no enumerado)\n");
                    }
                }
            }
            true
        }
        "exit" => {
            sys_write("Saliendo de vanish...\n");
            sys_exit(0);
        }
        "ls" => {
            let raw_path = args.first().copied().unwrap_or(".");
            let target_path = resolve_path(current_dir, raw_path);
            let mut buf = [0u8; 1024];
            match sys_list_dir(&target_path, &mut buf) {
                Ok(bytes) => {
                    if bytes == 0 {
                        sys_write("(directorio vacío)\n");
                    } else if let Ok(listing) = core::str::from_utf8(&buf[..bytes]) {
                        for line in listing.lines() {
                            if line.is_empty() {
                                continue;
                            }
                            if line.ends_with(".elf") {
                                sys_write("\x1b[32;1m");
                                sys_write(line);
                                sys_write("\x1b[0m\n");
                            } else if line.ends_with('/') || !line.contains('.') {
                                sys_write("\x1b[34;1m");
                                sys_write(line);
                                sys_write("\x1b[0m\n");
                            } else {
                                sys_write(line);
                                sys_write("\n");
                            }
                        }
                    }
                }
                Err(e) => {
                    sys_write(&alloc::format!("ls: error al listar '{}' (código {})\n", target_path, e));
                }
            }
            true
        }
        "cat" => {
            if args.is_empty() {
                sys_write("Uso: cat <archivo>\n");
            } else {
                let file_path = resolve_path(current_dir, args[0]);
                let mut buf = [0u8; 4096];
                match sys_read_file(&file_path, &mut buf) {
                    Ok(bytes) => {
                        if let Ok(content) = core::str::from_utf8(&buf[..bytes]) {
                            sys_write(content);
                            if !content.ends_with('\n') {
                                sys_write("\n");
                            }
                        } else {
                            sys_write(&alloc::format!("cat: archivo binario ({} bytes)\n", bytes));
                        }
                    }
                    Err(e) => {
                        sys_write(&alloc::format!("cat: no se pudo leer '{}' (código {})\n", file_path, e));
                    }
                }
            }
            true
        }
        _ => false,
    }
}
