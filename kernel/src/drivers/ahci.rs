use crate::drivers::block::{BlockDevice, BlockError, BlockGeometry, DmaBuffer};
use crate::memory::pmm::PAGE_SIZE;
use crate::sync::IrqSafeSpinlock;
use core::sync::atomic::{AtomicBool, Ordering};
pub static PRIMARY_DISK: IrqSafeSpinlock<Option<AhciDisk>> = IrqSafeSpinlock::new(None);
static DISK_IO_LOCK: AtomicBool = AtomicBool::new(false);
const IO_LOCK_SPINS: usize = 10_000_000;
const TASK_FILE_POLLS: usize = 5_000_000;
const COMMAND_ISSUE_POLLS: usize = 10_000_000;
struct DiskIoGuard;
impl DiskIoGuard {
    fn acquire() -> Option<Self> {
        let mut spins = 0;
        while DISK_IO_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            if spins >= IO_LOCK_SPINS {
                return None;
            }
            spins += 1;
            core::hint::spin_loop();
        }
        Some(Self)
    }
}
impl Drop for DiskIoGuard {
    fn drop(&mut self) {
        DISK_IO_LOCK.store(false, Ordering::Release);
    }
}
pub struct GlobalAhciDisk;
impl BlockDevice for GlobalAhciDisk {
    fn geometry(&self) -> Result<BlockGeometry, BlockError> {
        let disk = PRIMARY_DISK.lock().as_ref().copied();
        disk.ok_or(BlockError::NoDevice)?.geometry()
    }
    fn read_blocks(
        &self,
        start: u64,
        count: u16,
        buffer: &mut DmaBuffer,
    ) -> Result<(), BlockError> {
        let disk = PRIMARY_DISK
            .lock()
            .as_ref()
            .copied()
            .ok_or(BlockError::NoDevice)?;
        disk.read_blocks(start, count, buffer)
    }
    fn write_blocks(&self, start: u64, count: u16, buffer: &DmaBuffer) -> Result<(), BlockError> {
        let disk = PRIMARY_DISK
            .lock()
            .as_ref()
            .copied()
            .ok_or(BlockError::NoDevice)?;
        disk.write_blocks(start, count, buffer)
    }
    fn sync(&self) -> Result<(), BlockError> {
        let disk = PRIMARY_DISK
            .lock()
            .as_ref()
            .copied()
            .ok_or(BlockError::NoDevice)?;
        disk.sync()
    }
}
pub static GLOBAL_DISK: GlobalAhciDisk = GlobalAhciDisk;
#[derive(Clone, Copy)]
pub struct AhciDisk {
    pub port: *mut HbaPort,
    pub block_count: u64,
}
unsafe impl Send for AhciDisk {}
unsafe impl Sync for AhciDisk {}
impl BlockDevice for AhciDisk {
    fn geometry(&self) -> Result<BlockGeometry, BlockError> {
        if self.block_count == 0 {
            return Err(BlockError::InvalidGeometry);
        }
        Ok(BlockGeometry {
            block_count: self.block_count,
            block_size: 512,
        })
    }
    fn read_blocks(
        &self,
        start: u64,
        count: u16,
        buffer: &mut DmaBuffer,
    ) -> Result<(), BlockError> {
        let geometry = self.geometry()?;
        let end = start
            .checked_add(count as u64)
            .ok_or(BlockError::InvalidRange)?;
        if count == 0 || end > geometry.block_count || count as usize > PAGE_SIZE / 512 {
            return Err(BlockError::InvalidRange);
        }
        let lba = u32::try_from(start).map_err(|_| BlockError::Unsupported)?;
        let _io_guard = DiskIoGuard::acquire().ok_or(BlockError::Timeout)?;
        let ok = unsafe {
            read_sector(
                self.port,
                lba,
                count,
                buffer.physical_address().get() as *mut u8,
            )
        };
        if ok {
            Ok(())
        } else {
            Err(BlockError::DeviceFault)
        }
    }
    fn write_blocks(&self, start: u64, count: u16, buffer: &DmaBuffer) -> Result<(), BlockError> {
        let geometry = self.geometry()?;
        let end = start
            .checked_add(count as u64)
            .ok_or(BlockError::InvalidRange)?;
        if count == 0 || end > geometry.block_count || count as usize > PAGE_SIZE / 512 {
            return Err(BlockError::InvalidRange);
        }
        let lba = u32::try_from(start).map_err(|_| BlockError::Unsupported)?;
        let _io_guard = DiskIoGuard::acquire().ok_or(BlockError::Timeout)?;
        let ok = unsafe {
            write_sector(
                self.port,
                lba,
                count,
                buffer.physical_address().get() as *mut u8,
            )
        };
        if ok {
            Ok(())
        } else {
            Err(BlockError::DeviceFault)
        }
    }
    fn sync(&self) -> Result<(), BlockError> {
        let _io_guard = DiskIoGuard::acquire().ok_or(BlockError::Timeout)?;
        let ok = unsafe { flush_cache(self.port) };
        if ok {
            Ok(())
        } else {
            Err(BlockError::DeviceFault)
        }
    }
}
#[repr(C, packed)]
pub struct HbaMem {
    pub cap: u32,
    pub ghc: u32,
    pub is: u32,
    pub pi: u32,
    pub vs: u32,
    pub ccc_ctl: u32,
    pub ccc_pts: u32,
    pub em_loc: u32,
    pub em_ctl: u32,
    pub cap2: u32,
    pub bohc: u32,
    pub reserved: [u8; 0x74],
    pub vendor: [u8; 0x60],
    pub ports: [HbaPort; 32],
}
#[repr(C, packed)]
pub struct HbaPort {
    pub clb: u32,
    pub clbu: u32,
    pub fb: u32,
    pub fbu: u32,
    pub is: u32,
    pub ie: u32,
    pub cmd: u32,
    pub rsv0: u32,
    pub tfd: u32,
    pub sig: u32,
    pub ssts: u32,
    pub sctl: u32,
    pub serr: u32,
    pub sact: u32,
    pub ci: u32,
    pub sntf: u32,
    pub fbs: u32,
    pub rsv1: [u32; 11],
    pub vendor: [u32; 4],
}
pub fn init(bus: u8, slot: u8, func: u8) {
    crate::kprintln!(
        "[ahci] Inicializando controladora en {}:{}:{}...",
        bus,
        slot,
        func
    );
    let bar5 = crate::drivers::pci::pci_read_dword(bus, slot, func, 0x24);
    if bar5 & 0x01 != 0 {
        crate::kprintln!("[ahci] AVISO: BAR5 no es Memory Mapped.");
        return;
    }
    let phys_addr = (bar5 & 0xFFFFFFF0) as u64;
    crate::kprintln!("[ahci] ABAR físico encontrado en: {:#x}", phys_addr);
    let cr3: u64;
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3) };
    let pml4_frame = (cr3 & 0x000FFFFFFFFFF000) as usize / PAGE_SIZE;
    unsafe {
        let abar = match crate::memory::vmm::map_mmio_region(
            pml4_frame,
            crate::memory::address::PhysAddr::new(phys_addr),
            core::mem::size_of::<HbaMem>(),
        ) {
            Ok(address) => address.get() as *mut HbaMem,
            Err(error) => {
                crate::kprintln!("[ahci] No se pudo mapear ABAR como MMIO: {}", error);
                return;
            }
        };
        let pi = (*abar).pi;
        crate::kprintln!("[ahci] Puertos implementados (bitmap): {:#b}", pi);
        for i in 0..32 {
            if (pi & (1 << i)) != 0 {
                let ssts = (*abar).ports[i as usize].ssts;
                let det = ssts & 0x0F;
                let ipm = (ssts >> 8) & 0x0F;
                if det == 3 && ipm == 1 {
                    let sig = (*abar).ports[i as usize].sig;
                    if sig == 0x0101 {
                        crate::kprintln!("[ahci] Disco SATA detectado en el Puerto {}", i);
                        let port = &mut (*abar).ports[i as usize] as *mut HbaPort;
                        if !port_rebase(port, i as usize) {
                            continue;
                        }
                        let block_count = match identify_block_count(port) {
                            Ok(block_count) if block_count > 0 => block_count,
                            Ok(_) => {
                                crate::kprintln!(
                                    "[ahci] IDENTIFY reportó 0 sectores en el Puerto {}; disco descartado",
                                    i
                                );
                                continue;
                            }
                            Err(error) => {
                                crate::kprintln!(
                                    "[ahci] Puerto {}: disco no identificado ({})",
                                    i,
                                    error
                                );
                                continue;
                            }
                        };
                        *PRIMARY_DISK.lock() = Some(AhciDisk { port, block_count });
                        crate::fs::ext4::init_from_disk(&GLOBAL_DISK);
                        let _ = crate::drivers::block::register_block_device(
                            "ahci0",
                            &GLOBAL_DISK,
                            false,
                        );
                    } else if sig == 0xEB140101 {
                        crate::kprintln!("[ahci] CD-ROM detectado en el Puerto {}", i);
                    }
                }
            }
        }
    }
}
#[repr(C, packed)]
pub struct HbaCmdHeader {
    pub cfl: u8,
    pub flags: u8,
    pub prdtl: u16,
    pub prdbc: u32,
    pub ctba: u32,
    pub ctbau: u32,
    pub reserved: [u32; 4],
}
#[repr(C, packed)]
pub struct HbaCmdTable {
    pub cfis: [u8; 64],
    pub acmd: [u8; 16],
    pub reserved: [u8; 48],
    pub prdt_entry: [HbaPrdtEntry; 1],
}
#[repr(C, packed)]
pub struct HbaPrdtEntry {
    pub dba: u32,
    pub dbau: u32,
    pub reserved: u32,
    pub dbc: u32,
}
pub unsafe fn port_rebase(port: *mut HbaPort, _port_no: usize) -> bool {
    (*port).cmd &= !1;
    let mut timeout = 1_000_000;
    while (*port).cmd & 0x4000 != 0 && timeout > 0 {
        core::hint::spin_loop();
        timeout -= 1;
    }
    if timeout == 0 {
        crate::kprintln!("[ahci] Timeout deteniendo motor de comandos");
        return false;
    }
    (*port).cmd &= !0x10;
    let mut timeout = 1_000_000;
    while (*port).cmd & 0x8000 != 0 && timeout > 0 {
        core::hint::spin_loop();
        timeout -= 1;
    }
    if timeout == 0 {
        crate::kprintln!("[ahci] Timeout deteniendo recepción FIS");
        return false;
    }
    let Some(frame) = crate::memory::pmm::alloc_frame() else {
        crate::kprintln!("[ahci] Sin memoria para estructuras DMA");
        return false;
    };
    let phys_frame = frame * PAGE_SIZE;
    let hhdm_offset = crate::memory::address::hhdm_offset();
    let Ok(virt_addr) =
        hhdm_offset.map_phys(crate::memory::address::PhysAddr::new(phys_frame as u64))
    else {
        crate::kprintln!("[ahci] Dirección DMA no mapeable: no se pudo inicializar el puerto");
        return false;
    };
    let virt_addr = virt_addr.get();
    core::ptr::write_bytes(virt_addr as *mut u8, 0, 4096);
    (*port).clb = phys_frame as u32;
    (*port).clbu = (phys_frame >> 32) as u32;
    (*port).fb = (phys_frame + 1024) as u32;
    (*port).fbu = ((phys_frame + 1024) >> 32) as u32;
    let cmd_header = virt_addr as *mut HbaCmdHeader;
    let ct_phys = phys_frame + 1280;
    (*cmd_header).ctba = ct_phys as u32;
    (*cmd_header).ctbau = (ct_phys >> 32) as u32;
    (*cmd_header).prdtl = 1;
    let mut timeout = 1_000_000;
    while (*port).cmd & 0x4000 != 0 && timeout > 0 {
        core::hint::spin_loop();
        timeout -= 1;
    }
    (*port).cmd |= 0x10;
    (*port).cmd |= 1;
    true
}
/// Devuelve `None` si alguna de las direcciones físicas del controlador no es
/// traducible: antes era un `panic!` en anillo 0 que dejaba el sistema
/// clavado en mitad de una operación de disco; ahora el llamante lo
/// convierte en un fallo de E/S (`false`/`Err`).
unsafe fn setup_command(port: *mut HbaPort, buf_phys: *mut u8, byte_count: u32) -> Option<*mut u8> {
    let phys_clb = ((*port).clbu as u64) << 32 | (*port).clb as u64;
    let hhdm_offset = crate::memory::address::hhdm_offset();
    let virt_clb = hhdm_offset
        .map_phys(crate::memory::address::PhysAddr::new(phys_clb))
        .ok()?
        .get();
    let cmd_header = virt_clb as *mut HbaCmdHeader;
    (*cmd_header).cfl = 5;
    (*cmd_header).flags = 0;
    (*cmd_header).prdbc = 0;
    let phys_ctba = ((*cmd_header).ctbau as u64) << 32 | (*cmd_header).ctba as u64;
    let virt_ctba = hhdm_offset
        .map_phys(crate::memory::address::PhysAddr::new(phys_ctba))
        .ok()?
        .get();
    let cmd_table = virt_ctba as *mut HbaCmdTable;
    core::ptr::write_bytes(cmd_table as *mut u8, 0, core::mem::size_of::<HbaCmdTable>());
    (*cmd_table).prdt_entry[0].dba = buf_phys as u32;
    (*cmd_table).prdt_entry[0].dbau = (buf_phys as u64 >> 32) as u32;
    (*cmd_table).prdt_entry[0].dbc = byte_count - 1;
    Some((*cmd_table).cfis.as_mut_ptr())
}
unsafe fn wait_task_file_ready(port: *mut HbaPort) -> bool {
    let mut tfd_timeout = TASK_FILE_POLLS;
    while ((*port).tfd & (0x80 | 0x08)) != 0 && tfd_timeout > 0 {
        core::hint::spin_loop();
        tfd_timeout -= 1;
    }
    if tfd_timeout == 0 {
        crate::kprintln!("[ahci] Advertencia: timeout esperando puerto libre (BSY/DRQ)");
        return false;
    }
    true
}
unsafe fn wait_command_complete(port: *mut HbaPort) -> bool {
    let mut polls = 0;
    while (core::ptr::read_volatile(&raw const (*port).ci) & 1) != 0 {
        let tfd = core::ptr::read_volatile(&raw const (*port).tfd);
        let is = core::ptr::read_volatile(&raw const (*port).is);
        if is & 0x40000000 != 0 {
            crate::kprintln!("[ahci] Task File Error! tfd={:#x}, is={:#x}", tfd, is);
            return false;
        }
        if polls >= COMMAND_ISSUE_POLLS {
            let ci = core::ptr::read_volatile(&raw const (*port).ci);
            let ssts = core::ptr::read_volatile(&raw const (*port).ssts);
            crate::kprintln!(
                "[ahci] TIMEOUT! ci={:#x}, tfd={:#x}, is={:#x}, ssts={:#x}",
                ci,
                tfd,
                is,
                ssts
            );
            return false;
        }
        polls += 1;
        core::hint::spin_loop();
    }
    core::ptr::read_volatile(&raw const (*port).is) & 0x40000000 == 0
}
pub unsafe fn read_sector(
    port: *mut HbaPort,
    start_lba: u32,
    count: u16,
    buf_phys: *mut u8,
) -> bool {
    (*port).is = 0xFFFFFFFF;
    let Some(cfis) = setup_command(port, buf_phys, count as u32 * 512) else {
        crate::kprintln!("[ahci] Dirección DMA no mapeable: abortando lectura");
        return false;
    };
    *cfis.offset(0) = 0x27;
    *cfis.offset(1) = 0x80;
    *cfis.offset(2) = 0x24;
    *cfis.offset(4) = (start_lba & 0xFF) as u8;
    *cfis.offset(5) = ((start_lba >> 8) & 0xFF) as u8;
    *cfis.offset(6) = ((start_lba >> 16) & 0xFF) as u8;
    *cfis.offset(7) = 0x40;
    *cfis.offset(8) = ((start_lba >> 24) & 0xFF) as u8;
    *cfis.offset(12) = (count & 0xFF) as u8;
    *cfis.offset(13) = ((count >> 8) & 0xFF) as u8;
    if !wait_task_file_ready(port) {
        return false;
    }
    (*port).ci = 1;
    wait_command_complete(port)
}
/// Escribe `count` sectores de 512 B desde `buf_phys` en `start_lba`.
///
/// Espejo exacto de [`read_sector`] salvo el código de comando: WRITE SECTORS
/// EXT (`0x34`) en lugar de READ SECTORS EXT (`0x24`). El FIS es idéntico
/// (tipo `0x27`, bit C `0x80`, LBA en los bytes 4–6/8–10, dispositivo `0x40`,
/// contador en 12–13); sólo cambia la dirección del flujo de datos, que
/// determina el propio comando a través del PRDT ya configurado.
pub unsafe fn write_sector(
    port: *mut HbaPort,
    start_lba: u32,
    count: u16,
    buf_phys: *mut u8,
) -> bool {
    (*port).is = 0xFFFFFFFF;
    let Some(cfis) = setup_command(port, buf_phys, count as u32 * 512) else {
        crate::kprintln!("[ahci] Dirección DMA no mapeable: abortando escritura");
        return false;
    };
    *cfis.offset(0) = 0x27;
    *cfis.offset(1) = 0x80;
    *cfis.offset(2) = 0x34;
    *cfis.offset(4) = (start_lba & 0xFF) as u8;
    *cfis.offset(5) = ((start_lba >> 8) & 0xFF) as u8;
    *cfis.offset(6) = ((start_lba >> 16) & 0xFF) as u8;
    *cfis.offset(7) = 0x40;
    *cfis.offset(8) = ((start_lba >> 24) & 0xFF) as u8;
    *cfis.offset(12) = (count & 0xFF) as u8;
    *cfis.offset(13) = ((count >> 8) & 0xFF) as u8;
    if !wait_task_file_ready(port) {
        return false;
    }
    (*port).ci = 1;
    wait_command_complete(port)
}
/// Emite FLUSH CACHE EXT (`0xEA`) para forzar la persistencia de escrituras
/// previas en la caché del dispositivo SATA.
///
/// Es un comando sin datos: la cabecera de comando se programa con `prdtl = 0`
/// y sin PRDT, sólo con el FIS de registro.
pub unsafe fn flush_cache(port: *mut HbaPort) -> bool {
    (*port).is = 0xFFFFFFFF;
    let hhdm_offset = crate::memory::address::hhdm_offset();
    let phys_clb = ((*port).clbu as u64) << 32 | (*port).clb as u64;
    let Ok(virt_clb) = hhdm_offset.map_phys(crate::memory::address::PhysAddr::new(phys_clb)) else {
        crate::kprintln!("[ahci] CLB no mapeable: abortando flush");
        return false;
    };
    let cmd_header = virt_clb.get() as *mut HbaCmdHeader;
    (*cmd_header).cfl = 5;
    (*cmd_header).flags = 0;
    (*cmd_header).prdtl = 0;
    let phys_ctba = ((*cmd_header).ctbau as u64) << 32 | (*cmd_header).ctba as u64;
    let Ok(virt_ctba) = hhdm_offset.map_phys(crate::memory::address::PhysAddr::new(phys_ctba))
    else {
        crate::kprintln!("[ahci] CTBA no mapeable: abortando flush");
        return false;
    };
    let cmd_table = virt_ctba.get() as *mut HbaCmdTable;
    let cfis = (*cmd_table).cfis.as_mut_ptr();
    core::ptr::write_bytes(cfis, 0, core::mem::size_of_val(&(*cmd_table).cfis));
    *cfis.offset(0) = 0x27;
    *cfis.offset(1) = 0x80;
    *cfis.offset(2) = 0xEA;
    if !wait_task_file_ready(port) {
        return false;
    }
    (*port).ci = 1;
    wait_command_complete(port)
}
unsafe fn identify_block_count(port: *mut HbaPort) -> Result<u64, &'static str> {
    let buffer = DmaBuffer::allocate().map_err(|_| "sin memoria para IDENTIFY")?;
    let physical = buffer.physical_address().get() as *mut u8;
    (*port).is = 0xFFFFFFFF;
    let Some(cfis) = setup_command(port, physical, 512) else {
        return Err("dirección DMA no mapeable para IDENTIFY");
    };
    *cfis.offset(0) = 0xEC;
    *cfis.offset(1) = 0x80;
    if !wait_task_file_ready(port) {
        return Err("timeout esperando el puerto para IDENTIFY");
    }
    (*port).ci = 1;
    if !wait_command_complete(port) {
        return Err("IDENTIFY DEVICE no completó");
    }
    if (*port).is & 0x40000000 != 0 {
        return Err("IDENTIFY DEVICE reportó error de task file");
    }
    let data = buffer
        .as_slice(PAGE_SIZE)
        .map_err(|_| "buffer IDENTIFY inválido")?;
    let lba48 = u64::from_le_bytes(
        data[200..208]
            .try_into()
            .map_err(|_| "buffer IDENTIFY inválido")?,
    );
    if lba48 != 0 {
        return Ok(lba48);
    }
    let lba28 = u32::from_le_bytes(
        data[120..124]
            .try_into()
            .map_err(|_| "buffer IDENTIFY inválido")?,
    ) as u64;
    if lba28 != 0 {
        return Ok(lba28);
    }
    Err("IDENTIFY DEVICE reportó 0 sectores")
}
