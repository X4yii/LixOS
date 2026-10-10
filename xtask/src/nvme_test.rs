use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const TEST_ROOT_PARTUUID: &str = "00000001-0000-0000-0000-000000000001";
const MISSING_ROOT_PARTUUID: &str = "DEADBEEF-0000-4000-8000-000000000000";

#[derive(Clone, Copy, Debug)]
enum BootExpectation {
    MountedRoot,
    RejectedRoot,
    InitrdOnly,
}

fn ovmf_path() -> Result<&'static str, String> {
    [
        "/usr/share/edk2/x64/OVMF.4m.fd",
        "/usr/share/ovmf/x64/OVMF.4m.fd",
        "/usr/share/edk2-ovmf/x64/OVMF_CODE.fd",
        "/usr/share/OVMF/OVMF_CODE.fd",
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file())
    .ok_or_else(|| "no se encontró firmware OVMF; test-nvme exige arranque UEFI".into())
}

fn ovmf_pflash() -> Result<(&'static str, &'static str), String> {
    [
        (
            "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
            "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
        ),
        (
            "/usr/share/ovmf/x64/OVMF_CODE.4m.fd",
            "/usr/share/ovmf/x64/OVMF_VARS.4m.fd",
        ),
        (
            "/usr/share/edk2-ovmf/x64/OVMF_CODE.fd",
            "/usr/share/edk2-ovmf/x64/OVMF_VARS.fd",
        ),
        (
            "/usr/share/OVMF/OVMF_CODE.fd",
            "/usr/share/OVMF/OVMF_VARS.fd",
        ),
    ]
    .into_iter()
    .find(|(code, vars)| Path::new(code).is_file() && Path::new(vars).is_file())
    .ok_or_else(|| "test install image requiere OVMF code y vars".into())
}

fn require_absent(path: &Path) -> Result<(), String> {
    match path.symlink_metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(format!(
            "el artefacto ya existe; no se sobrescribe: {}",
            path.display()
        )),
        Err(error) => Err(format!("no se pudo validar {}: {error}", path.display())),
    }
}

fn run_case(block_size: u64, firmware: &str, expectation: BootExpectation) -> Result<(), String> {
    let suffix = format!("{}-{block_size}-{expectation:?}", std::process::id());
    let image = Path::new("target").join(format!(".lixos-nvme-test-{suffix}.img"));
    let serial = Path::new("target").join(format!(".lixos-nvme-test-{suffix}.serial"));
    if image.symlink_metadata().is_ok() || serial.symlink_metadata().is_ok() {
        return Err(format!(
            "el artefacto de prueba ya existe; no se sobrescribe: {}",
            image.display()
        ));
    }

    let result = run_case_inner(block_size, firmware, &image, &serial, expectation);
    let _ = fs::remove_file(&image);
    let _ = fs::remove_file(&serial);
    result
}

