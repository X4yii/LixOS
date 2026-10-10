use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SyscallDef {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub rights: String,
}

pub fn run() {
    println!("[xtask codegen] Procesando abi/syscalls.toml...");
    let toml_path = Path::new("abi/syscalls.toml");
    let content = match fs::read_to_string(toml_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[xtask codegen] Error leyendo {}: {e}", toml_path.display());
            std::process::exit(1);
        }
    };

    let syscalls = parse_syscalls(&content);
    println!("[xtask codegen] {} syscalls encontradas en abi/syscalls.toml.", syscalls.len());

    generate_kernel_syscall_table(&syscalls);
    generate_c_header(&syscalls);
    println!("[xtask codegen] Generación de código de ABI finalizada con éxito.");
}

fn parse_syscalls(content: &str) -> Vec<SyscallDef> {
    let mut defs = Vec::new();
    let mut current_id = None;
    let mut current_name = None;
    let mut current_desc = None;
    let mut current_rights = None;

    let flush_entry = |defs: &mut Vec<SyscallDef>,
                       id: &mut Option<u64>,
                       name: &mut Option<String>,
                       desc: &mut Option<String>,
                       rights: &mut Option<String>| {
        if let (Some(id_val), Some(name_val)) = (id.take(), name.take()) {
            defs.push(SyscallDef {
                id: id_val,
                name: name_val,
                description: desc.take().unwrap_or_default(),
                rights: rights.take().unwrap_or_else(|| "NONE".to_string()),
            });
        }
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[[syscall]]" {
            flush_entry(&mut defs, &mut current_id, &mut current_name, &mut current_desc, &mut current_rights);
            continue;
        }

        if let Some((k, v)) = trimmed.split_once('=') {
            let key = k.trim();
            let val = v.trim();
            match key {
                "id" => {
                    let num_str = val.trim_start_matches("0x");
                    if val.starts_with("0x") {
                        current_id = u64::from_str_radix(num_str, 16).ok();
                    } else {
                        current_id = val.parse::<u64>().ok();
                    }
                }
                "name" => {
                    current_name = Some(val.trim_matches('"').to_string());
                }
                "description" => {
                    current_desc = Some(val.trim_matches('"').to_string());
                }
                "rights" => {
                    current_rights = Some(val.trim_matches('"').to_string());
                }
                _ => {}
            }
        }
    }

    flush_entry(&mut defs, &mut current_id, &mut current_name, &mut current_desc, &mut current_rights);
    defs
}

fn generate_kernel_syscall_table(syscalls: &[SyscallDef]) {
    let mut out = String::new();
    out.push_str("//! ARCHIVO GENERADO AUTOMÁTICAMENTE POR `cargo xtask codegen`\n");
    out.push_str("//! NO EDITAR DIRECTAMENTE. Fuente de la verdad: `abi/syscalls.toml`.\n\n");
    out.push_str("#[derive(Debug, Clone, Copy)]\n");
    out.push_str("pub struct SyscallEntry {\n");
    out.push_str("    pub id: u64,\n");
    out.push_str("    pub name: &'static str,\n");
    out.push_str("    pub description: &'static str,\n");
    out.push_str("    pub required_rights: &'static str,\n");
    out.push_str("}\n\n");

    out.push_str("pub const SYSCALL_REGISTRY: &[SyscallEntry] = &[\n");
    for s in syscalls {
        out.push_str(&format!(
            "    SyscallEntry {{ id: {:#x}, name: {:?}, description: {:?}, required_rights: {:?} }},\n",
            s.id, s.name, s.description, s.rights
        ));
    }
    out.push_str("];\n\n");

    out.push_str("/// Consulta el nombre y descripción de una syscall por su número de llamada.\n");
    out.push_str("pub fn lookup_syscall(id: u64) -> Option<&'static SyscallEntry> {\n");
    out.push_str("    SYSCALL_REGISTRY.iter().find(|e| e.id == id)\n");
    out.push_str("}\n");

    let dest = Path::new("kernel/src/task/syscall_table.rs");
    if let Err(e) = fs::write(dest, out) {
        eprintln!("[xtask codegen] Error escribiendo {}: {e}", dest.display());
    } else {
        println!("[xtask codegen] {} generado exitosamente.", dest.display());
    }
}

fn generate_c_header(syscalls: &[SyscallDef]) {
    let mut out = String::new();
    out.push_str("/* ARCHIVO GENERADO AUTOMÁTICAMENTE POR `cargo xtask codegen` */\n");
    out.push_str("/* NO EDITAR DIRECTAMENTE. Fuente de la verdad: `abi/syscalls.toml`. */\n");
    out.push_str("#ifndef _LIX_SYSCALL_H\n");
    out.push_str("#define _LIX_SYSCALL_H\n\n");
    out.push_str("#include <stdint.h>\n\n");

    for s in syscalls {
        let macro_name = s.name.to_ascii_uppercase();
        out.push_str(&format!("/* {} */\n", s.description));
        out.push_str(&format!("#define {} {:#x}\n\n", macro_name, s.id));
    }

    out.push_str("#endif /* _LIX_SYSCALL_H */\n");

    let dir = Path::new("include/lix");
    let _ = fs::create_dir_all(dir);
    let dest = dir.join("syscall.h");
    if let Err(e) = fs::write(&dest, out) {
        eprintln!("[xtask codegen] Error escribiendo {}: {e}", dest.display());
    } else {
        println!("[xtask codegen] {} generado exitosamente.", dest.display());
    }
}
