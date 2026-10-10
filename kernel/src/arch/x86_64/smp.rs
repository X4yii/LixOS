use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use crate::arch::x86_64::{acpi::AcpiTopology, apic, percpu};
use crate::memory::address::{PhysAddr, hhdm_offset};

pub static ONLINE_CPUS: AtomicU32 = AtomicU32::new(1);
static AP_STARTUP_ACK: AtomicBool = AtomicBool::new(false);

const KERNEL_STACK_SIZE: usize = 256 * 1024;
const TRAMPOLINE_PHYS_ADDR: u64 = 0x8000;

static TRAMPOLINE_TEMPLATE: [u8; 256] = {
    let mut buf = [0u8; 256];

    buf[0x00] = 0xfa;
    buf[0x01] = 0x31; buf[0x02] = 0xc0;
    buf[0x03] = 0x8e; buf[0x04] = 0xd8;
    buf[0x05] = 0x8e; buf[0x06] = 0xd0;
    buf[0x07] = 0xbc; buf[0x08] = 0x00; buf[0x09] = 0x7c;
    buf[0x0a] = 0x0f; buf[0x0b] = 0x01; buf[0x0c] = 0x16; buf[0x0d] = 0x40; buf[0x0e] = 0x80;
    buf[0x0f] = 0x0f; buf[0x10] = 0x20; buf[0x11] = 0xc0;
    buf[0x12] = 0x0c; buf[0x13] = 0x01;
    buf[0x14] = 0x0f; buf[0x15] = 0x22; buf[0x16] = 0xc0;
    buf[0x17] = 0x66; buf[0x18] = 0xea; buf[0x19] = 0x70; buf[0x1a] = 0x80; buf[0x1b] = 0x00; buf[0x1c] = 0x00; buf[0x1d] = 0x08; buf[0x1e] = 0x00;

    buf[0x40] = 31; buf[0x41] = 0;
    buf[0x42] = 0x46; buf[0x43] = 0x80; buf[0x44] = 0x00; buf[0x45] = 0x00;

    buf[0x4e] = 0xff; buf[0x4f] = 0xff; buf[0x50] = 0x00; buf[0x51] = 0x00;
    buf[0x52] = 0x00; buf[0x53] = 0x9a; buf[0x54] = 0xcf; buf[0x55] = 0x00;

    buf[0x56] = 0xff; buf[0x57] = 0xff; buf[0x58] = 0x00; buf[0x59] = 0x00;
    buf[0x5a] = 0x00; buf[0x5b] = 0x92; buf[0x5c] = 0xcf; buf[0x5d] = 0x00;

    buf[0x5e] = 0xff; buf[0x5f] = 0xff; buf[0x60] = 0x00; buf[0x61] = 0x00;
    buf[0x62] = 0x00; buf[0x63] = 0x9a; buf[0x64] = 0xaf; buf[0x65] = 0x00;

    buf[0x70] = 0x66; buf[0x71] = 0xb8; buf[0x72] = 0x10; buf[0x73] = 0x00;
    buf[0x74] = 0x8e; buf[0x75] = 0xd8;
    buf[0x76] = 0x8e; buf[0x77] = 0xc0;
    buf[0x78] = 0x8e; buf[0x79] = 0xd0;
    buf[0x7a] = 0x0f; buf[0x7b] = 0x20; buf[0x7c] = 0xe0;
    buf[0x7d] = 0x83; buf[0x7e] = 0xc8; buf[0x7f] = 0x20;
    buf[0x80] = 0x0f; buf[0x81] = 0x22; buf[0x82] = 0xe0;
    buf[0x83] = 0xa1; buf[0x84] = 0xe0; buf[0x85] = 0x80; buf[0x86] = 0x00; buf[0x87] = 0x00;
    buf[0x88] = 0x0f; buf[0x89] = 0x22; buf[0x8a] = 0xd8;
    buf[0x8b] = 0xb9; buf[0x8c] = 0x80; buf[0x8d] = 0x00; buf[0x8e] = 0x00; buf[0x8f] = 0xc0;
    buf[0x90] = 0x0f; buf[0x91] = 0x32;
    buf[0x92] = 0x0d; buf[0x93] = 0x00; buf[0x94] = 0x09; buf[0x95] = 0x00; buf[0x96] = 0x00;
    buf[0x97] = 0x0f; buf[0x98] = 0x30;
    buf[0x99] = 0x0f; buf[0x9a] = 0x20; buf[0x9b] = 0xc0;
    buf[0x9c] = 0x0d; buf[0x9d] = 0x00; buf[0x9e] = 0x00; buf[0x9f] = 0x00; buf[0xa0] = 0x80;
    buf[0xa1] = 0x0f; buf[0xa2] = 0x22; buf[0xa3] = 0xc0;
    buf[0xa4] = 0xea; buf[0xa5] = 0xb0; buf[0xa6] = 0x80; buf[0xa7] = 0x00; buf[0xa8] = 0x00; buf[0xa9] = 0x18; buf[0xaa] = 0x00;

    buf[0xb0] = 0x66; buf[0xb1] = 0xb8; buf[0xb2] = 0x10; buf[0xb3] = 0x00;
    buf[0xb4] = 0x8e; buf[0xb5] = 0xd8;
    buf[0xb6] = 0x8e; buf[0xb7] = 0xc0;
    buf[0xb8] = 0x8e; buf[0xb9] = 0xd0;
    buf[0xba] = 0x48; buf[0xbb] = 0x8b; buf[0xbc] = 0x24; buf[0xbd] = 0x25; buf[0xbe] = 0xe8; buf[0xbf] = 0x80; buf[0xc0] = 0x00; buf[0xc1] = 0x00;
    buf[0xc2] = 0x48; buf[0xc3] = 0x8b; buf[0xc4] = 0x04; buf[0xc5] = 0x25; buf[0xc6] = 0xf0; buf[0xc7] = 0x80; buf[0xc8] = 0x00; buf[0xc9] = 0x00;
    buf[0xca] = 0x48; buf[0xcb] = 0x8b; buf[0xcc] = 0x3c; buf[0xcd] = 0x25; buf[0xce] = 0xf8; buf[0xcf] = 0x80; buf[0xd0] = 0x00; buf[0xd1] = 0x00;
    buf[0xd2] = 0xff; buf[0xd3] = 0xe0;

    buf
};

