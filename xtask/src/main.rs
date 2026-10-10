use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, exit};
use std::io::{Seek, SeekFrom, Write};
use lix_storage::{GptPartition, build_gpt_image};

mod nvme_test;
mod codegen;
mod assembler;
mod deps;

/// Builds a disposable QEMU-only GPT image. Existing images are never replaced.
fn create_nvme_test_image(path: &Path, logical_block_size: u64) -> Result<(), String> {
    if !matches!(logical_block_size, 512 | 4096) { return Err("logical block size must be 512 or 4096 bytes".into()); }
    if path.symlink_metadata().is_ok() { return Err(format!("{} ya existe; no se sobrescribió", path.display())); }
    let status = Command::new("qemu-img").args(["create", "-f", "raw", path.to_str().ok_or("ruta inválida")?, "10G"])
        .status().map_err(|e| format!("qemu-img: {e}"))?;
    if !status.success() { return Err("qemu-img no pudo crear la imagen".into()); }
    let mut file = fs::OpenOptions::new().read(true).write(true).open(path).map_err(|e| e.to_string())?;
    let image_bytes = file.metadata().map_err(|e| e.to_string())?.len();
    let total_lbas = image_bytes / logical_block_size;
    let alignment_blocks = 1024 * 1024 / logical_block_size;
    let partition_first = alignment_blocks;
    let table_blocks = (128 * 128u64).div_ceil(logical_block_size);
    let partition_last = total_lbas.checked_sub(table_blocks + 2).ok_or("imagen demasiado pequeña")?;
    let partition_lba_count = partition_last - partition_first + 1;
    let layout = build_gpt_image(logical_block_size as u32, total_lbas,
        [0x4c,0x69,0x78,0x4f,0x53,0x54,0x45,0x53,0x54,0,0,0,0,0,0,1],
        &[GptPartition {
            type_guid: [0xAF,0x3D,0xC6,0x0F,0x83,0x84,0x72,0x47,0x8E,0x79,0x3D,0x69,0xD8,0x47,0x7D,0xE4],
            unique_guid: [1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1],
            first_lba: partition_first,
            last_lba: partition_last,
            attributes: 0,
        }]).map_err(|error| format!("GPT layout: {error:?}"))?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    file.write_all(&layout.protective_mbr).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(logical_block_size)).map_err(|e| e.to_string())?;
    file.write_all(&layout.primary_header).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(2 * logical_block_size)).map_err(|e| e.to_string())?;
    file.write_all(&layout.primary_entries).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(layout.backup_entries_lba * logical_block_size)).map_err(|e| e.to_string())?;
    file.write_all(&layout.backup_entries).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start((total_lbas - 1) * logical_block_size)).map_err(|e| e.to_string())?;
    file.write_all(&layout.backup_header).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    let partition_blocks = partition_lba_count.checked_mul(logical_block_size)
        .ok_or("partición demasiado grande")? / 4096;
    format_test_ext4(path, partition_first * logical_block_size, partition_blocks)
}

fn format_test_ext4(path: &Path, offset: u64, partition_blocks: u64) -> Result<(), String> {
    let offset_arg = format!("offset={offset}");
    let partition_blocks_arg = partition_blocks.to_string();
    let status = Command::new("mkfs.ext4")
        .args(["-q", "-F", "-b", "4096", "-O", "extents,uninit_bg,dir_index,filetype,has_journal,^64bit,^flex_bg,^ea_inode,^metadata_csum,^metadata_csum_seed", "-d", "rootfs",
            "-E", &offset_arg, path.to_str().ok_or("ruta inválida")?, &partition_blocks_arg])
        .status().map_err(|e| format!("mkfs.ext4: {e}"))?;
    if !status.success() { return Err("mkfs.ext4 no pudo crear el sistema de archivos de prueba".into()); }
    Ok(())
}

