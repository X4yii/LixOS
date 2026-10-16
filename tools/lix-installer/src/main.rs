mod disk;
mod fat32;
mod image;
mod plan;
use disk::{guid_string, inspect_image, DiskSnapshot};
use plan::{plan_for, InstallMode, InstallPlan};
use std::env;
use std::path::Path;
const DEFAULT_BLOCK_SIZE: u32 = 512;
fn help() {
    println!("lix-installer — auditoría de imágenes GPT y planificador offline (solo lectura)");
    println!("\nUso:");
    println!("  lix-installer inspect <IMAGE> [BLOCK_SIZE]");
    println!("  lix-installer plan <IMAGE> <whole-disk|free-space> [--confirm-target <IMAGE>] [BLOCK_SIZE]");
    println!(
        "  lix-installer image <NEW_IMAGE> <KERNEL> <INITRD> <BOOTX64.EFI> <ROOTFS_DIR> [SIZE_GIB] [BLOCK_SIZE]"
    );
    println!("\n'image' solo crea imágenes raw nuevas; rechaza rutas existentes y nunca abre nodos de dispositivo.");
    println!("No hay todavía aplicación a discos físicos ni recuperación transaccional desde el LiveISO.");
}
fn show_disk(disk: &DiskSnapshot) {
    let bytes = disk.block_count.saturating_mul(u64::from(disk.block_size));
    println!(
        "Disco: {} bytes ({} bloques × {} B)",
        bytes, disk.block_count, disk.block_size
    );
    println!(
        "GPT: {} .. {} utilizables; GUID {}",
        disk.first_usable_lba,
        disk.last_usable_lba,
        guid_string(disk.disk_guid)
    );
    if disk.partitions.is_empty() {
        println!("Particiones: ninguna");
    }
    for (index, part) in disk.partitions.iter().enumerate() {
        println!(
            "  {}: LBA {}..{} ({} bloques), tipo {}, GUID {}",
            index + 1,
            part.first_lba,
            part.last_lba,
            part.block_count().unwrap_or(0),
            guid_string(part.type_guid),
            guid_string(part.unique_guid)
        );
    }
}
fn show_plan(path: &str, disk: &DiskSnapshot, plan: &InstallPlan) {
    println!("\nPLAN — solo lectura; no se ha escrito nada");
    println!("Destino: {path}");
    println!(
        "Modo: {}",
        match plan.mode {
            InstallMode::WholeDisk => "disco completo",
            InstallMode::FreeSpace => "espacio sin asignar",
        }
    );
    println!(
        "Tabla GPT: {}",
        if plan.create_gpt {
            "crear nueva"
        } else {
            "preservar"
        }
    );
    println!(
        "Particiones existentes: {}",
        if plan.erase_existing_partitions {
            "SE ELIMINARÍAN al aplicar"
        } else {
            "se preservan"
        }
    );
    if let Some(esp_guid) = plan.reused_esp {
        println!("ESP: reutilizar {}, nunca formatear", guid_string(esp_guid));
    }
    for partition in &plan.partitions_to_create {
        let mib = partition
            .blocks()
            .saturating_mul(u64::from(disk.block_size))
            / (1024 * 1024);
        println!(
            "Crear {}: LBA {}..{} ({} MiB), tipo {}",
            partition.name,
            partition.first_lba,
            partition.last_lba,
            mib,
            guid_string(partition.type_guid)
        );
    }
    println!(
        "Se preservan {} particiones existentes.",
        plan.preserved_partitions.len()
    );
    println!("Sin resize, particionado manual ni escritura. Revisión humana obligatoria antes de una futura fase apply.");
}
fn parse_block_size(value: Option<&String>) -> Result<u32, String> {
    match value {
        Some(value) => value
            .parse::<u32>()
            .map_err(|_| "BLOCK_SIZE debe ser entero".into()),
        None => Ok(DEFAULT_BLOCK_SIZE),
    }
}
fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--help" | "-h") | None => {
            help();
            Ok(())
        }
        Some("inspect") if args.len() == 2 || args.len() == 3 => {
            let block_size = parse_block_size(args.get(2))?;
            let disk = inspect_image(Path::new(&args[1]), block_size)?;
            show_disk(&disk);
            println!("\nSolo se aceptan archivos de imagen regulares. No se abrió ningún dispositivo para escritura.");
            Ok(())
        }
        Some("plan") if args.len() >= 3 => {
            let image = &args[1];
            let mode = match args[2].as_str() {
                "whole-disk" => InstallMode::WholeDisk,
                "free-space" => InstallMode::FreeSpace,
                _ => return Err("mode debe ser whole-disk o free-space".into()),
            };
            let mut confirmation = None;
            let mut block_size = DEFAULT_BLOCK_SIZE;
            let mut index = 3;
            while index < args.len() {
                if args[index] == "--confirm-target" {
                    index += 1;
                    confirmation = args.get(index).map(String::as_str);
                    if confirmation.is_none() {
                        return Err("--confirm-target requiere el nombre exacto del destino".into());
                    }
                } else {
                    block_size = args[index]
                        .parse()
                        .map_err(|_| "argumento adicional inválido".to_string())?;
                }
                index += 1;
            }
            if mode == InstallMode::WholeDisk && confirmation != Some(image.as_str()) {
                return Err(
                    "whole-disk requiere --confirm-target con el mismo nombre exacto de IMAGE"
                        .into(),
                );
            }
            let disk = inspect_image(Path::new(image), block_size)?;
            let plan = plan_for(mode, &disk)?;
            show_disk(&disk);
            show_plan(image, &disk, &plan);
            Ok(())
        }
        Some("image") if (6..=8).contains(&args.len()) => {
            let size_gib = args
                .get(6)
                .map(|value| value.parse::<u64>().map_err(|_| "SIZE_GIB debe ser entero"))
                .transpose()?
                .unwrap_or(10);
            let block_size = args
                .get(7)
                .map(|value| {
                    value
                        .parse::<u32>()
                        .map_err(|_| "BLOCK_SIZE debe ser entero")
                })
                .transpose()?
                .unwrap_or(512);
            let root_guid = image::create_bootable_image(
                Path::new(&args[1]),
                size_gib,
                block_size,
                Path::new(&args[2]),
                Path::new(&args[3]),
                Path::new(&args[4]),
                Path::new(&args[5]),
            )?;
            println!("Imagen UEFI raw creada (LBA lógico {block_size} B) y GPT verificada.");
            println!("PARTUUID raíz: {root_guid}");
            println!("El destino se creó como archivo nuevo; todavía no se escribió ningún dispositivo físico.");
            Ok(())
        }
        _ => {
            help();
            Err("comando o argumentos inválidos".into())
        }
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("lix-installer: {error}");
        std::process::exit(2);
    }
}
