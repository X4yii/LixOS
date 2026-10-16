mod blk;
mod fs;
mod net;
mod system;
use alloc::string::String;
pub struct Command {
    pub name: &'static str,
    pub usage: &'static str,
    pub help: &'static str,
    pub run: fn(&[&str], &mut String) -> bool,
}
pub const COMMANDS: &[Command] = &[
    Command {
        name: "cd",
        usage: "[dir]",
        help: "Cambia el directorio actual (por defecto /home/user o ~)",
        run: fs::cd,
    },
    Command {
        name: "pwd",
        usage: "",
        help: "Muestra el directorio de trabajo actual",
        run: fs::pwd,
    },
    Command {
        name: "help",
        usage: "",
        help: "Muestra este mensaje de ayuda",
        run: system::help,
    },
    Command {
        name: "clear",
        usage: "",
        help: "Limpia la pantalla de la terminal",
        run: system::clear,
    },
    Command {
        name: "echo",
        usage: "",
        help: "Imprime argumentos en pantalla (soporta > y >>)",
        run: system::echo,
    },
    Command {
        name: "write",
        usage: "<f>",
        help: "Escribe texto en un archivo del disco",
        run: fs::write,
    },
    Command {
        name: "mkdir",
        usage: "<d>",
        help: "Crea directorios (soporta -p)",
        run: fs::mkdir,
    },
    Command {
        name: "rm",
        usage: "<ruta>",
        help: "Elimina archivos o directorios (soporta -rf)",
        run: fs::rm,
    },
    Command {
        name: "edit",
        usage: "<f>",
        help: "Editor de texto interactivo TUI (estilo Nano/IDE)",
        run: fs::edit,
    },
    Command {
        name: "ls",
        usage: "[dir]",
        help: "Lista archivos del sistema de archivos",
        run: fs::ls,
    },
    Command {
        name: "cat",
        usage: "<file>",
        help: "Muestra el contenido de un archivo",
        run: fs::cat,
    },
    Command {
        name: "zoom",
        usage: "",
        help: "Redimensiona celdas (zoom in/out/reset o Ctrl + -/+)",
        run: system::zoom,
    },
    Command {
        name: "keymap",
        usage: "<nombre>",
        help: "Configura distribución de teclado (latam, es, us, uk, de, fr, etc.)",
        run: system::keymap,
    },
    Command {
        name: "blktest",
        usage: "",
        help: "Diagnóstico y verificación del ABI de bloques / NVMe",
        run: blk::run,
    },
    Command {
        name: "net",
        usage: "<send|listen|ping>",
        help: "Diagnostico y comunicacion por sockets de red UDP",
        run: net::run,
    },
    Command {
        name: "ping",
        usage: "<ip>",
        help: "Envia sonda de red UDP a una direccion IP",
        run: |args, current_dir| {
            let mut full_args = alloc::vec!["ping"];
            full_args.extend_from_slice(args);
            net::run(&full_args, current_dir)
        },
    },
    Command {
        name: "ps",
        usage: "",
        help: "Lista los procesos en ejecución",
        run: system::ps,
    },
    Command {
        name: "shutdown",
        usage: "",
        help: "Cierra procesos, desmonta particiones y apaga el equipo",
        run: system::shutdown,
    },
    Command {
        name: "exit",
        usage: "",
        help: "Sale de la shell",
        run: system::exit,
    },
];
pub const SYSTEM_BINARIES: &[(&str, &str)] = &[
    (
        "ltop",
        "Administrador de procesos interactivo (en /bin/system)",
    ),
    (
        "lixinstall",
        "Instalador TUI del sistema operativo (en /bin/system)",
    ),
    ("hello", "Binario de prueba Ring 3 C (en /bin/apps)"),
];
pub fn try_execute_builtin(cmd: &str, args: &[&str], current_dir: &mut String) -> bool {
    COMMANDS
        .iter()
        .find(|command| command.name == cmd)
        .is_some_and(|command| (command.run)(args, current_dir))
}
