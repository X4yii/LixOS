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
    let shift = ((offset & 2) * 8) as u32;
    let current = pci_read_dword(bus, slot, func, offset);
    let updated = (current & !(0xFFFFu32 << shift)) | ((value as u32) << shift);
    pci_write_dword(bus, slot, func, offset, updated);
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
    let address = ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | (offset as u32 & 0xFC)
        | 0x80000000;
    unsafe {
        out32(PCI_CONFIG_ADDRESS, address);
        out32(PCI_CONFIG_DATA, value);
    }
}
pub fn enable_memory_and_bus_master(bus: u8, slot: u8, func: u8) {
    let command = pci_read_word(bus, slot, func, 0x04) | (1 << 1) | (1 << 2);
    pci_write_word(bus, slot, func, 0x04, command);
}
pub fn enable_io_and_bus_master(bus: u8, slot: u8, func: u8) {
    let command = pci_read_word(bus, slot, func, 0x04) | (1 << 0) | (1 << 2);
    pci_write_word(bus, slot, func, 0x04, command);
}
pub fn init() {
    crate::kprintln!("[pci] Escaneando bus PCI en busca de dispositivos...");
    let mut nvme_found = false;
    let mut ahci_found = false;
    let mut xhci_found = false;
    let mut ethernet_found = false;
    for bus in 0..=255 {
        for slot in 0..32 {
            let vendor = pci_read_word(bus, slot, 0, 0);
            if vendor == 0xFFFF {
                continue;
            }
            let header = (pci_read_dword(bus, slot, 0, 0x0C) >> 16) as u8;
            let functions = if header & 0x80 != 0 { 8 } else { 1 };
            for func in 0..functions {
                let vendor = pci_read_word(bus, slot, func, 0);
                if vendor == 0xFFFF {
                    continue;
                }
                let class_subclass = pci_read_word(bus, slot, func, 0x0A);
                let class = (class_subclass >> 8) as u8;
                let subclass = (class_subclass & 0xFF) as u8;
                let prog_if = (pci_read_word(bus, slot, func, 0x08) >> 8) as u8;
                if class == 0x0c && subclass == 0x03 && prog_if == 0x30 {
                    xhci_found = true;
                    crate::kprintln!(
                        "[pci] Controladora USB xHCI {}:{}:{} (vendor={:#06x}, device={:#06x})",
                        bus,
                        slot,
                        func,
                        vendor,
                        pci_read_word(bus, slot, func, 2)
                    );
                    match crate::drivers::xhci::init(bus, slot, func) {
                        Ok(()) => {}
                        Err(error) => crate::kprintln!("[xhci] No se pudo iniciar: {}", error),
                    }
                }
                if class == 0x02 {
                    ethernet_found = true;
                    let device = pci_read_word(bus, slot, func, 2);
                    if vendor == 0x1af4 && device == 0x1000 {
                        crate::kprintln!("[pci] Virtio-net legacy en {}:{}:{}", bus, slot, func);
                        match crate::drivers::virtio_net::init(bus, slot, func) {
                            Ok(()) => {}
                            Err(error) => {
                                crate::kprintln!("[virtio-net] No se pudo iniciar: {}", error)
                            }
                        }
                    } else if vendor == 0x8086 && device == 0xa0f0 {
                        crate::kprintln!(
                            "[pci] Intel Wi-Fi 6 AX201 detectado en {}:{}:{}; requiere driver CNVio2 y firmware (Wi-Fi aún no disponible)",
                            bus,
                            slot,
                            func
                        );
                    } else {
                        crate::kprintln!("[pci] Interfaz de red {}:{}:{} (vendor={:#06x}, device={:#06x}); driver pendiente", bus, slot, func, vendor, device);
                    }
                }
                if class == 0x01 {
                    if subclass == 0x08 && prog_if == 0x02 {
                        crate::kprintln!(
                            "[pci] Controladora NVMe PCIe en {}:{}:{}",
                            bus,
                            slot,
                            func
                        );
                        enable_memory_and_bus_master(bus, slot, func);
                        crate::drivers::nvme::init(bus, slot, func);
                        nvme_found = true;
                    } else if subclass == 0x06 && prog_if == 0x01 {
                        crate::kprintln!(
                            "[pci] Controladora AHCI SATA en {}:{}:{}",
                            bus,
                            slot,
                            func
                        );
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
    if !xhci_found {
        crate::kprintln!("[pci] No se encontraron controladoras xHCI.");
    }
    if !ethernet_found {
        crate::kprintln!("[pci] No se encontraron interfaces Ethernet PCI.");
    }
}
pub fn enable_msi(bus: u8, slot: u8, func: u8, vector: u8, lapic_id: u32) -> bool {
    let status = pci_read_word(bus, slot, func, 0x06);
    if (status & (1 << 4)) == 0 {
        return false;
    }
    let mut cap_ptr = (pci_read_word(bus, slot, func, 0x34) & 0xFF) as u8;
    while cap_ptr != 0 {
        let cap_id = (pci_read_word(bus, slot, func, cap_ptr) & 0xFF) as u8;
        let next_ptr = ((pci_read_word(bus, slot, func, cap_ptr) >> 8) & 0xFF) as u8;
        if cap_id == 0x05 {
            let msg_ctrl = pci_read_word(bus, slot, func, cap_ptr + 2);
            let is_64bit = (msg_ctrl & (1 << 7)) != 0;
            let msg_addr = 0xFEE0_0000u32 | (lapic_id << 12);
            pci_write_dword(bus, slot, func, cap_ptr + 4, msg_addr);
            let data_offset = if is_64bit {
                pci_write_dword(bus, slot, func, cap_ptr + 8, 0);
                cap_ptr + 12
            } else {
                cap_ptr + 8
            };
            pci_write_word(bus, slot, func, data_offset, vector as u16);
            pci_write_word(bus, slot, func, cap_ptr + 2, msg_ctrl | 1);
            return true;
        }
        cap_ptr = next_ptr;
    }
    false
}
