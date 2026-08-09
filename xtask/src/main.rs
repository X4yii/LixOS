use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, exit};

fn main() {
    let mut args = env::args().skip(1);
    let task = args.next().unwrap_or_else(|| "help".to_string());

    match task.as_str() {
        "build" => task_build(),
        "check" => task_check(),
        "iso" => task_iso(),
        "run" => task_run(),
        _ => print_help(),
    }
}

fn print_help() {
    println!("LixOS xtask Build Runner");
    println!("Uso: cargo xtask <comando>");
    println!();
    println!("Comandos disponibles:");
    println!("  build   - Compila el kernel para bare-metal (x86_64-unknown-none)");
    println!("  check   - Verifica sintaxis y tipos del workspace completo");
    println!("  iso     - Prepara el árbol de arranque y configuración de Limine");
    println!("  run     - Compila y lanza el kernel en QEMU");
}

/// Flags del kernel: higher-half (-2 GB) con su propio linker script.
const KERNEL_RUSTFLAGS: &str = "-C code-model=kernel -C relocation-model=static -C force-frame-pointers=yes -C link-arg=-Tkernel/linker.ld -C link-arg=-nostdlib -C link-arg=-zmax-page-size=0x1000";
/// Flags de userspace: mitad baja (0x400000), sin code-model=kernel.
const USER_RUSTFLAGS: &str = "-C relocation-model=static -C force-frame-pointers=yes -C link-arg=-Tuserland/linker.ld -C link-arg=-nostdlib -C link-arg=-zmax-page-size=0x1000";

/// Inicializa los submódulos git (litui vive en su propio repo). Best-effort: si no es un repo
/// con submódulos (p.ej. árbol exportado sin .git) o git no está disponible, se ignora en silencio.
fn init_submodules() {
    if !Path::new(".git").exists() {
        return;
    }
    let _ = Command::new("git")
        .args(["submodule", "update", "--init", "--recursive"])
        .status();
}

fn strip_binary(path: &Path) {
    if !path.exists() {
        return;
    }
    // Best-effort: usa `strip` del host si existe; si no, deja el binario sin strippear.
    let _ = Command::new("strip").arg(path).status();
}

fn task_build() {
    init_submodules();

    println!("[xtask] Compilando lix-kernel (x86_64-unknown-none)...");
    let status_k = Command::new("cargo")
        .env("RUSTC_BOOTSTRAP", "1")
        .env("RUSTFLAGS", KERNEL_RUSTFLAGS)
        .args([
            "build", "--package", "kernel",
            "--target", "x86_64-unknown-none",
            "-Z", "build-std=core,alloc,compiler_builtins",
        ])
        .status()
        .expect("Error al ejecutar cargo build (kernel)");

    println!("[xtask] Compilando ltop (x86_64-unknown-none)...");
    let status_u = Command::new("cargo")
        .env("RUSTC_BOOTSTRAP", "1")
        .env("RUSTFLAGS", USER_RUSTFLAGS)
        .args([
            "build", "--release", "--package", "ltop",
            "--target", "x86_64-unknown-none",
            "-Z", "build-std=core,alloc,compiler_builtins",
        ])
        .status()
        .expect("Error al ejecutar cargo build (ltop)");

    if !status_k.success() || !status_u.success() {
        eprintln!("[xtask] Fallo al compilar.");
        exit(1);
    }
    
    // Copiamos el ejecutable a rootfs/ y lo strippeamos (quita symtab/debug → mucho más ligero
    // para la LiveISO; el binario de userspace no necesita símbolos).
    let _ = fs::copy("target/x86_64-unknown-none/release/ltop", "rootfs/ltop.elf");
    strip_binary(Path::new("rootfs/ltop.elf"));

    println!("[xtask] Kernel y LTOP compilados exitosamente.");
}

fn task_check() {
    println!("[xtask] Verificando crates del workspace...");
    let status = Command::new("cargo")
        .args(["check", "--workspace"])
        .status()
        .expect("Error al ejecutar cargo check");

    if !status.success() {
        eprintln!("[xtask] Errores detectados durante cargo check.");
        exit(1);
    }
}