fn run_case_inner(
    block_size: u64,
    firmware: &str,
    image: &Path,
    serial: &Path,
    expectation: BootExpectation,
) -> Result<(), String> {
    super::create_nvme_test_image(image, block_size)?;
    let drive = format!(
        "id=disk,file={},if=none,format=raw,readonly=on",
        image.display()
    );
    let serial_arg = format!("file:{}", serial.display());
    let namespace = if block_size == 4096 {
        "nvme-ns,drive=disk,nsid=1,logical_block_size=4096,physical_block_size=4096"
    } else {
        "nvme-ns,drive=disk,nsid=1"
    };
    let mut child = Command::new("qemu-system-x86_64")
        .args([
            "-M",
            "q35,accel=tcg",
            "-cpu",
            "max",
            "-m",
            "2G",
            "-cdrom",
            "target/lixos.iso",
            "-drive",
            &drive,
            "-device",
            "nvme,id=nvme0,serial=lixos-ci",
            "-device",
            namespace,
            "-serial",
            &serial_arg,
            "-display",
            "none",
            "-bios",
            firmware,
            "-no-reboot",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("no se pudo iniciar QEMU: {error}"))?;

    let deadline = Instant::now() + Duration::from_secs(45);
    let outcome = loop {
        let output = fs::read_to_string(serial).unwrap_or_default();
        let namespace_ready = output.contains("[nvme] Namespace");
        let root_mounted = output.contains("[ext4] VFS root redirigido a Ext4.");
        let root_rejected = output.contains("[nvme] GPT sin partición raíz Linux válida");
        let satisfied = match expectation {
            BootExpectation::MountedRoot => namespace_ready && root_mounted,
            BootExpectation::RejectedRoot => namespace_ready && root_rejected && !root_mounted,
            BootExpectation::InitrdOnly => {
                namespace_ready
                    && output.contains("root=none: se conserva initrd como raíz Live")
                    && output.contains("Shell Vanish iniciada")
                    && !root_mounted
            }
        };
        if satisfied {
            break Ok(());
        }
        match child
            .try_wait()
            .map_err(|error| format!("QEMU wait: {error}"))?
        {
            Some(status) => break Err(format!("QEMU terminó ({status}) antes de montar ext4")),
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
            None => break Err("timeout esperando resultado de selección root=PARTUUID".into()),
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    outcome
}

fn run_installed_image_test(block_size: u32) -> Result<(), String> {
    let suffix = format!("{}-{block_size}", std::process::id());
    let image = Path::new("target").join(format!(".lixos-installed-test-{suffix}.img"));
    let serial = Path::new("target").join(format!(".lixos-installed-test-{suffix}.serial"));
    let vars = Path::new("target").join(format!(".lixos-installed-test-{suffix}.vars.fd"));
    for path in [&image, &serial, &vars] {
        require_absent(path)?;
    }

    let result = (|| {
        let build = Command::new("cargo")
            .args(["build", "-p", "lix-installer"])
            .status()
            .map_err(|error| format!("no se pudo compilar lix-installer: {error}"))?;
        if !build.success() {
            return Err("falló la compilación de lix-installer".into());
        }
        let block_size_arg = block_size.to_string();
        let create = Command::new("target/debug/lix-installer")
            .args([
                "image",
                image.to_str().ok_or("ruta temporal inválida")?,
                "target/x86_64-unknown-none/debug/kernel",
                "target/iso_root/initramfs.tar",
                "/usr/share/limine/BOOTX64.EFI",
                "rootfs",
                "4",
                &block_size_arg,
            ])
            .status()
            .map_err(|error| format!("no se pudo generar la imagen de instalación: {error}"))?;
        if !create.success() {
            return Err("lix-installer no pudo generar la imagen de prueba".into());
        }

        let (firmware_code, vars_template) = ovmf_pflash()?;
        fs::copy(vars_template, &vars).map_err(|error| error.to_string())?;
        let drive = format!(
            "id=disk,file={},if=none,format=raw,readonly=on",
            image.display()
        );
        let firmware_code_arg = format!("if=pflash,format=raw,readonly=on,file={firmware_code}");
        let firmware_vars = format!("if=pflash,format=raw,file={}", vars.display());
        let serial_arg = format!("file:{}", serial.display());
        let namespace = if block_size == 4096 {
            "nvme-ns,drive=disk,nsid=1,logical_block_size=4096,physical_block_size=4096"
        } else {
            "nvme-ns,drive=disk,nsid=1"
        };
        let mut child = Command::new("qemu-system-x86_64")
            .args([
                "-M",
                "q35,accel=tcg",
                "-cpu",
                "max",
                "-m",
                "2G",
                "-drive",
                &firmware_code_arg,
                "-drive",
                &firmware_vars,
                "-drive",
                &drive,
                "-device",
                "nvme,id=nvme0,serial=lixos-install-test",
                "-device",
                namespace,
                "-serial",
                &serial_arg,
                "-display",
                "none",
                "-no-reboot",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("no se pudo iniciar QEMU: {error}"))?;
        let deadline = Instant::now() + Duration::from_secs(60);
        let outcome = loop {
            let output = fs::read_to_string(&serial).unwrap_or_default();
            if output.contains("[nvme] Namespace")
                && output.contains("[ext4] VFS root redirigido a Ext4.")
            {
                break Ok(());
            }
            match child
                .try_wait()
                .map_err(|error| format!("QEMU wait: {error}"))?
            {
                Some(status) => break Err(format!("QEMU terminó ({status}) antes de montar ext4")),
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
                None => break Err("timeout esperando arranque desde imagen instalada".into()),
            }
        };
        let _ = child.kill();
        let _ = child.wait();
        outcome
    })();

    for path in [&image, &serial, &vars] {
        let _ = fs::remove_file(path);
    }
    result
}

pub(super) fn run() {
    super::task_iso_with_root(Some(TEST_ROOT_PARTUUID));
    let firmware = match ovmf_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("[xtask] {error}");
            std::process::exit(1);
        }
    };
    for block_size in [512, 4096] {
        println!("[xtask] UEFI NVMe integration test: logical block size {block_size} B");
        if let Err(error) = run_case(block_size, firmware, BootExpectation::MountedRoot) {
            eprintln!("[xtask] {error}");
            std::process::exit(1);
        }
        println!("[xtask] NVMe {block_size} B: root=PARTUUID, GPT y ext4 validados");
    }
    for block_size in [512, 4096] {
        println!("[xtask] Probando imagen instalada UEFI con LBA {block_size} B...");
        if let Err(error) = run_installed_image_test(block_size) {
            eprintln!("[xtask] {error}");
            std::process::exit(1);
        }
        println!(
            "[xtask] Imagen instalada LBA {block_size} B: fallback UEFI, PARTUUID y ext4 validados"
        );
    }
    println!("[xtask] Verificando rechazo de una root=PARTUUID inexistente...");
    super::task_iso_with_root(Some(MISSING_ROOT_PARTUUID));
    if let Err(error) = run_case(512, firmware, BootExpectation::RejectedRoot) {
        eprintln!("[xtask] {error}");
        std::process::exit(1);
    }
    super::task_iso_with_root(Some(TEST_ROOT_PARTUUID));
    println!("[xtask] PARTUUID inexistente rechazada sin montar raíz incorrecta");
    println!("[xtask] Verificando arranque Live con root=none...");
    super::task_iso_with_root(None);
    if let Err(error) = run_case(512, firmware, BootExpectation::InitrdOnly) {
        eprintln!("[xtask] {error}");
        std::process::exit(1);
    }
    println!("[xtask] raíz Live initrd confirmada sin montar el NVMe objetivo");
}