fn main() {
    let mut args = env::args().skip(1);
    let task = args.next().unwrap_or_else(|| "help".to_string());

    match task.as_str() {
        "build" => task_build(),
        "check" => task_check(),
        "iso" => task_iso(),
        "run" => task_run(),
        "asm" => {
            if let Err(error) = assembler::assemble_kernel_asm() {
                eprintln!("[xtask] {error}");
                exit(1);
            }
        }
        "deps" => {
            if let Err(error) = deps::ensure_dependencies() {
                eprintln!("[xtask] {error}");
                exit(1);
            }
        }
        "test-nvme" => nvme_test::run(),
        "codegen" => codegen::run(),
        "nvme-image" => {
            let output = env::args().nth(2).unwrap_or_else(|| "target/lixos_nvme.img".to_string());
            if let Err(error) = create_nvme_test_image(Path::new(&output), 512) {
                eprintln!("[xtask] {error}");
                exit(1);
            }
        }
        "nvme-image-4k" => {
            let output = env::args().nth(2).unwrap_or_else(|| "target/lixos_nvme_4k.img".to_string());
            if let Err(error) = create_nvme_test_image(Path::new(&output), 4096) {
                eprintln!("[xtask] {error}");
                exit(1);
            }
        }
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
    println!("  codegen - Genera tablas de syscalls e includes desde abi/syscalls.toml");
    println!("  iso     - Prepara el árbol de arranque y configuración de Limine");
    println!("  run [--headless] [--nvme-4k] - Compila y lanza QEMU; --nvme-4k prueba sectores NVMe de 4 KiB");
    println!("  asm     - Ensambla archivos .S en kernel/asm/");
    println!("  deps    - Verifica e instala dependencias requeridas del sistema");
    println!("  test-nvme - Prueba arranque UEFI+NVMe de 512 B y 4 KiB en QEMU");
    println!("  nvme-image [PATH] - Crea una imagen GPT+ext4 desechable para pruebas (no sobrescribe)");
    println!("  nvme-image-4k [PATH] - Igual, con sectores lógicos NVMe de 4 KiB");
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
    let _ = Command::new("strip").arg(path).status();
}

fn task_build() {
    init_submodules();
    let _ = deps::ensure_dependencies();

    let asm_objs = match assembler::assemble_kernel_asm() {
        Ok(objs) => objs,
        Err(e) => {
            eprintln!("[xtask] Error ensamblando archivos .S: {e}");
            exit(1);
        }
    };

    let mut kernel_rustflags = KERNEL_RUSTFLAGS.to_string();
    for obj in &asm_objs {
        kernel_rustflags.push_str(&format!(" -C link-arg={}", obj.display()));
    }

    println!("[xtask] Compilando lix-kernel (x86_64-unknown-none)...");
    let status_k = Command::new("cargo")
        .env("RUSTC_BOOTSTRAP", "1")
        .env("RUSTFLAGS", kernel_rustflags)
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

    println!("[xtask] Compilando vanish (x86_64-unknown-none)...");
    let status_v = Command::new("cargo")
        .env("RUSTC_BOOTSTRAP", "1")
        .env("RUSTFLAGS", USER_RUSTFLAGS)
        .args([
            "build", "--release", "--package", "vanish",
            "--target", "x86_64-unknown-none",
            "-Z", "build-std=core,alloc,compiler_builtins",
        ])
        .status()
        .expect("Error al ejecutar cargo build (vanish)");

    if !status_k.success() || !status_u.success() || !status_v.success() {
        eprintln!("[xtask] Fallo al compilar.");
        exit(1);
    }
    
    let _ = fs::create_dir_all("rootfs/bin");
    let _ = fs::copy("target/x86_64-unknown-none/release/ltop", "rootfs/ltop.elf");
    strip_binary(Path::new("rootfs/ltop.elf"));
    let _ = fs::copy("target/x86_64-unknown-none/release/ltop", "rootfs/bin/ltop.elf");
    strip_binary(Path::new("rootfs/bin/ltop.elf"));

    let _ = fs::copy("target/x86_64-unknown-none/release/vanish", "rootfs/vanish.elf");
    strip_binary(Path::new("rootfs/vanish.elf"));
    let _ = fs::copy("target/x86_64-unknown-none/release/vanish", "rootfs/bin/vanish.elf");
    strip_binary(Path::new("rootfs/bin/vanish.elf"));

    println!("[xtask] Kernel, LTOP y Vanish Shell compilados exitosamente.");
}

fn task_check() {
    println!("[xtask] Verificando crates del workspace...");
    let status = Command::new("cargo")
        .env("RUSTC_BOOTSTRAP", "1")
        .args(["check", "--workspace"])
        .status()
        .expect("Error al ejecutar cargo check");

    if !status.success() {
        eprintln!("[xtask] Errores detectados durante cargo check.");
        exit(1);
    }
}

fn task_iso() {
    let root_guid = match env::var("LIXOS_ROOT_PARTUUID") {
        Ok(guid) => Some(guid),
        Err(env::VarError::NotPresent) => None,
        Err(_) => {
            eprintln!("[xtask] LIXOS_ROOT_PARTUUID no es UTF-8 válido.");
            exit(2);
        }
    };
    task_iso_with_root(root_guid.as_deref());
}

fn task_iso_with_root(root_partuuid: Option<&str>) {
    let root_cmdline = match root_partuuid {
        Some(guid) if lix_storage::parse_guid(guid).is_some() => {
            format!("    cmdline: root=PARTUUID={guid}\n")
        }
        Some(_) => {
            eprintln!("[xtask] LIXOS_ROOT_PARTUUID debe ser un GUID válido; se cancela la creación del ISO.");
            exit(2);
        }
        None => "    cmdline: root=none\n".to_string(),
    };
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

    let limine_cfg = format!("\
timeout: 0

/LixOS v0.0.1
    protocol: limine
    kernel_path: boot():/kernel.elf
    module_path: boot():/initramfs.tar
{root_cmdline}");
    fs::write(iso_dir.join("limine.conf"), &limine_cfg).expect("Error escribiendo limine.conf");
    fs::write(iso_dir.join("limine.cfg"), &limine_cfg).expect("Error escribiendo limine.cfg");
    fs::write(efi_dir.join("limine.conf"), &limine_cfg).expect("Error escribiendo EFI limine.conf");

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

            let use_4k_sectors = env::args().any(|arg| arg == "--nvme-4k");
            let disk_path = if use_4k_sectors { Path::new("target/lixos_nvme_4k.img") } else { Path::new("target/lixos_nvme.img") };
            let logical_block_size = if use_4k_sectors { 4096 } else { 512 };
            match disk_path.symlink_metadata() {
                Ok(metadata) if metadata.file_type().is_file() => {
                    println!("[xtask] Reutilizando imagen NVMe en modo read-only: {}", disk_path.display());
                }
                Ok(_) => {
                    eprintln!("[xtask] El destino existe y no es un archivo regular; se rechaza por seguridad.");
                    exit(1);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    println!("[xtask] Creando disco NVMe de prueba con GPT...");
                    if let Err(error) = create_nvme_test_image(disk_path, logical_block_size) {
                        eprintln!("[xtask] No se pudo preparar la imagen: {error}");
                        exit(1);
                    }
                }
                Err(error) => {
                    eprintln!("[xtask] No se pudo verificar el destino: {error}");
                    exit(1);
                }
            }

            let ovmf_candidates = [
                "/usr/share/edk2/x64/OVMF.4m.fd",
                "/usr/share/ovmf/x64/OVMF.4m.fd",
                "/usr/share/edk2-ovmf/x64/OVMF_CODE.fd",
                "/usr/share/OVMF/OVMF_CODE.fd",
            ];
            let ovmf_path = ovmf_candidates.iter().find(|p| Path::new(p).exists());

            let is_headless = env::args().any(|a| a == "--headless" || a == "-n")
                || (env::var("DISPLAY").is_err() && env::var("WAYLAND_DISPLAY").is_err());

            let mut qemu = Command::new("qemu-system-x86_64");
            let drive_arg = format!("id=disk,file={},if=none,format=raw,readonly=on", disk_path.display());
            let namespace_arg = if use_4k_sectors {
                "nvme-ns,drive=disk,nsid=1,logical_block_size=4096,physical_block_size=4096"
            } else {
                "nvme-ns,drive=disk,nsid=1"
            };
            let smp_count = env::args().position(|a| a == "--smp")
                .and_then(|idx| env::args().nth(idx + 1))
                .unwrap_or_else(|| "4".to_string());
            qemu.args([
                "-M", "q35,accel=kvm:tcg",
                "-cpu", "max",
                "-smp", &smp_count,
                "-m", "2G",
                "-cdrom", "target/lixos.iso",
                "-drive", &drive_arg,
                "-device", "nvme,id=nvme0,serial=lixos-test",
                "-device", namespace_arg,
                "-serial", "stdio",
            ]);

            if is_headless {
                println!("[xtask] Modo Headless activado (-display none).");
                qemu.args(["-display", "none"]);
            } else {
                println!("[xtask] Modo Gráfico activado: abriendo ventana QEMU...");
            }

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
