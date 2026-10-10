//! Minimal polling NVMe driver for a single standard PCIe namespace.
//! MSI/MSI-X, multiple namespaces, PRP lists and power management are deferred.
use crate::drivers::block::{BlockDevice, BlockError, BlockGeometry, DmaBuffer};
use crate::memory::pmm::PAGE_SIZE;
use crate::sync::IrqSafeSpinlock;

const QUEUE_DEPTH: u16 = 16;
const ADMIN_TIMEOUT: usize = 20_000_000;
const IO_TIMEOUT: usize = 20_000_000;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Submission {
    opcode: u8, flags: u8, cid: u16, nsid: u32, reserved: [u32; 2],
    metadata: u64, prp1: u64, prp2: u64, cdw: [u32; 6],
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Completion { result: u32, reserved: u32, sq_head: u16, sq_id: u16, cid: u16, status: u16 }

struct Queue {
    sq: DmaBuffer,
    cq: DmaBuffer,
    sq_tail: u16,
    cq_head: u16,
    phase: u16,
    next_cid: u16,
    qid: u16,
    doorbell_sq: *mut u32,
    doorbell_cq: *mut u32,
}
unsafe impl Send for Queue {}

struct NvmeController {
    mmio: *mut u8,
    namespace: u32,
    block_size: u32,
    block_count: u64,
    _admin: Queue,
    io: Queue,
}
unsafe impl Send for NvmeController {}

static CONTROLLER: IrqSafeSpinlock<Option<NvmeController>> = IrqSafeSpinlock::new(None);
pub struct NvmeDisk;
pub static DISK: NvmeDisk = NvmeDisk;

impl Queue {
    fn new(qid: u16, doorbells: *mut u8, dstrd: u32) -> Result<Self, BlockError> {
        let sq = DmaBuffer::allocate()?;
        let cq = DmaBuffer::allocate()?;
        let stride = 4usize.checked_shl(dstrd).ok_or(BlockError::InvalidGeometry)?;
        let sq_off = 0x1000usize.checked_add((2 * qid as usize) * stride).ok_or(BlockError::InvalidGeometry)?;
        let cq_off = 0x1000usize.checked_add((2 * qid as usize + 1) * stride).ok_or(BlockError::InvalidGeometry)?;
        Ok(Self { sq, cq, sq_tail: 0, cq_head: 0, phase: 1, next_cid: 1, qid,
            doorbell_sq: unsafe { doorbells.add(sq_off) as *mut u32 },
            doorbell_cq: unsafe { doorbells.add(cq_off) as *mut u32 } })
    }

    unsafe fn submit(&mut self, command: Submission, timeout: usize) -> Result<Completion, BlockError> {
        let expected_cid = command.cid;
        let sq = self.sq.as_mut_slice(PAGE_SIZE)? as *mut [u8];
        let entry = sq.cast::<Submission>().add(self.sq_tail as usize);
        core::ptr::write_volatile(entry, command);
        self.sq_tail = (self.sq_tail + 1) % QUEUE_DEPTH;
        core::ptr::write_volatile(self.doorbell_sq, self.sq_tail as u32);
        let cq = self.cq.as_slice(PAGE_SIZE)?.as_ptr().cast::<Completion>();
        for _ in 0..timeout {
            let completion = core::ptr::read_volatile(cq.add(self.cq_head as usize));
            if completion.status & 1 == self.phase {
                self.cq_head = (self.cq_head + 1) % QUEUE_DEPTH;
                if self.cq_head == 0 { self.phase ^= 1; }
                core::ptr::write_volatile(self.doorbell_cq, self.cq_head as u32);
                if completion.cid != expected_cid || completion.sq_id != self.qid {
                    return Err(BlockError::DeviceFault);
                }
                if (completion.status >> 1) & 0x7FF != 0 { return Err(BlockError::DeviceFault); }
                return Ok(completion);
            }
            core::hint::spin_loop();
        }
        Err(BlockError::Timeout)
    }

}

unsafe fn mmio_read32(base: *mut u8, offset: usize) -> u32 { core::ptr::read_volatile(base.add(offset).cast()) }
unsafe fn mmio_write32(base: *mut u8, offset: usize, value: u32) { core::ptr::write_volatile(base.add(offset).cast(), value); }
unsafe fn mmio_read64(base: *mut u8, offset: usize) -> u64 { core::ptr::read_volatile(base.add(offset).cast()) }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RootSelection {
    Automatic,
    InitrdOnly,
    PartitionGuid([u8; 16]),
}

fn parse_root_selection(command_line: Option<&str>) -> Result<RootSelection, ()> {
    let mut requested = None;
    if let Some(command_line) = command_line {
        for token in command_line.split_ascii_whitespace() {
            let selection = if token == "root=none" {
                Some(RootSelection::InitrdOnly)
            } else if let Some(value) = token.strip_prefix("root=PARTUUID=") {
                Some(RootSelection::PartitionGuid(
                    crate::drivers::gpt::parse_guid(value).ok_or(())?,
                ))
            } else if token.starts_with("root=") {
                return Err(());
            } else {
                None
            };
            if let Some(selection) = selection {
                if requested.replace(selection).is_some() {
                    return Err(());
                }
            }
        }
    }
    Ok(requested.unwrap_or(RootSelection::Automatic))
}

pub fn init(bus: u8, slot: u8, func: u8) {
    let bar_low = crate::drivers::pci::pci_read_dword(bus, slot, func, 0x10);
    let bar_high = crate::drivers::pci::pci_read_dword(bus, slot, func, 0x14);
    if bar_low & 1 != 0 || (bar_low >> 1) & 3 != 2 {
        crate::kprintln!("[nvme] BAR0 no es MMIO de 64 bits; controlador omitido"); return;
    }
    let physical = ((bar_high as u64) << 32) | (bar_low as u64 & !0xF);
    let cr3: u64;
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3) };
    let pml4 = (cr3 & 0x000F_FFFF_FFFF_F000) as usize / PAGE_SIZE;
    let Some(mut admin_sq) = DmaBuffer::allocate().ok() else { return; };
    let Some(admin_cq) = DmaBuffer::allocate().ok() else { return; };
    let _ = &mut admin_sq;
    let Some(mmio_len) = (|| unsafe {
        let mmio = crate::memory::vmm::map_mmio_region(pml4, crate::memory::address::PhysAddr::new(physical), 0x4000).ok()?.get() as *mut u8;
        let cap = mmio_read64(mmio, 0);
        let max_entries = ((cap & 0xFFFF) + 1) as u16;
        if max_entries < QUEUE_DEPTH { return None; }
        let dstrd = ((cap >> 32) & 0xF) as u32;
        if dstrd > 2 || (cap >> 48) & 0xF != 0 { return None; }
        let timeout = (((cap >> 24) & 0xFF).max(1) as usize).saturating_mul(500_000);
        let cc = mmio_read32(mmio, 0x14);
        mmio_write32(mmio, 0x14, cc & !1);
        let mut ready = false;
        for _ in 0..timeout { if mmio_read32(mmio, 0x1C) & 1 == 0 { ready = true; break; } core::hint::spin_loop(); }
        if !ready { return None; }
        mmio_write32(mmio, 0x24, ((QUEUE_DEPTH - 1) as u32) << 16 | (QUEUE_DEPTH - 1) as u32);
        core::ptr::write_volatile(mmio.add(0x28).cast::<u64>(), admin_sq.physical_address().get());
        core::ptr::write_volatile(mmio.add(0x30).cast::<u64>(), admin_cq.physical_address().get());
        mmio_write32(mmio, 0x14, (6 << 16) | (4 << 20) | (1 << 1) | 1);
        let mut ready = false;
        for _ in 0..timeout { if mmio_read32(mmio, 0x1C) & 1 != 0 { ready = true; break; } core::hint::spin_loop(); }
        if !ready { return None; }
        let mut admin = Queue::new(0, mmio, dstrd).ok()?;
        admin.sq = admin_sq;
        admin.cq = admin_cq;
        let identify = DmaBuffer::allocate().ok()?;
        let mut cmd = Submission { opcode: 0x06, cid: 1, prp1: identify.physical_address().get(), ..Default::default() };
        cmd.cdw[0] = 1;
        admin.submit(cmd, ADMIN_TIMEOUT).ok()?;
        let mut cmd = Submission { opcode: 0x06, cid: 2, nsid: 1, prp1: identify.physical_address().get(), ..Default::default() };
        cmd.cdw[0] = 0;
        admin.submit(cmd, ADMIN_TIMEOUT).ok()?;
        let ns = identify.as_slice(PAGE_SIZE).ok()?;
        let ns_size = u64::from_le_bytes(ns[0..8].try_into().ok()?);
        let flbas = ns[26] as usize & 0xF;
        let lbaf_offset = 128usize.checked_add(flbas.checked_mul(4)?)?;
        let lbads = *ns.get(lbaf_offset + 2)?;
        let block_size = 1u32.checked_shl(lbads as u32)?;
        if ns_size == 0 || block_size == 0 || block_size as usize > PAGE_SIZE { return None; }
        let io = Queue::new(1, mmio, dstrd).ok()?;
        let create_cq = Submission { opcode: 0x05, cid: 3, prp1: io.cq.physical_address().get(), cdw: [
            1 | ((QUEUE_DEPTH as u32 - 1) << 16), 3, 0, 0, 0, 0], ..Default::default() };
        admin.submit(create_cq, ADMIN_TIMEOUT).ok()?;
        let create_sq = Submission { opcode: 0x01, cid: 4, prp1: io.sq.physical_address().get(), cdw: [
            1 | ((QUEUE_DEPTH as u32 - 1) << 16), 1 | (1 << 16), 0, 0, 0, 0], ..Default::default() };
        admin.submit(create_sq, ADMIN_TIMEOUT).ok()?;
        Some((mmio, dstrd, admin, io, ns_size, block_size))
    })() else { crate::kprintln!("[nvme] Inicialización fallida o timeout"); return; };
    let (mmio, _, admin, io, block_count, block_size) = mmio_len;
    *CONTROLLER.lock() = Some(NvmeController { mmio, namespace: 1, block_size, block_count, _admin: admin, io });
    crate::kprintln!("[nvme] Namespace 1 listo: {} bloques de {} bytes", block_count, block_size);
    let root_selection = match crate::EXECUTABLE_CMDLINE_REQUEST.get_response() {
        Some(response) => response
            .cmdline()
            .to_str()
            .map_err(|_| ())
            .and_then(|command_line| parse_root_selection(Some(command_line))),
        None => parse_root_selection(None),
    };
    let root_selection = match root_selection {
        Ok(guid) => guid,
        Err(()) => { crate::kprintln!("[nvme] root= inválido; montaje rechazado"); return; }
    };
    if root_selection == RootSelection::InitrdOnly {
        crate::kprintln!("[nvme] root=none: se conserva initrd como raíz Live, sin montar particiones");
        return;
    }
    let requested_guid = match root_selection {
        RootSelection::PartitionGuid(guid) => Some(guid),
        RootSelection::Automatic => None,
        RootSelection::InitrdOnly => unreachable!(),
    };
    match crate::drivers::gpt::find_linux_root(&DISK, requested_guid) {
        Ok(root) => crate::fs::ext4::init_from_disk(root),
        Err(error) => crate::kprintln!("[nvme] GPT sin partición raíz Linux válida: {}", error),
    }
}

