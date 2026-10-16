use crate::build;
use crate::image;
use crate::util;
use std::env;
use std::path::Path;
pub fn task_iso() -> Result<(), String> {
    let root_guid = match env::var("LIXOS_ROOT_PARTUUID") {
        Ok(guid) => Some(guid),
        Err(env::VarError::NotPresent) => None,
        Err(_) => return Err("LIXOS_ROOT_PARTUUID no es UTF-8 válido".into()),
    };
    task_iso_with_root(root_guid.as_deref())
}
pub fn task_iso_with_root(root_partuuid: Option<&str>) -> Result<(), String> {
    let root_cmdline = match root_partuuid {
        Some(guid) if lix_storage::parse_guid(guid).is_some() => {
            format!("    cmdline: root=PARTUUID={guid}\n")
        }
        Some(_) => {
            return Err(
                "LIXOS_ROOT_PARTUUID debe ser un GUID válido; se cancela la creación del ISO"
                    .into(),
            )
        }
        None => "    cmdline: root=none\n".to_string(),
    };
    build::task_build()?;
    prepare_boot_tree(&root_cmdline)?;
    create_iso_image()
}
fn prepare_boot_tree(root_cmdline: &str) -> Result<(), String> {
    let iso_dir = Path::new(image::ISO_ROOT_DIR);
    let efi_dir = iso_dir.join("EFI/BOOT");
    util::ensure_dir(&efi_dir)?;
    let rootfs_dir = Path::new(image::ROOTFS_DIR);
    if rootfs_dir.exists() {
        println!("[xtask] Empaquetando rootfs en initramfs.tar...");
        util::run_checked(
            "tar",
            &["-cvf", image::INITRAMFS_TAR, "-C", image::ROOTFS_DIR, "."],
            "creación de initramfs.tar",
        )?;
    }
    let limine_cfg = format!(
        "\
timeout: 0

/LixOS v0.0.2
    protocol: limine
    kernel_path: boot():/kernel.elf
    module_path: boot():/initramfs.tar
{root_cmdline}"
    );
    util::write_required(&iso_dir.join("limine.conf"), &limine_cfg)?;
    util::write_required(&iso_dir.join("limine.cfg"), &limine_cfg)?;
    util::write_required(&efi_dir.join("limine.conf"), &limine_cfg)?;
    let limine_share = Path::new("/usr/share/limine");
    if !limine_share.exists() {
        return Err(
            "Limine no está instalado (/usr/share/limine); no se puede preparar el árbol de arranque"
                .into(),
        );
    }
    for file in [
        "limine-bios-cd.bin",
        "limine-uefi-cd.bin",
        "limine-bios.sys",
    ] {
        util::copy_required(&limine_share.join(file), &iso_dir.join(file))?;
    }
    util::copy_required(
        &limine_share.join("BOOTX64.EFI"),
        &efi_dir.join("BOOTX64.EFI"),
    )?;
    let kernel_src = Path::new(image::KERNEL_DEBUG_ELF);
    if !kernel_src.exists() {
        return Err(format!(
            "no se encontró {}; ejecutá primero cargo xtask build",
            kernel_src.display()
        ));
    }
    util::copy_required(kernel_src, &iso_dir.join("kernel.elf"))?;
    println!("[xtask] kernel.elf copiado a target/iso_root/.");
    Ok(())
}
fn create_iso_image() -> Result<(), String> {
    println!(
        "[xtask] Creando imagen ISO híbrida ({})...",
        image::ISO_OUTPUT
    );
    util::run_checked(
        "xorriso",
        &[
            "-as",
            "mkisofs",
            "-R",
            "-r",
            "-J",
            "-b",
            "limine-bios-cd.bin",
            "-no-emul-boot",
            "-boot-load-size",
            "4",
            "-boot-info-table",
            "-hfsplus",
            "-apm-block-size",
            "2048",
            "--efi-boot",
            "limine-uefi-cd.bin",
            "-efi-boot-part",
            "--efi-boot-image",
            "--protective-msdos-label",
            image::ISO_ROOT_DIR,
            "-o",
            image::ISO_OUTPUT,
        ],
        "creación del ISO con xorriso",
    )?;
    util::run_checked(
        "limine",
        &["bios-install", image::ISO_OUTPUT],
        "instalación de arranque BIOS con Limine",
    )?;
    println!(
        "[xtask] Imagen ISO generada exitosamente en {}.",
        image::ISO_OUTPUT
    );
    Ok(())
}
