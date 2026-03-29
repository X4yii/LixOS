use std::env;
use std::process::{Command, exit};

fn main() {
    let mut args = env::args().skip(1);
    let task = args.next().unwrap_or_else(|| "help".to_string());

    match task.as_str() {
        "build" => task_build(),
        "check" => task_check(),
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
    println!("  run     - Compila y lanza el kernel en QEMU");
}

fn task_build() {
    println!("[xtask] Compilando lix-kernel (x86_64-unknown-none)...");
    let status = Command::new("cargo")
        .args([
            "build",
            "--package",
            "kernel",
            "--target",
            "x86_64-unknown-none",
        ])
        .status()
        .expect("Error al ejecutar cargo build");

    if !status.success() {
        eprintln!("[xtask] Fallo al compilar el kernel.");
        exit(1);
    }
    println!("[xtask] Kernel compilado exitosamente.");
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

fn task_run() {
    task_build();
    println!("[xtask] Verificando presencia de QEMU...");
    let qemu_check = Command::new("which")
        .arg("qemu-system-x86_64")
        .output();

    match qemu_check {
        Ok(output) if output.status.success() => {
            println!("[xtask] Ejecutando QEMU...");
            // Ejecutar kernel directamente o vía cargador
            let mut qemu = Command::new("qemu-system-x86_64");
            qemu.args([
                "-M", "q35",
                "-m", "2G",
                "-kernel", "target/x86_64-unknown-none/debug/kernel",
                "-serial", "stdio",
            ]);
            let status = qemu.status().expect("Error al lanzar QEMU");
            exit(status.code().unwrap_or(0));
        }
        _ => {
            println!("[xtask] AVISO: 'qemu-system-x86_64' no está instalado en el host.");
            println!("[xtask] Instálalo con: sudo pacman -S qemu-desktop edk2-ovmf xorriso");
        }
    }
}
