use crate::assembler;
use crate::deps;
use crate::image;
use crate::util;
use std::path::Path;
use std::process::Command;
const KERNEL_RUSTFLAGS: &str = "-C code-model=kernel -C relocation-model=static -C force-frame-pointers=yes -C link-arg=-Tkernel/linker.ld -C link-arg=-nostdlib -C link-arg=-zmax-page-size=0x1000";
const USER_RUSTFLAGS: &str = "-C relocation-model=static -C force-frame-pointers=yes -C link-arg=-Tuserland/linker.ld -C link-arg=-nostdlib -C link-arg=-zmax-page-size=0x1000";
fn init_submodules() {
    if !Path::new(".git").exists() {
        return;
    }
    match Command::new("git")
        .args(["submodule", "update", "--init", "--recursive"])
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("[xtask] AVISO: git submodule update terminó con estado {status}"),
        Err(error) => eprintln!("[xtask] AVISO: no se pudo ejecutar git submodule update: {error}"),
    }
}
fn strip_binary(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let binary = path.to_str().ok_or("ruta inválida")?;
    util::run_checked("strip", &[binary], "strip")
}
fn stage_rootfs() -> Result<(), String> {
    for stale in [
        "rootfs/ltop.elf",
        "rootfs/vanish.elf",
        "rootfs/edit.elf",
        "rootfs/bin/ltop.elf",
        "rootfs/bin/vanish.elf",
        "rootfs/bin/edit.elf",
        "rootfs/bin/dummy.txt",
    ] {
        util::remove_if_present(Path::new(stale))?;
    }
    for dir in [
        "rootfs/boot",
        "rootfs/bin/system",
        "rootfs/bin/apps",
        "rootfs/etc",
        "rootfs/home/user",
    ] {
        util::ensure_dir(Path::new(dir))?;
    }
    util::copy_required(
        Path::new(image::KERNEL_DEBUG_ELF),
        Path::new("rootfs/boot/kernel.elf"),
    )?;
    let limine_share = Path::new("/usr/share/limine");
    if limine_share.exists() {
        util::copy_required(
            &limine_share.join("BOOTX64.EFI"),
            Path::new("rootfs/boot/BOOTX64.EFI"),
        )?;
    } else {
        eprintln!("[xtask] AVISO: Limine no está instalado; rootfs/boot/BOOTX64.EFI no se generó.");
    }
    let user_binaries = [
        (
            "target/x86_64-unknown-none/release/ltop",
            "rootfs/bin/system/ltop.elf",
        ),
        (
            "target/x86_64-unknown-none/release/lixinstall",
            "rootfs/bin/system/lixinstall.elf",
        ),
        (
            "target/x86_64-unknown-none/release/vanish",
            "rootfs/bin/apps/vanish.elf",
        ),
        (
            "target/x86_64-unknown-none/release/edit",
            "rootfs/bin/apps/edit.elf",
        ),
    ];
    for (source, destination) in user_binaries {
        util::copy_required(Path::new(source), Path::new(destination))?;
        strip_binary(Path::new(destination))?;
    }
    let legacy_hello = Path::new("rootfs/bin/hello.elf");
    if legacy_hello.exists() {
        util::copy_required(legacy_hello, Path::new("rootfs/bin/apps/hello.elf"))?;
        util::remove_if_present(legacy_hello)?;
    }
    util::write_if_missing(
        Path::new("rootfs/etc/kernel.conf"),
        "HOSTNAME=lixos\nSCHEDULER=smp\nLOGLEVEL=info\nRING=0\n",
    )?;
    util::write_if_missing(
        Path::new("rootfs/home/user/notas.txt"),
        "Bienvenido a LixOS.\nEste es tu directorio personal /home/user.\n",
    )?;
    println!(
        "[xtask] Estructura /, /boot, /etc, /bin/system, /bin/apps y /home/user generada exitosamente."
    );
    Ok(())
}
pub fn task_build() -> Result<(), String> {
    init_submodules();
    if let Err(error) = deps::ensure_dependencies() {
        eprintln!("[xtask] AVISO: no se pudieron verificar las dependencias del sistema: {error}");
    }
    let asm_objs = assembler::assemble_kernel_asm()?;
    let mut kernel_rustflags = KERNEL_RUSTFLAGS.to_string();
    for obj in &asm_objs {
        kernel_rustflags.push_str(&format!(" -C link-arg={}", obj.display()));
    }
    let targets = [
        ("kernel", "lix-kernel", false, KERNEL_RUSTFLAGS),
        ("ltop", "ltop", true, USER_RUSTFLAGS),
        ("vanish", "vanish", true, USER_RUSTFLAGS),
        ("edit", "edit", true, USER_RUSTFLAGS),
        ("lixinstall", "lixinstall", true, USER_RUSTFLAGS),
    ];
    for (package, label, release, rustflags) in targets {
        let rustflags = if package == "kernel" {
            kernel_rustflags.as_str()
        } else {
            rustflags
        };
        println!("[xtask] Compilando {label} (x86_64-unknown-none)...");
        let mut command = Command::new("cargo");
        command
            .env("RUSTC_BOOTSTRAP", "1")
            .env("RUSTFLAGS", rustflags)
            .args([
                "build",
                "--package",
                package,
                "--target",
                "x86_64-unknown-none",
                "-Z",
                "build-std=core,alloc,compiler_builtins",
            ]);
        if release {
            command.arg("--release");
        }
        let status = command
            .status()
            .map_err(|error| format!("no se pudo ejecutar cargo build ({label}): {error}"))?;
        if !status.success() {
            return Err(format!("fallo al compilar {label}"));
        }
    }
    stage_rootfs()
}
pub fn task_check() -> Result<(), String> {
    println!("[xtask] Verificando crates del workspace...");
    let status = Command::new("cargo")
        .env("RUSTC_BOOTSTRAP", "1")
        .args(["check", "--workspace"])
        .status()
        .map_err(|error| format!("no se pudo ejecutar cargo check: {error}"))?;
    if !status.success() {
        return Err("errores detectados durante cargo check".into());
    }
    Ok(())
}
