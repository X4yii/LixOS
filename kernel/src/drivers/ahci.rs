use core::ptr;
use crate::memory::vmm;
use crate::memory::pmm::PAGE_SIZE;

use crate::drivers::block::BlockDevice;

pub static mut PRIMARY_DISK: Option<AhciDisk> = None;

pub struct AhciDisk {
    pub port: *mut HbaPort,
}

unsafe impl Send for AhciDisk {}
unsafe impl Sync for AhciDisk {}

impl BlockDevice for AhciDisk {
    fn read_sectors(&self, start_sector: u64, count: u16, buf_phys: *mut u8) -> bool {
        // Our read_sector takes u32 for now, so we cast it. (Max 2TB support for now).
        unsafe { read_sector(self.port, start_sector as u32, count, buf_phys) }
    }
}

// Estructuras principales de AHCI (simplificadas)

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
    pub clb: u32,       // Command list base addr, 1K-byte aligned
    pub clbu: u32,      // Command list base addr upper 32 bits
    pub fb: u32,        // FIS base addr, 256-byte aligned
    pub fbu: u32,       // FIS base addr upper 32 bits
    pub is: u32,        // Interrupt status
    pub ie: u32,        // Interrupt enable
    pub cmd: u32,       // Command and status
    pub rsv0: u32,      // Reserved
    pub tfd: u32,       // Task file data
    pub sig: u32,       // Signature
    pub ssts: u32,      // SATA status (SCR0:SStatus)
    pub sctl: u32,      // SATA control (SCR2:SControl)
    pub serr: u32,      // SATA error (SCR1:SError)
    pub sact: u32,      // SATA active (SCR3:SActive)
    pub ci: u32,        // Command issue
    pub sntf: u32,      // SATA notification (SCR4:SNotification)
    pub fbs: u32,       // FIS-based switch control
    pub rsv1: [u32; 11], // Reserved
    pub vendor: [u32; 4], // Vendor specific
}

pub static mut ABAR: *mut HbaMem = ptr::null_mut();

