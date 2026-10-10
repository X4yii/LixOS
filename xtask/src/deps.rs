use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Distro {
    Arch,
    Debian,
    Fedora,
    OpenSuse,
    Alpine,
    Unknown,
}

pub fn detect_distro() -> Distro {
    let content = fs::read_to_string("/etc/os-release")
        .or_else(|_| fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();

    let mut id = String::new();
    let mut id_like = String::new();

    for line in content.lines() {
        if let Some(val) = line.strip_prefix("ID=") {
            id = val.trim_matches('"').trim_matches('\'').to_lowercase();
        } else if let Some(val) = line.strip_prefix("ID_LIKE=") {
            id_like = val.trim_matches('"').trim_matches('\'').to_lowercase();
        }
    }

    let combined = format!("{id} {id_like}");

    if combined.contains("arch") || combined.contains("manjaro") || combined.contains("endeavouros") {
        Distro::Arch
    } else if combined.contains("debian") || combined.contains("ubuntu") || combined.contains("pop") || combined.contains("mint") {
        Distro::Debian
    } else if combined.contains("fedora") || combined.contains("rhel") || combined.contains("centos") || combined.contains("rocky") || combined.contains("alma") {
        Distro::Fedora
    } else if combined.contains("suse") {
        Distro::OpenSuse
    } else if combined.contains("alpine") {
        Distro::Alpine
    } else {
        Distro::Unknown
    }
}

pub fn is_command_available(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn is_ovmf_available() -> bool {
    let ovmf_candidates = [
        "/usr/share/edk2/x64/OVMF.4m.fd",
        "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
        "/usr/share/ovmf/x64/OVMF.4m.fd",
        "/usr/share/ovmf/x64/OVMF_CODE.4m.fd",
        "/usr/share/edk2-ovmf/x64/OVMF_CODE.fd",
        "/usr/share/OVMF/OVMF_CODE.fd",
        "/usr/share/edk2/ovmf/OVMF_CODE.fd",
        "/usr/share/qemu/ovmf-x86_64-code.bin",
    ];
    ovmf_candidates.iter().any(|p| Path::new(p).exists())
}

pub fn check_missing_dependencies() -> Vec<&'static str> {
    let mut missing = Vec::new();

    if !is_command_available("qemu-system-x86_64") {
        missing.push("qemu-system-x86_64");
    }
    if !is_command_available("qemu-img") {
        missing.push("qemu-img");
    }
    if !is_command_available("xorriso") {
        missing.push("xorriso");
    }
    if !is_command_available("mkfs.ext4") {
        missing.push("mkfs.ext4");
    }
    if !is_command_available("as") && !is_command_available("gcc") && !is_command_available("clang") {
        missing.push("assembler (as/gcc/clang)");
    }
    if !is_command_available("git") {
        missing.push("git");
    }
    if !is_ovmf_available() {
        missing.push("ovmf");
    }

    missing
}

pub fn install_system_dependencies(distro: Distro) -> Result<(), String> {
    let is_root = libc_geteuid() == 0;

    match distro {
        Distro::Arch => {
            let mut cmd = Command::new(if is_root { "pacman" } else { "sudo" });
            if !is_root {
                cmd.arg("pacman");
            }
            cmd.args(["-S", "--needed", "--noconfirm",
                "qemu-system-x86", "qemu-img", "xorriso", "mtools", "e2fsprogs",
                "edk2-ovmf", "binutils", "gcc", "git"
            ]);
            let status = cmd.status().map_err(|e| format!("pacman: {e}"))?;
            if !status.success() {
                return Err("Fallo pacman al instalar paquetes".into());
            }
        }
        Distro::Debian => {
            let mut cmd_update = Command::new(if is_root { "apt-get" } else { "sudo" });
            if !is_root {
                cmd_update.arg("apt-get");
            }
            cmd_update.args(["update"]);
            let _ = cmd_update.status();

            let mut cmd = Command::new(if is_root { "apt-get" } else { "sudo" });
            if !is_root {
                cmd.arg("apt-get");
            }
            cmd.args(["install", "-y",
                "qemu-system-x86", "qemu-utils", "xorriso", "mtools", "e2fsprogs",
                "ovmf", "binutils", "gcc", "git"
            ]);
            let status = cmd.status().map_err(|e| format!("apt-get: {e}"))?;
            if !status.success() {
                return Err("Fallo apt-get al instalar paquetes".into());
            }
        }
        Distro::Fedora => {
            let mut cmd = Command::new(if is_root { "dnf" } else { "sudo" });
            if !is_root {
                cmd.arg("dnf");
            }
            cmd.args(["install", "-y",
                "qemu-system-x86", "qemu-img", "xorriso", "mtools", "e2fsprogs",
                "edk2-ovmf", "binutils", "gcc", "git"
            ]);
            let status = cmd.status().map_err(|e| format!("dnf: {e}"))?;
            if !status.success() {
                return Err("Fallo dnf al instalar paquetes".into());
            }
        }
        Distro::OpenSuse => {
            let mut cmd = Command::new(if is_root { "zypper" } else { "sudo" });
            if !is_root {
                cmd.arg("zypper");
            }
            cmd.args(["install", "-y", "-n",
                "qemu-x86", "qemu-tools", "xorriso", "mtools", "e2fsprogs",
                "qemu-ovmf-x86_64", "binutils", "gcc", "git"
            ]);
            let status = cmd.status().map_err(|e| format!("zypper: {e}"))?;
            if !status.success() {
                return Err("Fallo zypper al instalar paquetes".into());
            }
        }
        Distro::Alpine => {
            let mut cmd = Command::new(if is_root { "apk" } else { "sudo" });
            if !is_root {
                cmd.arg("apk");
            }
            cmd.args(["add",
                "qemu-system-x86_64", "qemu-img", "xorriso", "mtools", "e2fsprogs",
                "ovmf", "binutils", "gcc", "git"
            ]);
            let status = cmd.status().map_err(|e| format!("apk: {e}"))?;
            if !status.success() {
                return Err("Fallo apk al instalar paquetes".into());
            }
        }
        Distro::Unknown => {
            return Err("Distribucion de Linux no reconocida automaticamente. Instala manualmente qemu, xorriso, mtools, e2fsprogs, ovmf y binutils.".into());
        }
    }

    Ok(())
}

fn libc_geteuid() -> u32 {
    unsafe {
        extern "C" {
            fn geteuid() -> u32;
        }
        geteuid()
    }
}

pub fn ensure_rust_components() {
    if !is_command_available("rustup") {
        return;
    }

    let installed_targets = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    if !installed_targets.contains("x86_64-unknown-none") {
        println!("[xtask] Agregando target rustup x86_64-unknown-none...");
        let _ = Command::new("rustup")
            .args(["target", "add", "x86_64-unknown-none"])
            .status();
    }

    let installed_components = Command::new("rustup")
        .args(["component", "list", "--installed"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    if !installed_components.contains("rust-src") {
        println!("[xtask] Agregando componente rust-src...");
        let _ = Command::new("rustup")
            .args(["component", "add", "rust-src"])
            .status();
    }
}

pub fn ensure_dependencies() -> Result<(), String> {
    if std::env::var("LIXOS_SKIP_DEPS").is_ok() {
        return Ok(());
    }

    ensure_rust_components();

    let missing = check_missing_dependencies();
    if missing.is_empty() {
        return Ok(());
    }

    println!("[xtask] Dependencias faltantes detectadas: {}", missing.join(", "));
    let distro = detect_distro();
    println!("[xtask] Distribucion detectada: {distro:?}. Instalando paquetes requeridos...");
    install_system_dependencies(distro)?;

    let remaining = check_missing_dependencies();
    if !remaining.is_empty() {
        eprintln!("[xtask] AVISO: Aun no se encuentran: {}", remaining.join(", "));
    } else {
        println!("[xtask] Todas las dependencias se instalaron correctamente.");
    }

    Ok(())
}
