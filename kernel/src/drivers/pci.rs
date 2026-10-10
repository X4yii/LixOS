use core::arch::asm;

const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

pub unsafe fn out32(port: u16, value: u32) {
    asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
}

pub unsafe fn in32(port: u16) -> u32 {
    let mut value: u32;
    asm!("in eax, dx", out("eax") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

pub fn pci_read_word(bus: u8, slot: u8, func: u8, offset: u8) -> u16 {
    let address = ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | (offset as u32 & 0xFC)
        | 0x80000000;
        
    unsafe {
        out32(PCI_CONFIG_ADDRESS, address);
        let val = in32(PCI_CONFIG_DATA);
        ((val >> ((offset & 2) * 8)) & 0xFFFF) as u16
    }
}

pub fn pci_write_word(bus: u8, slot: u8, func: u8, offset: u8, value: u16) {
    let address = ((bus as u32) << 16) | ((slot as u32) << 11)
        | ((func as u32) << 8) | (offset as u32 & 0xFC) | 0x80000000;
    let shift = ((offset & 2) * 8) as u32;
    unsafe { out32(PCI_CONFIG_ADDRESS, address); out32(PCI_CONFIG_DATA, (value as u32) << shift); }
}

pub fn pci_read_dword(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let address = ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | (offset as u32 & 0xFC)
        | 0x80000000;
        
    unsafe {
        out32(PCI_CONFIG_ADDRESS, address);
        in32(PCI_CONFIG_DATA)
    }
}

pub fn pci_write_dword(bus: u8, slot: u8, func: u8, offset: u8, value: u32) {
    let address = ((bus as u32) << 16) | ((slot as u32) << 11)
        | ((func as u32) << 8) | (offset as u32 & 0xFC) | 0x80000000;
    unsafe { out32(PCI_CONFIG_ADDRESS, address); out32(PCI_CONFIG_DATA, value); }
}

/// Enable PCI memory decoding and bus mastering before a driver touches BARs or DMA.
pub fn enable_memory_and_bus_master(bus: u8, slot: u8, func: u8) {
    let command = pci_read_word(bus, slot, func, 0x04) | (1 << 1) | (1 << 2);
    pci_write_word(bus, slot, func, 0x04, command);
}

pub fn init() {
    crate::kprintln!("[pci] Escaneando bus PCI en busca de dispositivos de almacenamiento...");
    let mut nvme_found = false;
    let mut ahci_found = false;

    for bus in 0..=255 {
        for slot in 0..32 {
            let vendor = pci_read_word(bus, slot, 0, 0);
            if vendor == 0xFFFF { continue; }
            let header = (pci_read_dword(bus, slot, 0, 0x0C) >> 16) as u8;
            let functions = if header & 0x80 != 0 { 8 } else { 1 };
            for func in 0..functions {
                let vendor = pci_read_word(bus, slot, func, 0);
                if vendor == 0xFFFF { continue; }
                let class_subclass = pci_read_word(bus, slot, func, 0x0A);
                let class = (class_subclass >> 8) as u8;
                let subclass = (class_subclass & 0xFF) as u8;
                let prog_if = (pci_read_word(bus, slot, func, 0x08) >> 8) as u8;

                if class == 0x01 {
                    if subclass == 0x08 && prog_if == 0x02 {
                        crate::kprintln!("[pci] Controladora NVMe PCIe en {}:{}:{}", bus, slot, func);
                        enable_memory_and_bus_master(bus, slot, func);
                        crate::drivers::nvme::init(bus, slot, func);
                        nvme_found = true;
                    } else if subclass == 0x06 && prog_if == 0x01 {
                        crate::kprintln!("[pci] Controladora AHCI SATA en {}:{}:{}", bus, slot, func);
                        enable_memory_and_bus_master(bus, slot, func);
                        crate::drivers::ahci::init(bus, slot, func);
                        ahci_found = true;
                    }
                }
            }
        }
    }

    if !nvme_found && !ahci_found {
        crate::kprintln!("[pci] AVISO: No se encontraron controladoras de almacenamiento.");
    }
}