fn task_iso() {
    task_build();

    let iso_dir = Path::new("target/iso_root");
    let efi_dir = iso_dir.join("EFI/BOOT");
    fs::create_dir_all(&efi_dir).expect("Error creando directorios EFI");

    let rootfs_dir = Path::new("rootfs");
    if rootfs_dir.exists() {
        println!("[xtask] Empaquetando rootfs en initramfs.tar...");
        let status = Command::new("tar")
            .args(["-cvf", "target/iso_root/initramfs.tar", "-C", "rootfs", "."])
            .status()
            .expect("Error al ejecutar tar");
        if !status.success() {
            eprintln!("[xtask] Fallo al crear initramfs.tar");
            exit(1);
        }
    }

    let limine_cfg = "\
timeout: 0

/LixOS v0.0.1
    protocol: limine
    kernel_path: boot():/kernel.elf
    module_path: boot():/initramfs.tar
";
    fs::write(iso_dir.join("limine.conf"), limine_cfg).expect("Error escribiendo limine.conf");
    fs::write(iso_dir.join("limine.cfg"), limine_cfg).expect("Error escribiendo limine.cfg");
    fs::write(efi_dir.join("limine.conf"), limine_cfg).expect("Error escribiendo EFI limine.conf");

    let limine_share = Path::new("/usr/share/limine");
    if limine_share.exists() {
        let _ = fs::copy(limine_share.join("limine-bios-cd.bin"), iso_dir.join("limine-bios-cd.bin"));
        let _ = fs::copy(limine_share.join("limine-uefi-cd.bin"), iso_dir.join("limine-uefi-cd.bin"));
        let _ = fs::copy(limine_share.join("limine-bios.sys"), iso_dir.join("limine-bios.sys"));
        let _ = fs::copy(limine_share.join("BOOTX64.EFI"), efi_dir.join("BOOTX64.EFI"));
    }

    let kernel_src = Path::new("target/x86_64-unknown-none/debug/kernel");
    if kernel_src.exists() {
        fs::copy(kernel_src, iso_dir.join("kernel.elf")).expect("Error copiando kernel.elf");
        println!("[xtask] kernel.elf copiado a target/iso_root/.");
    }

    println!("[xtask] Creando imagen ISO híbrida (target/lixos.iso)...");
    let xorriso = Command::new("xorriso")
        .args([
            "-as", "mkisofs",
            "-R", "-r", "-J",
            "-b", "limine-bios-cd.bin",
            "-no-emul-boot", "-boot-load-size", "4", "-boot-info-table",
            "-hfsplus", "-apm-block-size", "2048",
            "--efi-boot", "limine-uefi-cd.bin",
            "-efi-boot-part", "--efi-boot-image", "--protective-msdos-label",
            "target/iso_root",
            "-o", "target/lixos.iso",
        ])
        .status();

    match xorriso {
        Ok(s) if s.success() => {
            let _ = Command::new("limine")
                .args(["bios-install", "target/lixos.iso"])
                .status();
            println!("[xtask] Imagen ISO generada exitosamente en target/lixos.iso.");
        }
        _ => eprintln!("[xtask] Advertencia: xorriso falló o no está instalado."),
    }
}

fn task_run() {
    task_iso();

    println!("[xtask] Verificando presencia de QEMU...");
    let qemu_check = Command::new("which")
        .arg("qemu-system-x86_64")
        .output();

    match qemu_check {
        Ok(output) if output.status.success() => {
            println!("[xtask] Ejecutando QEMU...");

            let disk_path = "target/lixos_disk.img";
            // Regenerar siempre el disco para reflejar el rootfs actual (evita un ltop.elf obsoleto).
            let _ = fs::remove_file(disk_path);
            {
                println!("[xtask] Creando disco virtual de 10GB en {}...", disk_path);
                Command::new("qemu-img")
                    .args(["create", "-f", "raw", disk_path, "10G"])
                    .status()
                    .expect("Fallo al crear qemu-img");
                
                println!("[xtask] Formateando disco virtual en Ext4 nativo...");
                Command::new("mkfs.ext4")
                    .args(["-q", "-F", "-O", "extents,uninit_bg,dir_index,filetype,has_journal", "-d", "rootfs", disk_path])
                    .status()
                    .expect("Fallo al formatear ext4");
            }

            let ovmf_candidates = [
                "/usr/share/edk2/x64/OVMF.4m.fd",
                "/usr/share/ovmf/x64/OVMF.4m.fd",
                "/usr/share/edk2-ovmf/x64/OVMF_CODE.fd",
                "/usr/share/OVMF/OVMF_CODE.fd",
            ];
            let ovmf_path = ovmf_candidates.iter().find(|p| Path::new(p).exists());

            let mut qemu = Command::new("qemu-system-x86_64");
            qemu.args([
                "-M", "q35",
                "-m", "2G",
                "-cdrom", "target/lixos.iso",
                "-drive", "id=disk,file=target/lixos_disk.img,if=none,format=raw",
                "-device", "ahci,id=ahci",
                "-device", "ide-hd,drive=disk,bus=ahci.0",
                "-serial", "stdio",
                "-display", "none",
            ]);

            if let Some(path) = ovmf_path {
                println!("[xtask] Firmware UEFI detectado: {}", path);
                qemu.args(["-bios", path]);
            } else {
                println!("[xtask] AVISO: Firmware OVMF no detectado. Arrancando en modo BIOS híbrido.");
            }

            let status = qemu.status().expect("Error al lanzar QEMU");
            exit(status.code().unwrap_or(0));
        }
        _ => {
            println!("[xtask] AVISO: 'qemu-system-x86_64' no está instalado en el host.");
            println!("[xtask] Instálalo con: sudo pacman -S qemu-desktop edk2-ovmf xorriso");
        }
    }
}