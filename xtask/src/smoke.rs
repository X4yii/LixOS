//! Test de humo: arranca el ISO en QEMU, interactúa con la shell de usuariospace
//! por el puerto serie y comprueba que nada se rompe.
//!
//! Es la prueba de integración mínima que corre en CI: valida que el kernel
//! arranca, que el SMP no se cuelga, que la shell en Ring 3 puede hacer syscalls
//! y que no aparece ninguna excepción fatal.

use crate::{build, iso};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
/// Marcador de que el kernel arrancó y la shell de usuariospace está viva.
const BOOT_MARKER: &str = "Shell Vanish iniciada en Ring 3";
/// Cadenas que aparecen solo cuando algo se ha roto de verdad.
const FATAL_MARKERS: &[&str] = &[
    "EXCEPCIÓN KERNEL",
    "DOUBLE FAULT",
    "PANIC",
    "Kernel panic",
    "EXCEPCIÓN CATASTRÓFICA",
];
fn ovmf_firmware() -> Option<&'static str> {
    crate::deps::OVMF_CODE_CANDIDATES
        .iter()
        .copied()
        .find(|path| Path::new(path).is_file())
}
pub(crate) struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    output: Arc<Mutex<String>>,
}
impl Session {
    pub(crate) fn snapshot(&self) -> String {
        self.output
            .lock()
            .map(|text| text.clone())
            .unwrap_or_default()
    }
    pub(crate) fn fatal_in_output(&self) -> Option<String> {
        let text = self.snapshot();
        FATAL_MARKERS
            .iter()
            .find(|marker| text.contains(**marker))
            .map(|marker| (*marker).to_string())
    }
    pub(crate) fn wait_for(&mut self, needle: &str, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.snapshot().contains(needle) {
                return Ok(());
            }
            if let Some(fatal) = self.fatal_in_output() {
                return Err(format!("apareció «{fatal}» antes de encontrar «{needle}»"));
            }
            match self
                .child
                .try_wait()
                .map_err(|error| format!("QEMU wait: {error}"))?
            {
                Some(status) => {
                    return Err(format!("QEMU terminó ({status}) sin llegar a «{needle}»"))
                }
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(200)),
                None => return Err(format!("timeout esperando «{needle}»")),
            }
        }
    }
    pub(crate) fn send_line(&mut self, line: &str) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("stdin de QEMU cerrado")?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("no se pudo enviar «{line}»: {error}"))
    }
    pub(crate) fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stdin = None;
    }
    /// Espera a que la shell vuelva al prompt (la línea acaba en «$ »).
    pub(crate) fn wait_for_prompt(&mut self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let snapshot = self.snapshot();
            if snapshot.len() > 2 && snapshot.ends_with("$ ") {
                return Ok(());
            }
            if let Some(fatal) = self.fatal_in_output() {
                return Err(format!("«{fatal}» en serie esperando el prompt"));
            }
            match self
                .child
                .try_wait()
                .map_err(|error| format!("QEMU wait: {error}"))?
            {
                Some(status) => return Err(format!("QEMU terminó ({status}) esperando el prompt")),
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(200)),
                None => return Err("timeout esperando el prompt de la shell".into()),
            }
        }
    }
    /// Espera a que QEMU se apague solo (ACPI). Devuelve `true` si salió.
    pub(crate) fn wait_for_exit(&mut self, timeout: Duration) -> Result<bool, String> {
        let deadline = Instant::now() + timeout;
        loop {
            match self
                .child
                .try_wait()
                .map_err(|error| format!("QEMU wait: {error}"))?
            {
                Some(_) => return Ok(true),
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(200)),
                None => return Ok(false),
            }
        }
    }
}
/// Arranca un QEMU ya configurado (STDIO serie embebido, descriptores capturados).
pub(crate) fn spawn_command(command: &mut Command) -> Result<Session, String> {
    let output = Arc::new(Mutex::new(String::new()));
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("no se pudo iniciar QEMU: {error}"))?;
    let stdin = child.stdin.take();
    let mut readers: Vec<thread::JoinHandle<()>> = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let sink = Arc::clone(&output);
        readers.push(thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            while let Ok(read) = stdout.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                let chunk = String::from_utf8_lossy(&buffer[..read]).into_owned();
                if let Ok(mut guard) = sink.lock() {
                    guard.push_str(&chunk);
                }
            }
        }));
    }
    if let Some(mut stderr) = child.stderr.take() {
        let sink = Arc::clone(&output);
        readers.push(thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            while let Ok(read) = stderr.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                let chunk = String::from_utf8_lossy(&buffer[..read]).into_owned();
                if let Ok(mut guard) = sink.lock() {
                    guard.push_str(&chunk);
                }
            }
        }));
    }
    drop(readers);
    Ok(Session {
        child,
        stdin,
        output,
    })
}
fn spawn(smp: &str) -> Result<Session, String> {
    let firmware = ovmf_firmware();
    let mut command = Command::new("qemu-system-x86_64");
    command.args([
        "-M",
        "q35,accel=tcg",
        "-cpu",
        "max",
        "-smp",
        smp,
        "-m",
        "2G",
        "-cdrom",
        image_iso(),
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
    if let Some(firmware) = firmware {
        command.args(["-bios", firmware]);
    } else {
        println!("[xtask] AVISO: sin OVMF; se arranca en modo BIOS híbrido.");
    }
    spawn_command(&mut command)
}
fn image_iso() -> &'static str {
    crate::image::ISO_OUTPUT
}
pub(crate) fn tail(text: &str, lines: usize) -> String {
    text.lines()
        .rev()
        .take(lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}
fn failure(session: &Session, message: String) -> String {
    let snapshot = session.snapshot();
    format!(
        "{message}\n--- últimas líneas del serie ---\n{}",
        tail(&snapshot, 40)
    )
}
/// Ejecuta una sentencia y espera a que la shell vuelva a mostrar el prompt.
fn run_command(session: &mut Session, command: &str) -> Result<(), String> {
    let len_before = session.snapshot().len();
    session.send_line(command)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let snapshot = session.snapshot();
        if snapshot.len() > len_before && snapshot.ends_with("$ ") {
            return Ok(());
        }
        if let Some(fatal) = session.fatal_in_output() {
            return Err(format!("«{command}» provocó «{fatal}»"));
        }
        match session
            .child
            .try_wait()
            .map_err(|error| format!("QEMU wait: {error}"))?
        {
            Some(status) => return Err(format!("QEMU terminó ({status}) en «{command}»")),
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(200)),
            None => return Err(format!("timeout ejecutando «{command}»")),
        }
    }
}
pub(super) fn run() -> Result<(), String> {
    let smp = std::env::args()
        .position(|arg| arg == "--smp")
        .and_then(|index| std::env::args().nth(index + 1))
        .unwrap_or_else(|| "2".to_string());
    let skip_build = std::env::args().any(|arg| arg == "--no-build");
    if !skip_build {
        println!("[xtask] smoke: construyendo kernel, userland e ISO...");
        build::task_build()?;
    }
    iso::task_iso()?;
    println!("[xtask] smoke: arrancando QEMU (smp={smp})...");
    let mut session = spawn(&smp)?;
    session
        .wait_for(BOOT_MARKER, Duration::from_secs(300))
        .map_err(|error| failure(&session, error))?;
    println!("[xtask] smoke: shell en Ring 3 detectada");
    run_command(&mut session, "help").map_err(|error| failure(&session, error))?;
    let snapshot = session.snapshot();
    if !snapshot.contains("Comandos internos de Vanish") {
        return Err(failure(
            &session,
            "«help» no devolvió la lista de comandos".into(),
        ));
    }
    println!("[xtask] smoke: sys_write de usuariospace verificado");
    run_command(&mut session, "pwd").map_err(|error| failure(&session, error))?;
    run_command(&mut session, "cat /home/user/notas.txt")
        .map_err(|error| failure(&session, error))?;
    let snapshot = session.snapshot();
    if !snapshot.contains("Bienvenido a LixOS") {
        return Err(failure(
            &session,
            "«cat» no leyó /home/user/notas.txt".into(),
        ));
    }
    println!("[xtask] smoke: lectura de fichero y copia user<->kernel verificadas");
    run_command(&mut session, "ls").map_err(|error| failure(&session, error))?;
    println!("[xtask] smoke: sys_list_dir verificado");
    run_command(&mut session, "hello").map_err(|error| failure(&session, error))?;
    println!("[xtask] smoke: ejecución de binario ELF desde Ring 3 verificada");
    run_command(&mut session, "ps").map_err(|error| failure(&session, error))?;
    let snapshot = session.snapshot();
    if !snapshot.contains("PID ESTADO") || !snapshot.contains("vanish") {
        return Err(failure(
            &session,
            "«ps» no listó procesos: se perdió el derecho INSPECT (política `declared`)".into(),
        ));
    }
    println!("[xtask] smoke: sys_process_list con derecho INSPECT verificado");
    if let Some(fatal) = session.fatal_in_output() {
        return Err(failure(
            &session,
            format!("marcador fatal «{fatal}» en el serie"),
        ));
    }
    session.stop();
    println!("[xtask] smoke: OK");
    Ok(())
}
