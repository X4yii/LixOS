use crate::memory::address::{PhysAddr, hhdm_offset};

pub const MAX_CPUS: usize = 32;

#[repr(C, packed)]
pub struct AcpiHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32,
}

#[repr(C, packed)]
struct RsdpDescriptor {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_addr: u32,
}

#[repr(C, packed)]
struct XsdpDescriptor {
    rsdp: RsdpDescriptor,
    length: u32,
    xsdt_addr: u64,
    extended_checksum: u8,
    reserved: [u8; 3],
}

#[derive(Clone, Copy)]
pub struct CpuDescriptor {
    pub apic_id: u8,
    pub acpi_id: u8,
    pub enabled: bool,
}

impl CpuDescriptor {
    pub const fn empty() -> Self {
        Self { apic_id: 0, acpi_id: 0, enabled: false }
    }
}

pub struct AcpiTopology {
    pub lapic_phys_addr: u64,
    pub cpus: [CpuDescriptor; MAX_CPUS],
    pub cpu_count: usize,
}

impl AcpiTopology {
    pub const fn empty() -> Self {
        Self {
            lapic_phys_addr: 0xFEE0_0000,
            cpus: [CpuDescriptor::empty(); MAX_CPUS],
            cpu_count: 0,
        }
    }
}

unsafe fn current_pml4() -> usize {
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
    ((cr3 & 0x000F_FFFF_FFFF_F000) / 4096) as usize
}

unsafe fn map_acpi_table(phys_addr: u64, len: usize) -> Option<u64> {
    if phys_addr >= 0xFFFF_8000_0000_0000 {
        return Some(phys_addr);
    }
    let pml4 = current_pml4();
    crate::memory::vmm::map_mmio_region(pml4, PhysAddr::new(phys_addr), len)
        .ok()
        .map(|v| v.get())
}

pub unsafe fn parse_madt(rsdp_phys: u64) -> Option<AcpiTopology> {
    let rsdp_addr = if rsdp_phys != 0 {
        rsdp_phys
    } else {
        find_rsdp_in_bios()?
    };

    let rsdp_virt = map_acpi_table(rsdp_addr, core::mem::size_of::<XsdpDescriptor>())?;
    let sig_read = core::ptr::read_unaligned(rsdp_virt as *const [u8; 8]);
    if &sig_read != b"RSD PTR " {
        return None;
    }

    let revision = *((rsdp_virt + 15) as *const u8);
    let mut topology = AcpiTopology::empty();

    let madt_phys = if revision >= 2 {
        let xsdt_addr = core::ptr::read_unaligned((rsdp_virt + 24) as *const u64);
        find_table_xsdt(xsdt_addr, b"APIC")
    } else {
        let rsdt_addr = core::ptr::read_unaligned((rsdp_virt + 16) as *const u32);
        find_table_rsdt(rsdt_addr as u64, b"APIC")
    };

    let madt_phys = madt_phys?;
    let madt_virt = map_acpi_table(madt_phys, 0x2000)?;
    let length = core::ptr::read_unaligned((madt_virt + 4) as *const u32) as usize;

    topology.lapic_phys_addr = core::ptr::read_unaligned((madt_virt + 36) as *const u32) as u64;

    let mut offset = 44usize;
    while offset + 2 <= length && topology.cpu_count < MAX_CPUS {
        let entry_type = *((madt_virt + offset as u64) as *const u8);
        let entry_len = *((madt_virt + offset as u64 + 1) as *const u8) as usize;
        if entry_len < 2 {
            break;
        }

        if entry_type == 0 && entry_len >= 8 {
            let acpi_id = *((madt_virt + offset as u64 + 2) as *const u8);
            let apic_id = *((madt_virt + offset as u64 + 3) as *const u8);
            let flags = core::ptr::read_unaligned((madt_virt + offset as u64 + 4) as *const u32);
            let enabled = (flags & 1 != 0) || (flags & 2 != 0);

            if enabled {
                topology.cpus[topology.cpu_count] = CpuDescriptor {
                    apic_id,
                    acpi_id,
                    enabled: true,
                };
                topology.cpu_count += 1;
            }
        } else if entry_type == 5 && entry_len >= 12 {
            let override_addr = core::ptr::read_unaligned((madt_virt + offset as u64 + 4) as *const u64);
            topology.lapic_phys_addr = override_addr;
        }

        offset += entry_len;
    }

    Some(topology)
}

unsafe fn find_rsdp_in_bios() -> Option<u64> {
    let hhdm = hhdm_offset();
    for page_start in (0xE0000u64..=0xFFFF0u64).step_by(16) {
        let virt_opt = hhdm.map_phys(PhysAddr::new(page_start)).ok().map(|v| v.get())
            .or_else(|| map_acpi_table(page_start, 16));
        if let Some(virt) = virt_opt {
            let sig = core::ptr::read_unaligned(virt as *const [u8; 8]);
            if &sig == b"RSD PTR " {
                return Some(page_start);
            }
        }
    }
    None
}

unsafe fn find_table_rsdt(rsdt_phys: u64, sig: &[u8; 4]) -> Option<u64> {
    let rsdt_virt = map_acpi_table(rsdt_phys, 0x1000)?;
    let len = core::ptr::read_unaligned((rsdt_virt + 4) as *const u32) as usize;
    if len < 36 {
        return None;
    }
    let entries = (len - 36) / 4;
    for i in 0..entries {
        let entry_phys = core::ptr::read_unaligned(((rsdt_virt + 36) as *const u8).add(i * 4) as *const u32) as u64;
        if let Some(virt) = map_acpi_table(entry_phys, core::mem::size_of::<AcpiHeader>()) {
            let sig_read = core::ptr::read_unaligned(virt as *const [u8; 4]);
            if &sig_read == sig {
                return Some(entry_phys);
            }
        }
    }
    None
}

unsafe fn find_table_xsdt(xsdt_phys: u64, sig: &[u8; 4]) -> Option<u64> {
    let xsdt_virt = map_acpi_table(xsdt_phys, 0x1000)?;
    let len = core::ptr::read_unaligned((xsdt_virt + 4) as *const u32) as usize;
    if len < 36 {
        return None;
    }
    let entries = (len - 36) / 8;
    for i in 0..entries {
        let entry_phys = core::ptr::read_unaligned(((xsdt_virt + 36) as *const u8).add(i * 8) as *const u64);
        if let Some(virt) = map_acpi_table(entry_phys, core::mem::size_of::<AcpiHeader>()) {
            let sig_read = core::ptr::read_unaligned(virt as *const [u8; 4]);
            if &sig_read == sig {
                return Some(entry_phys);
            }
        }
    }
    None
}
