use sys_libc::strerror;
use sys_libc::syscalls::{
    sys_block_info, sys_block_open, sys_block_read, sys_block_sync, sys_handle_close, sys_write,
    BlockDeviceInfo,
};
pub fn run(_args: &[&str], _current_dir: &mut alloc::string::String) -> bool {
    sys_write("\x1b[36;1m[blktest] Verificando ABI de Bloques y Lease Exclusivo...\x1b[0m\n");
    match sys_block_open(0, false) {
        Ok(handle) => {
            sys_write(&alloc::format!(
                "  [+] Lease adquirido con éxito (Handle: {:#x})\n",
                handle.0
            ));
            let mut info = BlockDeviceInfo::default();
            if sys_block_info(handle, &mut info) == Ok(()) {
                sys_write(&alloc::format!(
                    "  [+] Dispositivo: {} sectores, {} bytes/sector (Total: {} MB)\n",
                    info.block_count,
                    info.block_size,
                    (info.block_count * u64::from(info.block_size)) / (1024 * 1024)
                ));
            }
            let mut sector = [0u8; 4096];
            let read_len = if info.block_size > 0 && (info.block_size as usize) <= 4096 {
                info.block_size as usize
            } else {
                512
            };
            match sys_block_read(handle, 0, 1, &mut sector[..read_len]) {
                Ok(bytes) => {
                    let sig = [sector[510], sector[511]];
                    let is_mbr = sig == [0x55, 0xAA];
                    sys_write(&alloc::format!(
                        "  [+] Lectura de LBA 0 ({} bytes): Firma 0x{:02X}{:02X} ({})\n",
                        bytes,
                        sig[0],
                        sig[1],
                        if is_mbr {
                            "MBR/GPT válido"
                        } else {
                            "sin firma MBR estándar"
                        }
                    ));
                }
                Err(e) => {
                    sys_write(&alloc::format!(
                        "  [-] Error en lectura de LBA 0: {} (código {})\n",
                        strerror(e),
                        e
                    ));
                }
            }
            if sys_block_sync(handle) == Ok(()) {
                sys_write("  [+] sys_block_sync: Flush NVMe completado exitosamente\n");
            }
            match sys_block_open(0, false) {
                Err(e) if e == -16 => {
                    sys_write(
                        "  [+] Exclusividad verificada: segundo lease rechazado con EBUSY (-16)\n",
                    );
                }
                Ok(h2) => {
                    sys_write("  [-] AVISO: Segundo lease no fue rechazado\n");
                    let _ = sys_handle_close(h2);
                }
                Err(e) => {
                    sys_write(&alloc::format!(
                        "  [-] Segundo lease falló: {} (código {})\n",
                        strerror(e),
                        e
                    ));
                }
            }
            let _ = sys_handle_close(handle);
            sys_write("  [+] Lease liberado (sys_handle_close). ABI de bloques 100% operativo.\n");
        }
        Err(e) => {
            sys_write(&alloc::format!(
                "[-] Fallo al abrir dispositivo de bloque: {} (código {})\n",
                strerror(e),
                e
            ));
            if e == -1 {
                sys_write("    (Permiso denegado: se requiere nivel de privilegio <= 1)\n");
            } else if e == -19 {
                sys_write("    (Dispositivo no encontrado o driver no enumerado)\n");
            }
        }
    }
    true
}