#[no_mangle]
pub extern "C" fn ap_startup_entry(cpu_id: u64) -> ! {
    AP_STARTUP_ACK.store(true, Ordering::Release);
    let id = cpu_id as usize;
    unsafe {
        let mut cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0);
        cr0 &= !(1 << 2);
        cr0 |= (1 << 1) | (1 << 5);
        core::arch::asm!("mov cr0, {}", in(reg) cr0);

        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4);
        cr4 |= (1 << 9) | (1 << 10);
        core::arch::asm!("mov cr4, {}", in(reg) cr4);

        let percpu_ptr = percpu::percpu_ptr(id);
        x86_64::registers::model_specific::Msr::new(0xC000_0101).write(percpu_ptr as u64);

        let area = percpu::percpu_mut(id);
        area.self_ptr = percpu_ptr as u64;
        area.cpu_id = id as u32;

        crate::arch::x86_64::gdt::init_ap_gdt(id);
        crate::arch::x86_64::idt::load_idt();
        crate::task::syscall::init_syscall_msrs_for_cpu(id);
        apic::init_ap();
    }

    ONLINE_CPUS.fetch_add(1, Ordering::SeqCst);

    x86_64::instructions::interrupts::enable();
    crate::arch::x86_64::halt_loop();
}

pub unsafe fn boot_secondary_cores(topology: &AcpiTopology) {
    if topology.cpu_count <= 1 {
        crate::kprintln!("[smp] Sistema mononucleo detectado (1 CPU)");
        return;
    }

    crate::kprintln!("[smp] Preparando bootstrap nativo para {} CPUs...", topology.cpu_count);

    let hhdm = hhdm_offset();
    let tramp_virt = match hhdm.map_phys(PhysAddr::new(TRAMPOLINE_PHYS_ADDR)) {
        Ok(v) => v.get() as *mut u8,
        Err(_) => {
            crate::kprintln!("[smp] Error: No se pudo mapear direccion fisica 0x8000 para trampolin");
            return;
        }
    };

    core::ptr::copy_nonoverlapping(TRAMPOLINE_TEMPLATE.as_ptr(), tramp_virt, 256);

    let current_cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) current_cr3);
    let pml4_phys = current_cr3 & 0x000F_FFFF_FFFF_F000;
    let pml4_virt = hhdm.map_phys(PhysAddr::new(pml4_phys)).unwrap().get() as *mut u64;
    let saved_pml4_0 = *pml4_virt;
    *pml4_virt = *pml4_virt.add(256);

    let bsp_apic_id = percpu::bsp_percpu_mut().lapic_id as u8;
    let mut ap_index = 1usize;
    for i in 0..topology.cpu_count {
        let cpu = &topology.cpus[i];
        if !cpu.enabled || cpu.apic_id == bsp_apic_id {
            continue;
        }

        if ap_index >= percpu::MAX_CPUS {
            break;
        }

        let stack_layout = match core::alloc::Layout::from_size_align(KERNEL_STACK_SIZE, 16) {
            Ok(l) => l,
            Err(_) => break,
        };
        let stack_ptr = alloc::alloc::alloc(stack_layout) as u64;
        if stack_ptr == 0 {
            break;
        }
        let stack_top = stack_ptr + KERNEL_STACK_SIZE as u64;

        let ap_area = percpu::percpu_mut(ap_index);
        ap_area.lapic_id = cpu.apic_id as u32;
        ap_area.cpu_id = ap_index as u32;
        ap_area.kernel_rsp = stack_top;

        let data_slice = core::slice::from_raw_parts_mut(tramp_virt.add(0xe0) as *mut u64, 4);
        data_slice[0] = current_cr3;
        data_slice[1] = stack_top;
        data_slice[2] = ap_startup_entry as *const () as u64;
        data_slice[3] = ap_index as u64;

        AP_STARTUP_ACK.store(false, Ordering::Release);

        apic::send_init_ipi(cpu.apic_id);
        delay_approx_ms(10);

        apic::send_startup_ipi(cpu.apic_id, 0x08);
        delay_approx_us(200);

        if !AP_STARTUP_ACK.load(Ordering::Acquire) {
            apic::send_startup_ipi(cpu.apic_id, 0x08);
        }

        let mut wait_count = 50_000usize;
        while !AP_STARTUP_ACK.load(Ordering::Acquire) && wait_count > 0 {
            core::hint::spin_loop();
            wait_count -= 1;
        }

        if AP_STARTUP_ACK.load(Ordering::Acquire) {
            crate::task::scheduler::spawn_idle_for_cpu(ap_index);
            crate::kprintln!("[smp] CPU {} (APIC ID {}) iniciada exitosamente", ap_index, cpu.apic_id);
        } else {
            crate::kprintln!("[smp] AVISO: CPU {} (APIC ID {}) no respondio a SIPI", ap_index, cpu.apic_id);
        }

        ap_index += 1;
    }

    *pml4_virt = saved_pml4_0;
    core::arch::asm!("invlpg [0]");

    crate::kprintln!("[smp] Total de CPUs: {}", ONLINE_CPUS.load(Ordering::SeqCst));
}

fn delay_approx_us(us: usize) {
    for _ in 0..(us * 10) {
        core::hint::spin_loop();
    }
}

fn delay_approx_ms(ms: usize) {
    for _ in 0..(ms * 5_000) {
        core::hint::spin_loop();
    }
}

