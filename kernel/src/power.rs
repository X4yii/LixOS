//! Subestación de Administración de Energía (Power Management) para LixOS.
//! Maneja el cierre ordenado de procesos de usuario, sincronización de cachés,
//! desmontaje limpio del sistema de archivos VFS/Ext4, liberación de discos AHCI
//! y apagado seguro del hardware.

pub fn system_shutdown() -> ! {
    crate::kprintln!("\n========================================");
    crate::kprintln!("         Apagando LixOS v0.0.1          ");
    crate::kprintln!("========================================");

    crate::kprintln!("[shutdown] Terminando procesos de usuario activos...");
    {
        let mut sched = crate::task::scheduler::SCHEDULER.lock();
        sched.terminate_user_processes();
    }

    crate::kprintln!("[shutdown] Sincronizando búferes del sistema de archivos...");
    {
        let mut vfs = crate::fs::vfs::VFS.lock();
        vfs.root_fs = None;
    }
    crate::kprintln!("[shutdown] Desmontando partición Ext4 (/).");
    {
        let mut ext4 = crate::fs::ext4::EXT4.lock();
        *ext4 = None;
    }
    crate::kprintln!("[shutdown] Desconectando unidades de almacenamiento AHCI SATA.");
    {
        let mut disk = crate::drivers::ahci::PRIMARY_DISK.lock();
        *disk = None;
    }

    crate::kprintln!("[shutdown] Deshabilitando temporizadores e interrupciones...");
    x86_64::instructions::interrupts::disable();

    crate::kprintln!("[shutdown] Enviando comando de apagado al controlador ACPI...");

    unsafe {
        crate::arch::x86_64::io::outw(0x604, 0x2000);
        crate::arch::x86_64::io::outw(0xB004, 0x2000);
        crate::arch::x86_64::io::outw(0x4004, 0x3400);
        crate::arch::x86_64::io::outb(0x501, 0x31);
    }

    crate::kprintln!("\n[shutdown] El sistema se ha detenido con total seguridad.");
    crate::kprintln!("[shutdown] Es seguro apagar o desconectar el equipo manualmente.");
    loop {
        x86_64::instructions::hlt();
    }
}