pub fn init(bus: u8, slot: u8, func: u8) {
    crate::kprintln!("[ahci] Inicializando controladora en {}:{}:{}...", bus, slot, func);
    
    // Leer el BAR5 (offset 0x24) que contiene el ABAR (AHCI Base Memory Register)
    let bar5 = crate::drivers::pci::pci_read_dword(bus, slot, func, 0x24);
    if bar5 & 0x01 != 0 {
        crate::kprintln!("[ahci] AVISO: BAR5 no es Memory Mapped.");
        return;
    }
    
    let phys_addr = (bar5 & 0xFFFFFFF0) as u64;
    crate::kprintln!("[ahci] ABAR físico encontrado en: {:#x}", phys_addr);
    
    // El ABAR físico debe estar mapeado en nuestro HHDM (Higher Half Direct Map)
    let hhdm_offset = crate::HHDM_REQUEST.get_response().map(|r| r.offset()).unwrap_or(0xffff800000000000);
    let virt_addr = hhdm_offset + phys_addr;
    
    let cr3: u64;
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3) };
    let pml4_frame = (cr3 & 0x000FFFFFFFFFF000) as usize / PAGE_SIZE;
    
    let aligned_phys = phys_addr & !0xFFF;
    let aligned_virt = hhdm_offset + aligned_phys;
    
    unsafe {
        // Mapear 2 páginas por seguridad (8KB) para abarcar HbaMem
        crate::memory::vmm::map_page(pml4_frame, aligned_virt, aligned_phys, false, true, true).unwrap();
        crate::memory::vmm::map_page(pml4_frame, aligned_virt + 0x1000, aligned_phys + 0x1000, false, true, true).unwrap();

        ABAR = virt_addr as *mut HbaMem;
        
        let pi = (*ABAR).pi;
        crate::kprintln!("[ahci] Puertos implementados (bitmap): {:#b}", pi);
        
        // Buscar el primer puerto activo
        for i in 0..32 {
            if (pi & (1 << i)) != 0 {
                let ssts = (*ABAR).ports[i as usize].ssts;
                let det = ssts & 0x0F;
                let ipm = (ssts >> 8) & 0x0F;
                
                if det == 3 && ipm == 1 {
                    let sig = (*ABAR).ports[i as usize].sig;
                    if sig == 0x0101 { // SATA Drive
                        crate::kprintln!("[ahci] Disco SATA detectado en el Puerto {}", i);
                        let port = &mut (*ABAR).ports[i as usize] as *mut HbaPort;
                        port_rebase(port, i as usize);
                        PRIMARY_DISK = Some(AhciDisk { port });
                        crate::fs::ext4::init_from_primary_disk();
                    } else if sig == 0xEB140101 { // SATAPI (CD-ROM)
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

pub unsafe fn port_rebase(port: *mut HbaPort, port_no: usize) {
    // Detener el motor de comandos
    (*port).cmd &= !1; // bit 0 (ST)
    while (*port).cmd & 0x4000 != 0 { core::hint::spin_loop(); } // bit 14 (CR)
    
    (*port).cmd &= !0x10; // bit 4 (FRE)
    while (*port).cmd & 0x8000 != 0 { core::hint::spin_loop(); } // bit 15 (FR)
    
    // Asignar 1 frame (4KB) de DMA para las estructuras
    let phys_frame = crate::memory::pmm::alloc_frame().unwrap() * PAGE_SIZE;
    let hhdm_offset = crate::HHDM_REQUEST.get_response().map(|r| r.offset()).unwrap_or(0xffff800000000000);
    let virt_addr = hhdm_offset + phys_frame as u64;
    core::ptr::write_bytes(virt_addr as *mut u8, 0, 4096);
    
    // Clb (Command List Base): 1KB alineado -> Offset 0
    (*port).clb = phys_frame as u32;
    (*port).clbu = (phys_frame >> 32) as u32;
    
    // Fb (FIS Base): 256 bytes alineado -> Offset 1024
    (*port).fb = (phys_frame + 1024) as u32;
    (*port).fbu = ((phys_frame + 1024) >> 32) as u32;
    
    // Command Table Base para el Slot 0 -> Offset 1280 (alineado a 128 bytes)
    let cmd_header = virt_addr as *mut HbaCmdHeader;
    let ct_phys = phys_frame + 1280;
    (*cmd_header).ctba = ct_phys as u32;
    (*cmd_header).ctbau = (ct_phys >> 32) as u32;
    (*cmd_header).prdtl = 1;
    
    // Iniciar motor
    while (*port).cmd & 0x4000 != 0 { core::hint::spin_loop(); }
    (*port).cmd |= 0x10; // FRE
    (*port).cmd |= 1;    // ST
}

pub unsafe fn read_sector(port: *mut HbaPort, start_lba: u32, count: u16, buf_phys: *mut u8) -> bool {
    (*port).is = 0xFFFFFFFF; // Limpiar interrupciones
    
    let hhdm_offset = crate::HHDM_REQUEST.get_response().map(|r| r.offset()).unwrap_or(0xffff800000000000);
    let phys_clb = ((*port).clbu as u64) << 32 | (*port).clb as u64;
    let virt_clb = hhdm_offset + phys_clb;
    let cmd_header = virt_clb as *mut HbaCmdHeader;
    
    (*cmd_header).cfl = 5; // Tamaño en DWORDs de un FIS H2D (20 bytes)
    (*cmd_header).flags = 0; // Leer
    (*cmd_header).prdbc = 0;
    
    let phys_ctba = ((*cmd_header).ctbau as u64) << 32 | (*cmd_header).ctba as u64;
    let virt_ctba = hhdm_offset + phys_ctba;
    let cmd_table = virt_ctba as *mut HbaCmdTable;
    core::ptr::write_bytes(cmd_table as *mut u8, 0, core::mem::size_of::<HbaCmdTable>());
    
    (*cmd_table).prdt_entry[0].dba = buf_phys as u32;
    (*cmd_table).prdt_entry[0].dbau = (buf_phys as u64 >> 32) as u32;
    (*cmd_table).prdt_entry[0].dbc = (count as u32 * 512) - 1; // 0-indexed count
    
    let cfis = (*cmd_table).cfis.as_mut_ptr();
    *cfis.offset(0) = 0x27; // FIS_TYPE_REG_H2D
    *cfis.offset(1) = 0x80; // C (Command)
    *cfis.offset(2) = 0x24; // ATA_CMD_READ_EXT
    *cfis.offset(4) = (start_lba & 0xFF) as u8;
    *cfis.offset(5) = ((start_lba >> 8) & 0xFF) as u8;
    *cfis.offset(6) = ((start_lba >> 16) & 0xFF) as u8;
    *cfis.offset(7) = 0x40; // LBA mode
    *cfis.offset(8) = ((start_lba >> 24) & 0xFF) as u8;
    *cfis.offset(12) = (count & 0xFF) as u8;
    *cfis.offset(13) = ((count >> 8) & 0xFF) as u8;
    
    // Esperar a que el puerto esté libre
    while ((*port).tfd & (0x80 | 0x08)) != 0 { core::hint::spin_loop(); }
    
    (*port).ci = 1;
    
    let mut timeout = 0;
    loop {
        if ((*port).ci & 1) == 0 {
            break;
        }
        let tfd = (*port).tfd;
        let is = (*port).is;
        if is & 0x40000000 != 0 { // Task File Error
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
