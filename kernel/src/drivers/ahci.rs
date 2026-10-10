use crate::memory::pmm::PAGE_SIZE;
use crate::sync::IrqSafeSpinlock;
use crate::drivers::block::{BlockDevice, BlockError, BlockGeometry, DmaBuffer};
use core::sync::atomic::{AtomicBool, Ordering};

pub static PRIMARY_DISK: IrqSafeSpinlock<Option<AhciDisk>> = IrqSafeSpinlock::new(None);
static DISK_IO_LOCK: AtomicBool = AtomicBool::new(false);

struct DiskIoGuard;

impl DiskIoGuard {
    fn acquire() -> Self {
        while DISK_IO_LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        Self
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
    fn read_blocks(&self, start: u64, count: u16, buffer: &mut DmaBuffer) -> Result<(), BlockError> {
        let disk = PRIMARY_DISK.lock().as_ref().copied().ok_or(BlockError::NoDevice)?;
        disk.read_blocks(start, count, buffer)
    }
    fn write_blocks(&self, _start: u64, _count: u16, _buffer: &DmaBuffer) -> Result<(), BlockError> {
        Err(BlockError::Unsupported)
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
        if self.block_count == 0 { return Err(BlockError::InvalidGeometry); }
        Ok(BlockGeometry { block_count: self.block_count, block_size: 512 })
    }
    fn read_blocks(&self, start: u64, count: u16, buffer: &mut DmaBuffer) -> Result<(), BlockError> {
        let geometry = self.geometry()?;
        let end = start.checked_add(count as u64).ok_or(BlockError::InvalidRange)?;
        if count == 0 || end > geometry.block_count || count as usize > PAGE_SIZE / 512 {
            return Err(BlockError::InvalidRange);
        }
        let lba = u32::try_from(start).map_err(|_| BlockError::Unsupported)?;
        let _io_guard = DiskIoGuard::acquire();
        let ok = unsafe { read_sector(self.port, lba, count, buffer.physical_address().get() as *mut u8) };
        if ok { Ok(()) } else { Err(BlockError::DeviceFault) }
    }
    fn write_blocks(&self, _start: u64, _count: u16, _buffer: &DmaBuffer) -> Result<(), BlockError> {
        Err(BlockError::Unsupported)
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
    crate::kprintln!("[ahci] Inicializando controladora en {}:{}:{}...", bus, slot, func);
    
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
                        if !port_rebase(port, i as usize) { continue; }
                        *PRIMARY_DISK.lock() = Some(AhciDisk { port, block_count: 0 });
                        crate::fs::ext4::init_from_disk(&GLOBAL_DISK);
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
    if timeout == 0 { crate::kprintln!("[ahci] Timeout deteniendo motor de comandos"); return false; }
    
    (*port).cmd &= !0x10;
    let mut timeout = 1_000_000;
    while (*port).cmd & 0x8000 != 0 && timeout > 0 {
        core::hint::spin_loop();
        timeout -= 1;
    }
    if timeout == 0 { crate::kprintln!("[ahci] Timeout deteniendo recepción FIS"); return false; }
    
    let Some(frame) = crate::memory::pmm::alloc_frame() else {
        crate::kprintln!("[ahci] Sin memoria para estructuras DMA");
        return false;
    };
    let phys_frame = frame * PAGE_SIZE;
    let hhdm_offset = crate::memory::address::hhdm_offset();
    let virt_addr = hhdm_offset.map_phys(crate::memory::address::PhysAddr::new(phys_frame as u64))
        .unwrap_or_else(|error| panic!("invalid AHCI DMA frame address: {error}"))
        .get();
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

pub unsafe fn read_sector(port: *mut HbaPort, start_lba: u32, count: u16, buf_phys: *mut u8) -> bool {
    (*port).is = 0xFFFFFFFF;
    
    let phys_clb = ((*port).clbu as u64) << 32 | (*port).clb as u64;
    let hhdm_offset = crate::memory::address::hhdm_offset();
    let virt_clb = hhdm_offset.map_phys(crate::memory::address::PhysAddr::new(phys_clb))
        .unwrap_or_else(|error| panic!("invalid AHCI command-list address: {error}"))
        .get();
    let cmd_header = virt_clb as *mut HbaCmdHeader;
    
    (*cmd_header).cfl = 5;
    (*cmd_header).flags = 0;
    (*cmd_header).prdbc = 0;
    
    let phys_ctba = ((*cmd_header).ctbau as u64) << 32 | (*cmd_header).ctba as u64;
    let virt_ctba = hhdm_offset.map_phys(crate::memory::address::PhysAddr::new(phys_ctba))
        .unwrap_or_else(|error| panic!("invalid AHCI command-table address: {error}"))
        .get();
    let cmd_table = virt_ctba as *mut HbaCmdTable;
    core::ptr::write_bytes(cmd_table as *mut u8, 0, core::mem::size_of::<HbaCmdTable>());
    
    (*cmd_table).prdt_entry[0].dba = buf_phys as u32;
    (*cmd_table).prdt_entry[0].dbau = (buf_phys as u64 >> 32) as u32;
    (*cmd_table).prdt_entry[0].dbc = (count as u32 * 512) - 1;
    
    let cfis = (*cmd_table).cfis.as_mut_ptr();
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
    
    let mut tfd_timeout = 5_000_000;
    while ((*port).tfd & (0x80 | 0x08)) != 0 && tfd_timeout > 0 {
        core::hint::spin_loop();
        tfd_timeout -= 1;
    }
    if tfd_timeout == 0 {
        crate::kprintln!("[ahci] Advertencia: timeout esperando puerto libre (BSY/DRQ)");
        return false;
    }
    
    (*port).ci = 1;
    
    let mut timeout = 0;
    loop {
        if ((*port).ci & 1) == 0 {
            break;
        }
        let tfd = (*port).tfd;
        let is = (*port).is;
        if is & 0x40000000 != 0 {
            crate::kprintln!("[ahci] Task File Error! tfd={:#x}, is={:#x}", tfd, is);
            return false;
        }
        timeout += 1;
        if timeout > 10000000 {
            let ci = (*port).ci;
            let ssts = (*port).ssts;
            crate::kprintln!("[ahci] TIMEOUT! ci={:#x}, tfd={:#x}, is={:#x}, ssts={:#x}", ci, tfd, is, ssts);
            return false;
        }
        core::hint::spin_loop();
    }
    
    if (*port).is & 0x40000000 != 0 {
        return false;
    }
    
    true
}
