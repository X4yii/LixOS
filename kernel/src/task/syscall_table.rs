#[derive(Debug, Clone, Copy)]
pub struct SyscallEntry {
    pub id: u64,
    pub name: &'static str,
    pub description: &'static str,
    pub required_rights: &'static str,
}

pub const SYSCALL_REGISTRY: &[SyscallEntry] = &[
    SyscallEntry {
        id: 0x0,
        name: "sys_abi_version",
        description: "Devuelve (major << 16 | minor) del ABI del kernel",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x1,
        name: "sys_handle_close",
        description: "Cierra un handle y libera el objeto si no quedan referencias",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x2,
        name: "sys_handle_dup",
        description: "Duplica un handle atenuando sus derechos",
        required_rights: "DUP",
    },
    SyscallEntry {
        id: 0x3,
        name: "sys_handle_dup2",
        description: "Duplica un handle hacia un índice de descriptor específico",
        required_rights: "DUP",
    },
    SyscallEntry {
        id: 0x100,
        name: "sys_process_spawn",
        description: "Crea y arranca un proceso en un Job determinado",
        required_rights: "SPAWN",
    },
    SyscallEntry {
        id: 0x101,
        name: "sys_process_exit",
        description: "Termina el proceso actual con código de estado",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x102,
        name: "sys_process_kill",
        description:
            "Envía una señal POSIX a un proceso (hijo propio, uno mismo, o con derecho MANAGE)",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x107,
        name: "sys_sleep",
        description: "Bloquea el proceso actual durante un número de milisegundos",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x10a,
        name: "sys_waitpid",
        description:
            "Espera a la terminación de un proceso hijo concreto (hijo propio o con derecho MANAGE)",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x10c,
        name: "sys_process_list",
        description: "Enumera los procesos del sistema hacia un búfer de usuario",
        required_rights: "INSPECT",
    },
    SyscallEntry {
        id: 0x200,
        name: "sys_mmap",
        description: "Mapea memoria virtual anónima para el proceso",
        required_rights: "MAP",
    },
    SyscallEntry {
        id: 0x201,
        name: "sys_munmap",
        description: "Desmapea un rango de memoria virtual del proceso",
        required_rights: "MAP",
    },
    SyscallEntry {
        id: 0x300,
        name: "sys_open",
        description: "Abre un archivo devolviendo un descriptor/handle",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x301,
        name: "sys_read",
        description: "Lee bytes desde un handle hacia un búfer de usuario",
        required_rights: "READ",
    },
    SyscallEntry {
        id: 0x302,
        name: "sys_write",
        description: "Escribe bytes desde un búfer de usuario hacia un handle",
        required_rights: "WRITE",
    },
    SyscallEntry {
        id: 0x303,
        name: "sys_lseek",
        description: "Reposiciona el cursor de un descriptor de archivo",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x306,
        name: "sys_list_dir",
        description: "Enumera las entradas de un directorio hacia un búfer de usuario",
        required_rights: "READ",
    },
    SyscallEntry {
        id: 0x307,
        name: "sys_read_file",
        description: "Lee el contenido completo de un archivo hacia un búfer de usuario",
        required_rights: "READ",
    },
    SyscallEntry {
        id: 0x308,
        name: "sys_write_file",
        description: "Sobrescribe un archivo con el contenido de un búfer de usuario",
        required_rights: "WRITE",
    },
    SyscallEntry {
        id: 0x309,
        name: "sys_read_nonblocking",
        description: "Lee bytes disponibles sin bloquear el proceso",
        required_rights: "READ",
    },
    SyscallEntry {
        id: 0x30a,
        name: "sys_remove_file",
        description: "Elimina un archivo o directorio recursivo del sistema de archivos",
        required_rights: "WRITE",
    },
    SyscallEntry {
        id: 0x30b,
        name: "sys_mkdir",
        description: "Crea un directorio en el sistema de archivos",
        required_rights: "WRITE",
    },
    SyscallEntry {
        id: 0x400,
        name: "sys_channel_create",
        description: "Crea un par de extremos de canal IPC bidireccional",
        required_rights: "TRANSFER",
    },
    SyscallEntry {
        id: 0x401,
        name: "sys_channel_send",
        description: "Envía datos y transfiere handles a través de un canal",
        required_rights: "WRITE",
    },
    SyscallEntry {
        id: 0x402,
        name: "sys_channel_recv",
        description: "Recibe datos y handles transferidos desde un canal",
        required_rights: "READ",
    },
    SyscallEntry {
        id: 0x703,
        name: "sys_terminal_info",
        description: "Consulta dimensiones y escala geométrica de la terminal activa",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x704,
        name: "sys_terminal_set_scale",
        description: "Ajusta la escala tipográfica o zoom de la terminal",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x705,
        name: "sys_input_open",
        description: "Abre un flujo de eventos físicos de entrada",
        required_rights: "INPUT_READ",
    },
    SyscallEntry {
        id: 0x706,
        name: "sys_set_keymap",
        description: "Configura la distribución activa del teclado PS/2",
        required_rights: "DEVICES",
    },
    SyscallEntry {
        id: 0x707,
        name: "sys_get_keymap",
        description: "Consulta la distribución activa del teclado PS/2",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x801,
        name: "sys_log_write",
        description: "Escribe texto en el búfer circular klog",
        required_rights: "LOG",
    },
    SyscallEntry {
        id: 0x803,
        name: "sys_power",
        description: "Apagado y reinicio ordenado de la máquina",
        required_rights: "POWER",
    },
    SyscallEntry {
        id: 0x80a,
        name: "sys_monitor_metrics",
        description: "Consulta telemetría del sistema (RAM, CPU ticks, métricas de E/S)",
        required_rights: "INSPECT",
    },
    SyscallEntry {
        id: 0x820,
        name: "sys_block_open",
        description: "Adquiere un lease exclusivo sobre un dispositivo de bloques físico",
        required_rights: "DEVICES",
    },
    SyscallEntry {
        id: 0x821,
        name: "sys_block_info",
        description: "Consulta geometría y tamaño de bloque lógico del dispositivo",
        required_rights: "READ",
    },
    SyscallEntry {
        id: 0x822,
        name: "sys_block_read",
        description: "Lee bloques lógicos contiguos desde almacenamiento físico",
        required_rights: "READ",
    },
    SyscallEntry {
        id: 0x823,
        name: "sys_block_write",
        description: "Escribe bloques lógicos contiguos a almacenamiento físico",
        required_rights: "WRITE",
    },
    SyscallEntry {
        id: 0x824,
        name: "sys_block_sync",
        description: "Ejecuta Flush en la controladora para garantizar persistencia no volátil",
        required_rights: "WRITE",
    },
    SyscallEntry {
        id: 0x825,
        name: "sys_usb_enumerate",
        description: "Enumera los dispositivos USB detectados",
        required_rights: "ENUMERATE",
    },
    SyscallEntry {
        id: 0x830,
        name: "sys_socket_open",
        description: "Crea un handle de socket UDP IPv4 no bloqueante",
        required_rights: "NET_CONNECT",
    },
    SyscallEntry {
        id: 0x831,
        name: "sys_socket_bind",
        description: "Asocia un socket UDP a un puerto local IPv4",
        required_rights: "WRITE",
    },
    SyscallEntry {
        id: 0x832,
        name: "sys_socket_sendto",
        description: "Envía un datagrama UDP IPv4",
        required_rights: "WRITE",
    },
    SyscallEntry {
        id: 0x833,
        name: "sys_socket_recvfrom",
        description: "Recibe un datagrama UDP IPv4 sin bloquear",
        required_rights: "READ",
    },
    SyscallEntry {
        id: 0x600,
        name: "sys_io_uring_setup",
        description: "Create an io_uring instance",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x601,
        name: "sys_io_uring_submit",
        description: "Submit an SQE to the ring",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x602,
        name: "sys_io_uring_enter",
        description: "Wait for at least one completion",
        required_rights: "NONE",
    },
    SyscallEntry {
        id: 0x603,
        name: "sys_io_uring_peek",
        description: "Peek a completion event",
        required_rights: "NONE",
    },
];

pub fn lookup_syscall(id: u64) -> Option<&'static SyscallEntry> {
    SYSCALL_REGISTRY.iter().find(|e| e.id == id)
}