impl BlockDevice for NvmeDisk {
    fn geometry(&self) -> Result<BlockGeometry, BlockError> {
        CONTROLLER.lock().as_ref().map(|c| BlockGeometry { block_count: c.block_count, block_size: c.block_size }).ok_or(BlockError::NoDevice)
    }
    fn read_blocks(&self, start: u64, count: u16, buffer: &mut DmaBuffer) -> Result<(), BlockError> {
        let mut lock = CONTROLLER.lock();
        let c = lock.as_mut().ok_or(BlockError::NoDevice)?;
        let end = start.checked_add(count as u64).ok_or(BlockError::InvalidRange)?;
        let bytes = (count as usize).checked_mul(c.block_size as usize).ok_or(BlockError::InvalidRange)?;
        if count == 0 || end > c.block_count || bytes > PAGE_SIZE { return Err(BlockError::InvalidRange); }
        let cmd = Submission { opcode: 0x02, cid: c.io.next_cid, nsid: c.namespace, prp1: buffer.physical_address().get(),
            cdw: [start as u32, (start >> 32) as u32, 0, (count as u32) - 1, 0, 0], ..Default::default() };
        c.io.next_cid = c.io.next_cid.wrapping_add(1);
        unsafe { c.io.submit(cmd, IO_TIMEOUT)?; }
        let _ = c.mmio;
        Ok(())
    }
    fn write_blocks(&self, start: u64, count: u16, buffer: &DmaBuffer) -> Result<(), BlockError> {
        let mut lock = CONTROLLER.lock();
        let c = lock.as_mut().ok_or(BlockError::NoDevice)?;
        let end = start.checked_add(count as u64).ok_or(BlockError::InvalidRange)?;
        let bytes = (count as usize).checked_mul(c.block_size as usize).ok_or(BlockError::InvalidRange)?;
        if count == 0 || end > c.block_count || bytes > PAGE_SIZE { return Err(BlockError::InvalidRange); }
        let cmd = Submission { opcode: 0x01, cid: c.io.next_cid, nsid: c.namespace, prp1: buffer.physical_address().get(),
            cdw: [start as u32, (start >> 32) as u32, 0, (count as u32) - 1, 0, 0], ..Default::default() };
        c.io.next_cid = c.io.next_cid.wrapping_add(1);
        unsafe { c.io.submit(cmd, IO_TIMEOUT)?; }
        Ok(())
    }
    fn sync(&self) -> Result<(), BlockError> {
        let mut lock = CONTROLLER.lock();
        let c = lock.as_mut().ok_or(BlockError::NoDevice)?;
        let cmd = Submission {
            opcode: 0x00,
            cid: c.io.next_cid,
            nsid: c.namespace,
            ..Default::default()
        };
        c.io.next_cid = c.io.next_cid.wrapping_add(1);
        unsafe { c.io.submit(cmd, IO_TIMEOUT)?; }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_root_selection, RootSelection};

    #[test]
    fn root_selector_supports_live_initrd_and_explicit_partition() {
        assert_eq!(
            parse_root_selection(None),
            Ok(RootSelection::Automatic)
        );
        assert_eq!(
            parse_root_selection(Some("quiet root=none")),
            Ok(RootSelection::InitrdOnly)
        );
        assert_eq!(
            parse_root_selection(Some("root=PARTUUID=00112233-4455-6677-8899-aabbccddeeff")),
            Ok(RootSelection::PartitionGuid([
                0x33, 0x22, 0x11, 0x00, 0x55, 0x44, 0x77, 0x66,
                0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
            ]))
        );
    }

    #[test]
    fn root_selector_rejects_conflicting_or_malformed_values() {
        assert!(parse_root_selection(Some("root=none root=none")).is_err());
        assert!(parse_root_selection(Some("root=none root=PARTUUID=00112233-4455-6677-8899-aabbccddeeff")).is_err());
        assert!(parse_root_selection(Some("root=PARTUUID=invalid")).is_err());
        assert!(parse_root_selection(Some("root=/dev/nvme0n1p2")).is_err());
    }
}
