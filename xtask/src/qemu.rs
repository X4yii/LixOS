use crate::deps;
use crate::image;
use crate::iso;
use std::env;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{exit, Command};
fn ensure_nvme_disk(disk_path: &Path, logical_block_size: u64) -> Result<(), String> {
    match disk_path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_file() => {
            println!(
                "[xtask] Reutilizando imagen NVMe en modo read-only: {}",
                disk_path.display()
            );
            Ok(())
        }
        Ok(_) => {
            Err("El destino existe y no es un archivo regular; se rechaza por seguridad.".into())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            println!("[xtask] Creando disco NVMe de prueba con GPT...");
            image::create_nvme_test_image(disk_path, logical_block_size)
        }
        Err(error) => Err(format!("No se pudo verificar el destino: {error}")),
    }
}
fn launch_qemu() -> Result<(), String> {
    let use_4k_sectors = env::args().any(|arg| arg == "--nvme-4k");
    let disk_path = Path::new(if use_4k_sectors {
        image::NVME_IMAGE_4K
    } else {
        image::NVME_IMAGE_512
    });
    let logical_block_size = if use_4k_sectors { 4096 } else { 512 };
    ensure_nvme_disk(disk_path, logical_block_size)?;
    let ovmf_path = deps::OVMF_CODE_CANDIDATES
        .iter()
        .copied()
        .find(|path| Path::new(path).is_file());
    let is_headless = env::args().any(|arg| arg == "--headless" || arg == "-n")
        || (env::var("DISPLAY").is_err() && env::var("WAYLAND_DISPLAY").is_err());
    let mut qemu = Command::new("qemu-system-x86_64");
    let drive_arg = format!(
        "id=disk,file={},if=none,format=raw,readonly=on",
        disk_path.display()
    );
    let namespace_arg = if use_4k_sectors {
        "nvme-ns,drive=disk,nsid=1,logical_block_size=4096,physical_block_size=4096"
    } else {
        "nvme-ns,drive=disk,nsid=1"
    };
    let smp_count = env::args()
        .position(|arg| arg == "--smp")
        .and_then(|index| env::args().nth(index + 1))
        .unwrap_or_else(|| "4".to_string());
    qemu.args([
        "-M",
        "q35,accel=kvm:tcg",
        "-cpu",
        "max",
        "-smp",
        &smp_count,
        "-m",
        "2G",
        "-cdrom",
        image::ISO_OUTPUT,
        "-drive",
        &drive_arg,
        "-device",
        "nvme,id=nvme0,serial=lixos-test",
        "-device",
        namespace_arg,
        "-netdev",
        "user,id=lixnet0",
        "-device",
        "virtio-net-pci,netdev=lixnet0,disable-modern=on",
        "-device",
        "qemu-xhci,id=lixos-xhci",
        "-device",
        "usb-kbd,bus=lixos-xhci.0",
        "-serial",
        "stdio",
    ]);
    if is_headless {
        println!("[xtask] Modo Headless activado (-display none).");
        qemu.args(["-display", "none"]);
    } else {
        println!("[xtask] Modo Gráfico activado: abriendo ventana QEMU...");
    }
    if let Some(path) = ovmf_path {
        println!("[xtask] Firmware UEFI detectado: {path}");
        qemu.args(["-bios", path]);
    } else {
        println!("[xtask] AVISO: Firmware OVMF no detectado. Arrancando en modo BIOS híbrido.");
    }
    let status = qemu
        .status()
        .map_err(|error| format!("no se pudo lanzar QEMU: {error}"))?;
    exit(status.code().unwrap_or(0));
}
pub fn task_run() -> Result<(), String> {
    iso::task_iso()?;
    println!("[xtask] Verificando presencia de QEMU...");
    let qemu_check = Command::new("which").arg("qemu-system-x86_64").output();
    match qemu_check {
        Ok(output) if output.status.success() => launch_qemu(),
        _ => {
            println!("[xtask] AVISO: 'qemu-system-x86_64' no está instalado en el host.");
            println!("[xtask] Instálalo con: sudo pacman -S qemu-desktop edk2-ovmf xorriso");
            Ok(())
        }
    }
}
