const COMANDOS: &[(&str, &str)] = &[
    (
        "build",
        "Compila el kernel para bare-metal (x86_64-unknown-none)",
    ),
    ("check", "Verifica sintaxis y tipos del workspace completo"),
    (
        "codegen",
        "Genera tablas de syscalls e includes desde abi/syscalls.toml",
    ),
    (
        "iso",
        "Prepara el árbol de arranque y configuración de Limine",
    ),
    (
        "run [--headless] [--nvme-4k]",
        "Compila y lanza QEMU; --nvme-4k prueba sectores NVMe de 4 KiB",
    ),
    ("asm", "Ensambla archivos .S en kernel/asm/"),
    (
        "deps",
        "Verifica e instala dependencias requeridas del sistema",
    ),
    (
        "test-nvme",
        "Prueba arranque UEFI+NVMe de 512 B y 4 KiB en QEMU",
    ),
    (
        "test-installer [--no-build]",
        "Instala en un disco de 10 GiB (partición ext4 vacía) y valida con e2fsck/debugfs",
    ),
    (
        "smoke [--smp N] [--no-build]",
        "Test de humo: arranca QEMU y ejercita la shell de usuariospace",
    ),
    (
        "bench / microbench",
        "Ejecuta la suite de microbenchmarks 100% Rust con RDTSC",
    ),
    (
        "nvme-image [PATH]",
        "Crea una imagen GPT+ext4 desechable para pruebas (no sobrescribe)",
    ),
    (
        "nvme-image-4k [PATH]",
        "Igual, con sectores lógicos NVMe de 4 KiB",
    ),
];
pub fn print_help() {
    println!("LixOS xtask Build Runner");
    println!("Uso: cargo xtask <comando>");
    println!();
    println!("Comandos disponibles:");
    for (command, description) in COMANDOS {
        println!("  {command:<30} - {description}");
    }
}
