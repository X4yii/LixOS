//! ARCHIVO GENERADO AUTOMÁTICAMENTE POR `cargo xtask codegen`
//! NO EDITAR DIRECTAMENTE. Fuente de la verdad: `abi/syscalls.toml`.

#[derive(Debug, Clone, Copy)]
pub struct SyscallEntry {
    pub id: u64,
    pub name: &'static str,
    pub description: &'static str,
    pub required_rights: &'static str,
}

pub const SYSCALL_REGISTRY: &[SyscallEntry] = &[
    SyscallEntry { id: 0x0, name: "sys_abi_version", description: "Devuelve (major << 16 | minor) del ABI del kernel", required_rights: "NONE" },
    SyscallEntry { id: 0x1, name: "sys_handle_close", description: "Cierra un handle y libera el objeto si no quedan referencias", required_rights: "NONE" },
    SyscallEntry { id: 0x2, name: "sys_handle_dup", description: "Duplica un handle atenuando sus derechos", required_rights: "DUP" },
    SyscallEntry { id: 0x7, name: "sys_revoker_create", description: "Crea un objeto de revocación activa", required_rights: "NONE" },
    SyscallEntry { id: 0x8, name: "sys_revoker_revoke", description: "Invalida todos los handles y memoria virtual ligados al revoker", required_rights: "NONE" },
    SyscallEntry { id: 0x100, name: "sys_process_spawn", description: "Crea y arranca un proceso en un Job determinado", required_rights: "SPAWN" },
    SyscallEntry { id: 0x101, name: "sys_process_exit", description: "Termina el proceso actual con código de estado", required_rights: "NONE" },
    SyscallEntry { id: 0x200, name: "sys_vmo_create", description: "Crea un objeto de memoria anónima", required_rights: "NONE" },
    SyscallEntry { id: 0x202, name: "sys_vm_map", description: "Mapea un VMO en el espacio virtual con contabilidad estricta", required_rights: "MAP" },
    SyscallEntry { id: 0x700, name: "sys_display_info", description: "Consulta modos y resolución del display lix-drm", required_rights: "NONE" },
    SyscallEntry { id: 0x702, name: "sys_display_present", description: "Presenta un VMO en pantalla con vblank synchronization", required_rights: "DISPLAY_OWN" },
    SyscallEntry { id: 0x801, name: "sys_log_write", description: "Escribe texto en el búfer circular klog", required_rights: "LOG" },
    SyscallEntry { id: 0x102, name: "sys_process_kill", description: "Envía una señal POSIX a un proceso", required_rights: "MANAGE" },
    SyscallEntry { id: 0x10c, name: "sys_process_list", description: "Enumera los procesos del sistema hacia un búfer de usuario", required_rights: "INSPECT" },
    SyscallEntry { id: 0x703, name: "sys_terminal_info", description: "Consulta dimensiones y escala geométrica de la terminal activa", required_rights: "NONE" },
    SyscallEntry { id: 0x704, name: "sys_terminal_set_scale", description: "Ajusta la escala tipográfica o zoom de la terminal", required_rights: "NONE" },
    SyscallEntry { id: 0x706, name: "sys_set_keymap", description: "Configura la distribución activa del teclado PS/2", required_rights: "DEVICES" },
    SyscallEntry { id: 0x707, name: "sys_get_keymap", description: "Consulta la distribución activa del teclado PS/2", required_rights: "NONE" },
    SyscallEntry { id: 0x803, name: "sys_power", description: "Apagado y reinicio ordenado de la máquina", required_rights: "POWER" },
    SyscallEntry { id: 0x80a, name: "sys_monitor_metrics", description: "Consulta telemetría del sistema (RAM, CPU ticks, métricas de E/S)", required_rights: "INSPECT" },
    SyscallEntry { id: 0x820, name: "sys_block_open", description: "Adquiere un lease exclusivo sobre un dispositivo de bloques físico", required_rights: "DEVICES" },
    SyscallEntry { id: 0x821, name: "sys_block_info", description: "Consulta geometría y tamaño de bloque lógico del dispositivo", required_rights: "READ" },
    SyscallEntry { id: 0x822, name: "sys_block_read", description: "Lee bloques lógicos contiguos desde almacenamiento físico", required_rights: "READ" },
    SyscallEntry { id: 0x823, name: "sys_block_write", description: "Escribe bloques lógicos contiguos a almacenamiento físico", required_rights: "WRITE" },
    SyscallEntry { id: 0x824, name: "sys_block_sync", description: "Ejecuta Flush en la controladora para garantizar persistencia no volátil", required_rights: "WRITE" },
];

/// Consulta el nombre y descripción de una syscall por su número de llamada.
pub fn lookup_syscall(id: u64) -> Option<&'static SyscallEntry> {
    SYSCALL_REGISTRY.iter().find(|e| e.id == id)
}
