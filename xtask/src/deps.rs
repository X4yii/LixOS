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
    if combined.contains("arch") || combined.contains("manjaro") || combined.contains("endeavouros")
    {
        Distro::Arch
    } else if combined.contains("debian")
        || combined.contains("ubuntu")
        || combined.contains("pop")
        || combined.contains("mint")
    {
        Distro::Debian
    } else if combined.contains("fedora")
        || combined.contains("rhel")
        || combined.contains("centos")
        || combined.contains("rocky")
        || combined.contains("alma")
    {
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
        .is_ok_and(|o| o.status.success())
}
pub const OVMF_CODE_CANDIDATES: &[&str] = &[
    "/usr/share/edk2/x64/OVMF.4m.fd",
    "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
    "/usr/share/ovmf/x64/OVMF.4m.fd",
    "/usr/share/ovmf/x64/OVMF_CODE.4m.fd",
    "/usr/share/edk2-ovmf/x64/OVMF_CODE.fd",
    "/usr/share/OVMF/OVMF_CODE.fd",
    "/usr/share/edk2/ovmf/OVMF_CODE.fd",
    "/usr/share/qemu/ovmf-x86_64-code.bin",
];
pub fn is_ovmf_available() -> bool {
    OVMF_CODE_CANDIDATES.iter().any(|p| Path::new(p).exists())
}
/// Versión de Limine con la que se ha probado el árbol de arranque. Se usa
/// tanto para el aviso de versión como para el download de respaldo.
pub const LIMINE_VERSION: &str = "12.9.3";
/// Ficheros que `xtask iso` copia desde `/usr/share/limine`.
const LIMINE_FILES: &[&str] = &[
    "/usr/share/limine/limine-bios-cd.bin",
    "/usr/share/limine/limine-uefi-cd.bin",
    "/usr/share/limine/limine-bios.sys",
    "/usr/share/limine/BOOTX64.EFI",
];
/// `xtask iso` necesita los binarios en `/usr/share/limine` y el CLI `limine`
/// para `limine bios-install`.
pub fn is_limine_available() -> bool {
    LIMINE_FILES.iter().all(|path| Path::new(path).exists()) && is_command_available("limine")
}
fn run_optional(command: &mut Command, context: &str) {
    match command.status() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("[xtask] AVISO: {context} terminó con estado {status}"),
        Err(error) => eprintln!("[xtask] AVISO: no se pudo ejecutar {context}: {error}"),
    }
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
    if !is_command_available("as") && !is_command_available("gcc") && !is_command_available("clang")
    {
        missing.push("assembler (as/gcc/clang)");
    }
    if !is_command_available("git") {
        missing.push("git");
    }
    if !is_ovmf_available() {
        missing.push("ovmf");
    }
    if !is_limine_available() {
        missing.push("limine");
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
            cmd.args([
                "-S",
                "--needed",
                "--noconfirm",
                "qemu-system-x86",
                "qemu-img",
                "xorriso",
                "mtools",
                "e2fsprogs",
                "edk2-ovmf",
                "binutils",
                "gcc",
                "git",
                "limine",
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
            run_optional(&mut cmd_update, "apt-get update");
            let mut cmd = Command::new(if is_root { "apt-get" } else { "sudo" });
            if !is_root {
                cmd.arg("apt-get");
            }
            cmd.args([
                "install",
                "-y",
                "qemu-system-x86",
                "qemu-utils",
                "xorriso",
                "mtools",
                "e2fsprogs",
                "ovmf",
                "binutils",
                "gcc",
                "git",
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
            cmd.args([
                "install",
                "-y",
                "qemu-system-x86",
                "qemu-img",
                "xorriso",
                "mtools",
                "e2fsprogs",
                "edk2-ovmf",
                "binutils",
                "gcc",
                "git",
                "limine",
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
            cmd.args([
                "install",
                "-y",
                "-n",
                "qemu-x86",
                "qemu-tools",
                "xorriso",
                "mtools",
                "e2fsprogs",
                "qemu-ovmf-x86_64",
                "binutils",
                "gcc",
                "git",
                "limine",
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
            cmd.args([
                "add",
                "qemu-system-x86_64",
                "qemu-img",
                "xorriso",
                "mtools",
                "e2fsprogs",
                "ovmf",
                "binutils",
                "gcc",
                "git",
                "limine",
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
/// Ejecuta `program` con `sudo` si no somos raíz.
fn run_root(program: &str, args: &[&str]) -> Result<(), String> {
    let mut cmd = if libc_geteuid() == 0 {
        Command::new(program)
    } else {
        let mut cmd = Command::new("sudo");
        cmd.arg(program);
        cmd
    };
    cmd.args(args);
    let status = cmd
        .status()
        .map_err(|error| format!("{program}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} terminó con estado {status}"))
    }
}
fn find_file(root: &Path, name: &str) -> Option<std::path::PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|file| file == name) {
            return Some(path);
        }
    }
    None
}
/// Instala Limine si falta, en dos pasos:
///
/// 1. El gestor de paquetes de la distribución. En Debian/Ubuntu el paquete
///    `limine` solo existe a partir de ciertas versiones, así que este paso es
///    opcional y un fallo no aborta nada.
/// 2. Como respaldo, el tarball binario oficial de una versión fijada desde los
///    *releases* de GitHub. Esto es lo que los docs prometían que hacía `deps` y
///    lo que permite montar el job de CI del ISO sin depender del repositorio de
///    la distribución.
pub fn install_limine(distro: Distro) -> Result<(), String> {
    if is_limine_available() {
        return Ok(());
    }
    println!("[xtask] Limine no está instalado; intentando instalarlo...");
    let manager: &[&str] = match distro {
        Distro::Arch => &["pacman", "-S", "--needed", "--noconfirm", "limine"],
        Distro::Debian => &["apt-get", "install", "-y", "limine"],
        Distro::Fedora => &["dnf", "install", "-y", "limine"],
        Distro::OpenSuse => &["zypper", "install", "-n", "limine"],
        Distro::Alpine => &["apk", "add", "limine"],
        Distro::Unknown => &[],
    };
    if !manager.is_empty() {
        let (program, args) = manager.split_first().expect("lista no vacía");
        match run_root(program, args) {
            Ok(()) if is_limine_available() => return Ok(()),
            Ok(()) => {
                eprintln!("[xtask] AVISO: el gestor de paquetes no trajo los binarios esperados.");
            }
            Err(error) => {
                eprintln!("[xtask] AVISO: no se pudo instalar limine vía paquetes ({error}).");
            }
        }
    }
    if is_limine_available() {
        return Ok(());
    }
    download_limine_release()
}
/// Descarga e instala el tarball binario fijado de Limine.
///
/// El release binario no trae el CLI de Linux (solo `limine.exe` para Windows),
/// pero sí incluye `limine.c`, que es exactamente lo que upstream compila para
/// generar la herramienta: se compila en el sitio con el `cc` del sistema.
fn download_limine_release() -> Result<(), String> {
    if !is_command_available("curl") && !is_command_available("wget") {
        return Err(
            "Limine ausente y no hay curl/wget para descargarlo. Instálalo manualmente \
             (https://github.com/limine-bootloader/limine) en /usr/share/limine."
                .into(),
        );
    }
    if !is_command_available("tar") {
        return Err("Limine ausente y no hay `tar` para extraerlo.".into());
    }
    let url = format!(
        "https://github.com/Limine-Bootloader/Limine/releases/download/v{ver}/limine-binary.tar.xz",
        ver = LIMINE_VERSION,
    );
    let tmp = std::env::temp_dir().join(format!("limine-{LIMINE_VERSION}"));
    let _ = fs::remove_dir_all(&tmp);
    let extract = tmp.join("extract");
    fs::create_dir_all(&extract).map_err(|error| format!("mkdir {extract:?}: {error}"))?;
    let archive = tmp.join("limine-binary.tar.xz");
    let archive_str = archive.to_string_lossy().into_owned();
    println!("[xtask] Descargando Limine {LIMINE_VERSION}...");
    let mut fetch = if is_command_available("curl") {
        let mut cmd = Command::new("curl");
        cmd.args(["-fsSL", "-o", &archive_str, &url]);
        cmd
    } else {
        let mut cmd = Command::new("wget");
        cmd.args(["-q", "-O", &archive_str, &url]);
        cmd
    };
    let status = fetch
        .status()
        .map_err(|error| format!("descarga: {error}"))?;
    if !status.success() {
        return Err(format!("la descarga de {url} terminó con {status}"));
    }
    let extract_str = extract.to_string_lossy().into_owned();
    let status = Command::new("tar")
        .args(["-xf", &archive_str, "-C", &extract_str])
        .status()
        .map_err(|error| format!("tar: {error}"))?;
    if !status.success() {
        return Err(format!("tar terminó con {status}"));
    }
    build_limine_cli(&extract)?;
    run_root("install", &["-d", "-m", "755", "/usr/share/limine"])?;
    for file in [
        "limine-bios-cd.bin",
        "limine-uefi-cd.bin",
        "limine-bios.sys",
        "BOOTX64.EFI",
    ] {
        let source = find_file(&extract, file)
            .ok_or_else(|| format!("el tarball de Limine no contiene {file}"))?;
        let source = source.to_string_lossy().into_owned();
        let target = format!("/usr/share/limine/{file}");
        run_root("install", &["-m", "644", &source, &target])?;
    }
    let cli = find_file(&extract, "limine")
        .ok_or("el CLI `limine` no se pudo generar a partir de limine.c")?;
    let cli = cli.to_string_lossy().into_owned();
    run_root("install", &["-m", "755", &cli, "/usr/local/bin/limine"])?;
    if is_limine_available() {
        println!("[xtask] Limine {LIMINE_VERSION} instalado correctamente.");
        Ok(())
    } else {
        Err(
            "Limine se descargó pero los binarios siguen sin estar en /usr/share/limine; \
             revisa los permisos de `install`."
                .into(),
        )
    }
}
/// Compila el CLI de Limine dentro del árbol extraído (si el tarball ya lo
/// trae hecho, no hace nada).
fn build_limine_cli(extract: &Path) -> Result<(), String> {
    if find_file(extract, "limine").is_some() {
        return Ok(());
    }
    let source = find_file(extract, "limine.c").ok_or("el tarball de Limine no trae limine.c")?;
    let output = source.with_file_name("limine");
    let cc = ["cc", "gcc", "clang"]
        .into_iter()
        .find(|candidate| is_command_available(candidate));
    let Some(cc) = cc else {
        return Err(
            "hace falta un compilador C (cc/gcc/clang) para generar el CLI `limine`".into(),
        );
    };
    let source_str = source.to_string_lossy().into_owned();
    let output_str = output.to_string_lossy().into_owned();
    println!("[xtask] Compilando el CLI de Limine con {cc}...");
    let status = Command::new(cc)
        .args([
            "-std=c99",
            "-D_FILE_OFFSET_BITS=64",
            "-O2",
            "-o",
            &output_str,
            &source_str,
        ])
        .status()
        .map_err(|error| format!("{cc}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("la compilación de limine.c terminó con {status}"))
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
        run_optional(
            Command::new("rustup").args(["target", "add", "x86_64-unknown-none"]),
            "rustup target add x86_64-unknown-none",
        );
    }
    let installed_components = Command::new("rustup")
        .args(["component", "list", "--installed"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    if !installed_components.contains("rust-src") {
        println!("[xtask] Agregando componente rust-src...");
        run_optional(
            Command::new("rustup").args(["component", "add", "rust-src"]),
            "rustup component add rust-src",
        );
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
    println!(
        "[xtask] Dependencias faltantes detectadas: {}",
        missing.join(", ")
    );
    let distro = detect_distro();
    println!("[xtask] Distribucion detectada: {distro:?}. Instalando paquetes requeridos...");
    install_system_dependencies(distro)?;
    if let Err(error) = install_limine(distro) {
        eprintln!("[xtask] AVISO: no se pudo instalar Limine automáticamente: {error}");
        eprintln!(
            "[xtask]        Instálalo a mano (paquete `limine` o \
             https://github.com/limine-bootloader/limine) y sitúa los binarios en \
             /usr/share/limine; sin ellos `cargo xtask iso` falla."
        );
    }
    let remaining = check_missing_dependencies();
    if remaining.is_empty() {
        println!("[xtask] Todas las dependencias se instalaron correctamente.");
    } else {
        eprintln!(
            "[xtask] AVISO: Aun no se encuentran: {}",
            remaining.join(", ")
        );
    }
    Ok(())
}
