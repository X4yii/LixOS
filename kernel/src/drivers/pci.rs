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

pub fn find_ahci() -> Option<(u8, u8, u8)> {
    for bus in 0..=255 {
        for slot in 0..32 {
            let vendor = pci_read_word(bus, slot, 0, 0);
            if vendor == 0xFFFF {
                continue;
            }
            
            for func in 0..8 {
                let vendor = pci_read_word(bus, slot, func, 0);
                if vendor == 0xFFFF {
                    continue;
                }
                
                let class_subclass = pci_read_word(bus, slot, func, 0x0A);
                let class = (class_subclass >> 8) as u8;
                let subclass = (class_subclass & 0xFF) as u8;
                
                if class == 0x01 && subclass == 0x06 {
                    let prog_if = (pci_read_word(bus, slot, func, 0x08) >> 8) as u8;
                    if prog_if == 0x01 {
                        crate::kprintln!("[pci] Encontrada Controladora AHCI SATA en {}:{}:{}", bus, slot, func);
                        return Some((bus, slot, func));
                    }
                }
            }
        }
    }
    None
}

pub fn init() {
    crate::kprintln!("[pci] Escaneando bus PCI en busca de dispositivos de almacenamiento...");
    if let Some((bus, slot, func)) = find_ahci() {
        crate::drivers::ahci::init(bus, slot, func);
    } else {
        crate::kprintln!("[pci] AVISO: No se encontró controladora AHCI en el sistema.");
    }
}