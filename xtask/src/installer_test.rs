//! Test de integración del instalador de LixOS.
//!
//! Escenario: disco objetivo de 10 GiB con una única partición ext4 **vacía**
//! (el «disco de prueba» con una partición sin nada). Se arranca el LiveISO en
//! QEMU con ese disco como NVMe, se ejecuta `lixinstall` por el serie en modo
//! «Partición Existente» y, tras la instalación, se valida el resultado en el
//! host:
//!
//! 1. `e2fsck -fn` sobre la partición raíz instalada.
//! 2. `debugfs`: estructura de directorios, tamaños de binarios (comparados
//!    contra los del medio live, para cazar copias truncadas) y contenido de
//!    `/etc` y `/home`.
//! 3. Un segundo arranque desde el propio disco instalado, sin ISO.
//!
//! El disco y sus artefactos se guardan en `target/` y se conservan si algo
//! falla (se imprime la ruta para inspección manual con `debugfs`/`e2fsck`).

use crate::image::INITRAMFS_TAR;
use crate::smoke::{self, Session};
use crate::util;
use crate::{build, iso};
use lix_storage::{
    build_gpt_image, write_gpt_layout, BlockWriter, GptPartition, EXT4_MKFS_FEATURES,
    LINUX_FILESYSTEM_TYPE_GUID,
};
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;
const DISK_IMAGE: &str = "target/lixos-install-test.img";
const ROOT_SLICE: &str = "target/lixos-install-root-slice.img";
const BOOT_MARKER: &str = "Shell Vanish iniciada en Ring 3";
/// Sector lógico del disco de prueba (NVMe de 512 B).
const SECTOR: u64 = 512;
/// GUID de partición «Linux filesystem» tal y como la imprime `sfdisk`.
const LINUX_PART_TYPE: &str = "0FC63DAF";
struct FileBlockWriter<'a>(&'a mut File);
impl BlockWriter for FileBlockWriter<'_> {
    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<(), &'static str> {
        self.0
            .seek(SeekFrom::Start(offset))
            .map_err(|_| "error de seek en la imagen")?;
        self.0
            .write_all(bytes)
            .map_err(|_| "error de escritura en la imagen")?;
        Ok(())
    }
}
/// Crea el disco objetivo de 10 GiB: GPT con **una sola partición ext4 vacía**
/// (alineada a 1 MiB). Devuelve `(inicio_lba, sectores)` de la partición.
fn create_target_image(path: &Path) -> Result<(u64, u64), String> {
    let _ = fs::remove_file(path);
    let image_path = path
        .to_str()
        .ok_or_else(|| format!("ruta inválida: {}", path.display()))?;
    util::run_checked(
        "qemu-img",
        &["create", "-f", "raw", image_path, "10G"],
        "creación del disco objetivo de 10 GiB",
    )?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("no se pudo abrir {image_path}: {error}"))?;
    let image_bytes = file
        .metadata()
        .map_err(|error| format!("no se pudo medir {image_path}: {error}"))?
        .len();
    let total_lbas = image_bytes / SECTOR;
    let partition_first = 1024 * 1024 / SECTOR;
    let table_blocks = lix_storage::GPT_ENTRY_TABLE_BYTES.div_ceil(SECTOR);
    let partition_last = total_lbas
        .checked_sub(table_blocks + 2)
        .ok_or("disco demasiado pequeño para GPT")?;
    let layout = build_gpt_image(
        SECTOR as u32,
        total_lbas,
        [
            0x4c, 0x69, 0x78, 0x49, 0x4e, 0x53, 0x54, 0x00, 0, 0, 0, 0, 0, 0, 0, 1,
        ],
        &[GptPartition {
            type_guid: LINUX_FILESYSTEM_TYPE_GUID,
            unique_guid: [3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            first_lba: partition_first,
            last_lba: partition_last,
            attributes: 0,
        }],
    )
    .map_err(|error| format!("GPT layout: {error:?}"))?;
    {
        let mut writer = FileBlockWriter(&mut file);
        write_gpt_layout(&mut writer, &layout, SECTOR, total_lbas)
            .map_err(|error| format!("GPT: {error}"))?;
    }
    file.sync_all()
        .map_err(|error| format!("sync de la imagen: {error}"))?;
    drop(file);
    let partition_sectors = partition_last - partition_first + 1;
    let partition_blocks_4k = partition_sectors * SECTOR / 4096;
    let offset_arg = format!("offset={}", partition_first * SECTOR);
    util::run_checked(
        "mkfs.ext4",
        &[
            "-q",
            "-F",
            "-b",
            "4096",
            "-O",
            EXT4_MKFS_FEATURES,
            "-E",
            &offset_arg,
            image_path,
            &partition_blocks_4k.to_string(),
        ],
        "formateo ext4 de la partición de prueba (vacía)",
    )?;
    println!(
        "[xtask] Disco objetivo: {image_path} (10 GiB, partición ext4 vacía en LBA {partition_first}, {partition_sectors} sectores)"
    );
    Ok((partition_first, partition_sectors))
}
fn ovmf_firmware() -> Option<&'static str> {
    crate::deps::OVMF_CODE_CANDIDATES
        .iter()
        .copied()
        .find(|path| Path::new(path).is_file())
}
/// QEMU con el disco objetivo como NVMe; `con_iso` añade el LiveISO como CD.
fn spawn(install_phase: bool) -> Result<Session, String> {
    let mut command = Command::new("qemu-system-x86_64");
    command
        .args([
            "-M",
            "q35,accel=tcg",
            "-cpu",
            "max",
            "-smp",
            "2",
            "-m",
            "2G",
        ])
        .args([
            "-drive",
            &format!("id=disk,file={DISK_IMAGE},if=none,format=raw"),
            "-device",
            "nvme,id=nvme0,serial=lixos-install-test",
            "-device",
            "nvme-ns,drive=disk,nsid=1",
            "-device",
            "qemu-xhci,id=lixos-xhci",
            "-device",
            "usb-kbd,bus=lixos-xhci.0",
            "-serial",
            "stdio",
            "-display",
            "none",
            "-no-reboot",
        ]);
    if install_phase {
        command.args(["-cdrom", crate::image::ISO_OUTPUT]);
        command.args(["-boot", "d"]);
    }
    if let Some(firmware) = ovmf_firmware() {
        command.args(["-bios", firmware]);
    } else {
        return Err("sin OVMF no se puede probar el arranque UEFI del instalador".into());
    }
    smoke::spawn_command(&mut command)
}
fn failure(session: &Session, message: String) -> String {
    format!(
        "{message}\n--- últimas líneas del serie ---\n{}",
        crate::smoke::tail(&session.snapshot(), 40)
    )
}
/// Apaga la VM con el builtin `shutdown` de Vanish (ACPI). Si no sale en el
/// plazo, se mata el proceso con un aviso: los datos ya están sincronizados
/// antes del banner de éxito del instalador.
fn shutdown(session: &mut Session, contexto: &str) -> Result<(), String> {
    session
        .send_line("shutdown")
        .map_err(|error| failure(session, error))?;
    session
        .wait_for("Apagar el sistema ahora", Duration::from_secs(60))
        .map_err(|error| failure(session, format!("{contexto}: {error}")))?;
    session
        .send_line("s")
        .map_err(|error| failure(session, error))?;
    if !session
        .wait_for_exit(Duration::from_secs(120))
        .map_err(|error| failure(session, error))?
    {
        println!("[xtask] AVISO ({contexto}): QEMU no se apagó por ACPI; se termina el proceso");
    }
    Ok(())
}
/// Ejecuta `lixinstall` en la shell por el serie y espera la instalación.
fn drive_install(session: &mut Session) -> Result<(), String> {
    session
        .wait_for(BOOT_MARKER, Duration::from_secs(300))
        .map_err(|error| failure(session, error))?;
    println!("[xtask] Shell en Ring 3; lanzando lixinstall...");
    session
        .send_line("lixinstall")
        .map_err(|error| failure(session, error))?;
    session
        .wait_for("Selecciona el disco de destino", Duration::from_secs(120))
        .map_err(|error| failure(session, error))?;
    session
        .send_line("")
        .map_err(|error| failure(session, error))?;
    session
        .wait_for("Modo de particionado", Duration::from_secs(120))
        .map_err(|error| failure(session, error))?;
    session
        .send_line("2")
        .map_err(|error| failure(session, error))?;
    session
        .wait_for("Nombre del equipo", Duration::from_secs(60))
        .map_err(|error| failure(session, error))?;
    session
        .send_line("")
        .map_err(|error| failure(session, error))?;
    session
        .wait_for("proceder con la", Duration::from_secs(60))
        .map_err(|error| failure(session, error))?;
    session
        .send_line("")
        .map_err(|error| failure(session, error))?;
    println!(
        "[xtask] Instalación en curso (copias de ~14 MB por syscall de 4 KiB; TCG puede tardar)..."
    );
    session
        .wait_for("COMPLETADA EXITOSAMENTE", Duration::from_secs(1800))
        .map_err(|error| failure(session, error))?;
    println!("[xtask] Instalador terminó con éxito dentro de la VM");
    session
        .wait_for(
            "Presiona cualquier tecla para salir",
            Duration::from_secs(60),
        )
        .map_err(|error| failure(session, error))?;
    session
        .send_line("x")
        .map_err(|error| failure(session, error))?;
    session
        .wait_for_prompt(Duration::from_secs(60))
        .map_err(|error| failure(session, error))?;
    shutdown(session, "tras la instalación")
}
/// Extrae de `sfdisk --dump` el (inicio, tamaño) en sectores de la última
/// partición Linux (la raíz que escribe el instalador).
fn linux_partition(image: &Path) -> Result<(u64, u64), String> {
    let output = Command::new("sfdisk")
        .arg("--dump")
        .arg(image)
        .output()
        .map_err(|error| format!("sfdisk: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut result = None;
    for line in stdout.lines() {
        if !line.contains(LINUX_PART_TYPE) {
            continue;
        }
        let start = field_number(line, "start=")?;
        let size = field_number(line, "size=")?;
        result = Some((start, size));
    }
    result.ok_or_else(|| {
        format!("no se encontró la partición raíz Linux en el GPT instalado:\n{stdout}")
    })
}
fn field_number(line: &str, key: &str) -> Result<u64, String> {
    let rest = line
        .split_once(key)
        .ok_or_else(|| format!("falta «{key}» en: {line}"))?
        .1;
    let value = rest
        .split(|c: char| c == ',' || c.is_whitespace())
        .find(|token| !token.is_empty())
        .ok_or_else(|| format!("valor ilegible de «{key}» en: {line}"))?;
    value
        .parse::<u64>()
        .map_err(|error| format!("«{value}» no es un número ({error})"))
}
fn debugfs_query(image: &str, command: &str) -> Result<String, String> {
    let output = Command::new("debugfs")
        .args(["-R", command, image])
        .output()
        .map_err(|error| format!("debugfs: {error}"))?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
/// Tamaño de un fichero instalado, leído con `debugfs stat`.
fn installed_size(image: &str, path: &str) -> Result<Option<u64>, String> {
    let text = debugfs_query(image, &format!("stat {path}"))?;
    if text.contains("File not found") {
        return Ok(None);
    }
    for line in text.lines() {
        let mut tokens = line.split_whitespace();
        while let Some(token) = tokens.next() {
            if token == "Size:" {
                let value = tokens
                    .next()
                    .ok_or_else(|| format!("stat ilegible de {path}"))?;
                return value
                    .parse::<u64>()
                    .map(Some)
                    .map_err(|error| format!("tamaño ilegible de {path}: {error}"));
            }
        }
    }
    Ok(None)
}
/// Tamaño de un miembro del tar del medio live (la fuente que lee el instalador).
fn live_member_size(member: &str) -> Result<u64, String> {
    let output = Command::new("tar")
        .args(["-tvf", INITRAMFS_TAR])
        .output()
        .map_err(|error| format!("tar: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let wanted = format!("./{member}");
    for line in stdout.lines() {
        let mut tokens = line.split_whitespace();
        let _permisos = tokens.next();
        let _propietario = tokens.next();
        let Some(size) = tokens.next() else {
            continue;
        };
        let nombre = tokens.collect::<Vec<_>>().join(" ");
        if nombre == wanted {
            return size
                .parse::<u64>()
                .map_err(|error| format!("tamaño ilegible de {member}: {error}"));
        }
    }
    Err(format!("{member} no está en {INITRAMFS_TAR}"))
}
/// Valida en el host la partición raíz instalada (e2fsck + debugfs).
fn verify_image(image: &Path) -> Result<(), String> {
    let (start, size) = linux_partition(image)?;
    println!("[xtask] Partición raíz instalada: LBA {start}, {size} sectores");
    util::run_checked(
        "dd",
        &[
            &format!("if={}", image.display()),
            &format!("of={ROOT_SLICE}"),
            "bs=512",
            &format!("skip={start}"),
            &format!("count={size}"),
            "conv=sparse",
            "status=none",
        ],
        "extracción de la partición raíz instalada",
    )?;
    let fsck = Command::new("e2fsck")
        .args(["-fn", ROOT_SLICE])
        .output()
        .map_err(|error| format!("e2fsck: {error}"))?;
    let fsck_text = format!(
        "{}{}",
        String::from_utf8_lossy(&fsck.stdout),
        String::from_utf8_lossy(&fsck.stderr)
    );
    if !fsck.status.success() {
        return Err(format!(
            "e2fsck -fn encontró errores en la partición instalada (estado {}):\n{fsck_text}",
            fsck.status
        ));
    }
    println!("[xtask] e2fsck -fn: sin errores");
    let root_ls = debugfs_query(ROOT_SLICE, "ls -l /")?;
    for entry in ["boot", "etc", "bin", "home"] {
        if !root_ls.contains(entry) {
            return Err(format!(
                "la raíz instalada no contiene /{entry}:\n{root_ls}"
            ));
        }
    }
    for (ruta, miembro) in [
        ("/boot/kernel.elf", "boot/kernel.elf"),
        ("/bin/system/ltop.elf", "bin/system/ltop.elf"),
        ("/bin/apps/vanish.elf", "bin/apps/vanish.elf"),
        ("/bin/apps/edit.elf", "bin/apps/edit.elf"),
        ("/bin/apps/hello.elf", "bin/apps/hello.elf"),
    ] {
        let esperado = live_member_size(miembro)?;
        match installed_size(ROOT_SLICE, ruta)? {
            None => return Err(format!("{ruta} no existe en la partición instalada")),
            Some(real) if real != esperado => {
                return Err(format!(
                    "{ruta} pesa {real} bytes en el disco instalado pero {esperado} en el medio live: copia truncada"
                ))
            }
            Some(_) => println!("[xtask] {ruta}: {esperado} bytes OK"),
        }
    }
    let notas = debugfs_query(ROOT_SLICE, "cat /home/user/notas.txt")?;
    if !notas.contains("Bienvenido a LixOS") {
        return Err(format!("/home/user/notas.txt instalado ilegible:\n{notas}"));
    }
    let keymap = debugfs_query(ROOT_SLICE, "cat /etc/keyboard.conf")?;
    if !keymap.contains("KEYMAP=latam") {
        return Err(format!("/etc/keyboard.conf instalado ilegible:\n{keymap}"));
    }
    let kconf = debugfs_query(ROOT_SLICE, "cat /etc/kernel.conf")?;
    if !kconf.contains("HOSTNAME=lixos") {
        return Err(format!("/etc/kernel.conf instalado ilegible:\n{kconf}"));
    }
    println!("[xtask] /etc y /home instalados con el contenido esperado");
    Ok(())
}
/// Segundo arranque desde el propio disco instalado, sin ISO.
fn boot_installed() -> Result<(), String> {
    println!("[xtask] Arrancando desde el disco instalado (sin ISO)...");
    let mut session = spawn(false)?;
    session
        .wait_for("VFS root redirigido a Ext4", Duration::from_secs(300))
        .map_err(|error| failure(&session, error))?;
    session
        .wait_for(BOOT_MARKER, Duration::from_secs(300))
        .map_err(|error| failure(&session, error))?;
    shutdown(&mut session, "arranque instalado")?;
    session.stop();
    println!("[xtask] El disco instalado arranca y monta su propia raíz ext4");
    Ok(())
}
pub(super) fn run() -> Result<(), String> {
    let skip_build = std::env::args().any(|arg| arg == "--no-build");
    if !skip_build {
        println!("[xtask] test-installer: construyendo kernel, userland e ISO...");
        build::task_build()?;
    }
    iso::task_iso()?;
    let image = Path::new(DISK_IMAGE);
    create_target_image(image)?;
    let install_result = (|| {
        let mut session = spawn(true)?;
        drive_install(&mut session)?;
        session.stop();
        verify_image(image)?;
        boot_installed()
    })();
    if let Err(error) = install_result {
        return Err(format!(
            "{error}\n(el disco se conserva en {DISK_IMAGE} para inspección con e2fsck/debugfs)"
        ));
    }
    let _ = fs::remove_file(ROOT_SLICE);
    let _ = fs::remove_file(image);
    println!("[xtask] test-installer: OK (disco de 10 GiB instalado, validado y arrancable)");
    Ok(())
}
