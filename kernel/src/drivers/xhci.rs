use crate::drivers::block::DmaRegion;
use crate::memory::{address::PhysAddr, pmm::PAGE_SIZE};
use crate::sync::IrqSafeSpinlock;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
const MMIO_LENGTH: usize = 0x1_0000;
const RING_TRBS: usize = 256;
const TRB_SIZE: usize = 16;
const TRB_TYPE_LINK: u32 = 6;
const LINK_TOGGLE_CYCLE: u32 = 1 << 1;
const TRB_CYCLE: u32 = 1;
const TRB_TYPE_ENABLE_SLOT: u32 = 9;
const TRB_TYPE_COMMAND_COMPLETION: u32 = 33;
const TRB_TYPE_TRANSFER_EVENT: u32 = 32;
const TRB_TYPE_CONFIGURE_ENDPOINT: u32 = 12;
const TRB_TYPE_PORT_STATUS_CHANGE: u32 = 34;
const PORTSC_CONNECTED: u32 = 1;
const PORTSC_ENABLED: u32 = 1 << 1;
const PORTSC_RESET: u32 = 1 << 4;
const PORTSC_CHANGE_BITS: u32 = 0x7f << 17;
const MAX_EVENT_POLLS: usize = 1024;
struct XhciController {
    _dcbaa: DmaRegion,
    _command_ring: DmaRegion,
    _event_ring: DmaRegion,
    _erst: DmaRegion,
    operational: u64,
    ports: u8,
    _doorbell: u64,
    _devices: Vec<XhciDevice>,
    event_cursor: EventCursor,
    command_index: usize,
    context_size: usize,
    supports_64_bit_dma: bool,
    max_slots: u8,
}
struct XhciDevice {
    slot: u8,
    _port: u8,
    _device_context: DmaRegion,
    _ep0_ring: DmaRegion,
    hid: Option<HidTransfer>,
    wifi: Option<WifiTransfer>,
}
struct WifiTransfer {
    _slot: u8,
    wlan_in_dci: u8,
    wlan_out_dci: u8,
    reg_in_dci: u8,
    reg_out_dci: u8,
    wlan_in_ring: DmaRegion,
    wlan_out_ring: DmaRegion,
    reg_in_ring: DmaRegion,
    reg_out_ring: DmaRegion,
    wlan_rx_buf: DmaRegion,
    reg_rx_buf: DmaRegion,
    wlan_tx_buf: DmaRegion,
    reg_tx_buf: DmaRegion,
    wlan_in_cycle: u32,
    wlan_in_enqueue: usize,
    reg_in_cycle: u32,
    reg_in_enqueue: usize,
    wlan_out_cycle: u32,
    wlan_out_enqueue: usize,
    reg_out_cycle: u32,
    reg_out_enqueue: usize,
}
struct HidTransfer {
    device_id: u16,
    dci: u8,
    kind: lix_usb::BootHidKind,
    max_packet: u16,
    ring: DmaRegion,
    report: DmaRegion,
    cycle: u32,
    enqueue_index: usize,
    keyboard: lix_usb::BootKeyboard,
    mouse: lix_usb::BootMouse,
}
struct EventCursor {
    index: usize,
    cycle: u32,
    physical: u64,
    interrupter: u64,
}
impl EventCursor {
    fn try_next(&mut self, ring: &DmaRegion) -> Option<(u32, u32, u32, u32)> {
        let offset = self.index * TRB_SIZE;
        let bytes = ring.as_slice(PAGE_SIZE).ok()?;
        let control = unsafe { read_volatile(bytes.as_ptr().add(offset + 12).cast::<u32>()) };
        if control & TRB_CYCLE != self.cycle {
            return None;
        }
        let parameter_low = unsafe { read_volatile(bytes.as_ptr().add(offset).cast::<u32>()) };
        let parameter_high = unsafe { read_volatile(bytes.as_ptr().add(offset + 4).cast::<u32>()) };
        let status = unsafe { read_volatile(bytes.as_ptr().add(offset + 8).cast::<u32>()) };
        self.index += 1;
        if self.index == RING_TRBS {
            self.index = 0;
            self.cycle ^= 1;
        }
        unsafe {
            write64(
                self.interrupter,
                0x18,
                self.physical + (self.index * TRB_SIZE) as u64 | (1 << 3),
            );
        }
        Some((parameter_low, parameter_high, status, control))
    }
    fn next(&mut self, ring: &DmaRegion) -> Option<(u32, u32, u32, u32)> {
        for _ in 0..10_000_000 {
            let offset = self.index * TRB_SIZE;
            let bytes = ring.as_slice(PAGE_SIZE).ok()?;
            let control = unsafe { read_volatile(bytes.as_ptr().add(offset + 12).cast::<u32>()) };
            if control & TRB_CYCLE != self.cycle {
                core::hint::spin_loop();
                continue;
            }
            let parameter_low = unsafe { read_volatile(bytes.as_ptr().add(offset).cast::<u32>()) };
            let parameter_high =
                unsafe { read_volatile(bytes.as_ptr().add(offset + 4).cast::<u32>()) };
            let status = unsafe { read_volatile(bytes.as_ptr().add(offset + 8).cast::<u32>()) };
            self.index += 1;
            if self.index == RING_TRBS {
                self.index = 0;
                self.cycle ^= 1;
            }
            unsafe {
                write64(
                    self.interrupter,
                    0x18,
                    self.physical + (self.index * TRB_SIZE) as u64 | (1 << 3),
                );
            }
            return Some((parameter_low, parameter_high, status, control));
        }
        None
    }
}
fn write_trb(
    ring: &mut DmaRegion,
    index: usize,
    parameter: u64,
    status: u32,
    control: u32,
) -> bool {
    let Some(offset) = index.checked_mul(TRB_SIZE) else {
        return false;
    };
    let Ok(bytes) = ring.as_mut_slice(PAGE_SIZE) else {
        return false;
    };
    if offset + TRB_SIZE > bytes.len() {
        return false;
    }
    bytes[offset..offset + 8].copy_from_slice(&parameter.to_le_bytes());
    bytes[offset + 8..offset + 12].copy_from_slice(&status.to_le_bytes());
    bytes[offset + 12..offset + 16].copy_from_slice(&control.to_le_bytes());
    true
}
fn enable_slot(
    command_ring: &mut DmaRegion,
    event_ring: &DmaRegion,
    event_cursor: &mut EventCursor,
    doorbell: u64,
    command_index: &mut usize,
) -> Result<u8, &'static str> {
    let index = *command_index;
    if !write_trb(
        command_ring,
        index,
        0,
        0,
        (TRB_TYPE_ENABLE_SLOT << 10) | TRB_CYCLE,
    ) {
        return Err("unable to write xHCI Enable Slot command");
    }
    *command_index += 1;
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    unsafe {
        write32(doorbell, 0, 0);
    }
    for _ in 0..MAX_EVENT_POLLS {
        let Some((_, _, status, control)) = event_cursor.next(event_ring) else {
            return Err("timed out waiting for xHCI Enable Slot completion");
        };
        let event_type = (control >> 10) & 0x3f;
        if event_type == TRB_TYPE_PORT_STATUS_CHANGE {
            continue;
        }
        if event_type != TRB_TYPE_COMMAND_COMPLETION {
            continue;
        }
        let completion_code = status >> 24;
        let slot = (control >> 24) as u8;
        if completion_code != 1 || slot == 0 {
            return Err("xHCI Enable Slot command failed");
        }
        return Ok(slot);
    }
    Err("timed out waiting for xHCI Enable Slot completion")
}
fn command_completion(
    command_ring: &mut DmaRegion,
    event_ring: &DmaRegion,
    cursor: &mut EventCursor,
    doorbell: u64,
    command_index: &mut usize,
    parameter: u64,
    control: u32,
) -> Result<u8, &'static str> {
    let index = *command_index;
    if !write_trb(command_ring, index, parameter, 0, control | TRB_CYCLE) {
        return Err("unable to write xHCI command TRB");
    }
    *command_index += 1;
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    unsafe {
        write32(doorbell, 0, 0);
    }
    for _ in 0..MAX_EVENT_POLLS {
        let Some((_, _, status, event_control)) = cursor.next(event_ring) else {
            return Err("timed out waiting for xHCI command completion");
        };
        match (event_control >> 10) & 0x3f {
            TRB_TYPE_PORT_STATUS_CHANGE => continue,
            TRB_TYPE_COMMAND_COMPLETION => {
                if status >> 24 != 1 {
                    return Err("xHCI command completion reported failure");
                }
                return Ok((event_control >> 24) as u8);
            }
            _ => continue,
        }
    }
    Err("timed out waiting for xHCI command completion")
}
fn disable_slot(
    command_ring: &mut DmaRegion,
    event_ring: &DmaRegion,
    cursor: &mut EventCursor,
    doorbell: u64,
    command_index: &mut usize,
    slot: u8,
) -> Result<(), &'static str> {
    let returned = command_completion(
        command_ring,
        event_ring,
        cursor,
        doorbell,
        command_index,
        0,
        (10 << 10) | ((slot as u32) << 24),
    )?;
    if returned == slot {
        Ok(())
    } else {
        Err("xHCI disabled an unexpected slot")
    }
}
fn control_in(
    slot: u8,
    doorbell: u64,
    ring: &mut DmaRegion,
    event_ring: &DmaRegion,
    cursor: &mut EventCursor,
    producer: &mut usize,
    buffer: &mut DmaRegion,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
) -> Result<(), &'static str> {
    if *producer + 3 >= RING_TRBS - 1 {
        return Err("xHCI EP0 transfer ring is full");
    }
    let setup = (0x80u64)
        | ((request as u64) << 8)
        | ((value as u64) << 16)
        | ((index as u64) << 32)
        | ((length as u64) << 48);
    let setup_index = *producer;
    let data_index = setup_index + 1;
    let status_index = setup_index + 2;
    if !write_trb(
        ring,
        setup_index,
        setup,
        8,
        (2 << 10) | (1 << 6) | (3 << 16) | TRB_CYCLE,
    ) || !write_trb(
        ring,
        data_index,
        buffer.physical_address().get(),
        length as u32,
        (3 << 10) | (1 << 16) | TRB_CYCLE,
    ) || !write_trb(ring, status_index, 0, 0, (4 << 10) | (1 << 5) | TRB_CYCLE)
    {
        return Err("unable to write xHCI control transfer TRBs");
    }
    *producer += 3;
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    unsafe {
        write32(doorbell, slot as usize * 4, 1);
    }
    for _ in 0..MAX_EVENT_POLLS {
        let Some((_, _, status, control)) = cursor.next(event_ring) else {
            return Err("timed out waiting for USB control transfer");
        };
        if (control >> 10) & 0x3f == 32 && (control >> 24) as u8 == slot {
            let completion = status >> 24;
            if completion == 1 {
                return Ok(());
            }
            crate::kprintln!(
                "[xhci] Control transfer slot {} completó con código {} (TRB={:#x})",
                slot,
                completion,
                (control >> 10) & 0x3f
            );
            return Err("USB control transfer failed");
        }
    }
    Err("timed out waiting for USB control transfer")
}
#[allow(dead_code)]
fn control_out_data(
    slot: u8,
    doorbell: u64,
    ring: &mut DmaRegion,
    event_ring: &DmaRegion,
    cursor: &mut EventCursor,
    producer: &mut usize,
    buffer: &mut DmaRegion,
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    payload: &[u8],
) -> Result<(), &'static str> {
    if request_type & 0x80 != 0
        || payload.is_empty()
        || payload.len() > PAGE_SIZE
        || *producer + 3 >= RING_TRBS - 1
    {
        return Err("invalid USB control OUT payload or full EP0 ring");
    }
    let data = buffer
        .as_mut_slice(payload.len())
        .map_err(|_| "invalid USB control OUT DMA buffer")?;
    data.copy_from_slice(payload);
    let setup = request_type as u64
        | ((request as u64) << 8)
        | ((value as u64) << 16)
        | ((index as u64) << 32)
        | ((payload.len() as u64) << 48);
    let setup_index = *producer;
    let setup_control = (2 << 10) | (1 << 6) | (2 << 16) | TRB_CYCLE;
    let data_control = (3 << 10) | TRB_CYCLE;
    let status_control = (4 << 10) | (1 << 16) | (1 << 5) | TRB_CYCLE;
    if !write_trb(ring, setup_index, setup, 8, setup_control)
        || !write_trb(
            ring,
            setup_index + 1,
            buffer.physical_address().get(),
            payload.len() as u32,
            data_control,
        )
        || !write_trb(ring, setup_index + 2, 0, 0, status_control)
    {
        return Err("unable to write USB control OUT TRBs");
    }
    *producer += 3;
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    unsafe {
        write32(doorbell, slot as usize * 4, 1);
    }
    for _ in 0..MAX_EVENT_POLLS {
        let Some((_, _, status, control)) = cursor.next(event_ring) else {
            return Err("USB control OUT timed out");
        };
        if (control >> 10) & 0x3f == TRB_TYPE_TRANSFER_EVENT && (control >> 24) as u8 == slot {
            return if status >> 24 == 1 {
                Ok(())
            } else {
                Err("USB control OUT failed")
            };
        }
    }
    Err("USB control OUT timed out")
}
fn download_ar9271_firmware(
    slot: u8,
    doorbell: u64,
    ring: &mut DmaRegion,
    event_ring: &DmaRegion,
    cursor: &mut EventCursor,
    producer: &mut usize,
    dma: &mut DmaRegion,
) -> Result<(), &'static str> {
    let image = crate::fs::vfs::read_file("/lib/firmware/ath9k_htc/htc_9271-1.4.0.fw")
        .ok_or("AR9271 firmware file is missing from rootfs")?;
    let mut upload =
        lix_wifi::ath9k_htc::FirmwareUpload::new(lix_wifi::ath9k_htc::Chip::Ar9271, &image)
            .ok_or("invalid AR9271 firmware image size")?;
    while let Some(chunk) = upload.next_chunk() {
        let value = chunk
            .request_value()
            .ok_or("AR9271 firmware address is out of range")?;
        control_out_data(
            slot,
            doorbell,
            ring,
            event_ring,
            cursor,
            producer,
            dma,
            chunk.request_type,
            chunk.request,
            value,
            0,
            chunk.bytes,
        )?;
    }
    let (request, request_type, value) =
        lix_wifi::ath9k_htc::FirmwareUpload::completion_request(lix_wifi::ath9k_htc::Chip::Ar9271);
    control_no_data(
        slot,
        doorbell,
        ring,
        event_ring,
        cursor,
        producer,
        request_type,
        request,
        value,
        0,
    )
}
fn control_no_data(
    slot: u8,
    doorbell: u64,
    ring: &mut DmaRegion,
    event_ring: &DmaRegion,
    cursor: &mut EventCursor,
    producer: &mut usize,
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
) -> Result<(), &'static str> {
    if *producer + 2 >= RING_TRBS - 1 {
        return Err("xHCI EP0 transfer ring is full");
    }
    let setup = request_type as u64
        | ((request as u64) << 8)
        | ((value as u64) << 16)
        | ((index as u64) << 32);
    let setup_index = *producer;
    if !write_trb(
        ring,
        setup_index,
        setup,
        8,
        (2 << 10) | (1 << 6) | TRB_CYCLE,
    ) || !write_trb(
        ring,
        setup_index + 1,
        0,
        0,
        (4 << 10) | (1 << 16) | (1 << 5) | TRB_CYCLE,
    ) {
        return Err("unable to write xHCI no-data control transfer");
    }
    *producer += 2;
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    unsafe {
        write32(doorbell, slot as usize * 4, 1);
    }
    for _ in 0..MAX_EVENT_POLLS {
        let Some((_, _, status, control)) = cursor.next(event_ring) else {
            return Err("USB control transfer timed out");
        };
        if (control >> 10) & 0x3f == TRB_TYPE_TRANSFER_EVENT && (control >> 24) as u8 == slot {
            return if status >> 24 == 1 {
                Ok(())
            } else {
                Err("USB control request failed")
            };
        }
    }
    Err("USB control transfer timed out")
}
fn configure_boot_hid(
    slot: u8,
    device_id: u16,
    speed: u8,
    context_size: usize,
    device_context: &DmaRegion,
    command_ring: &mut DmaRegion,
    event_ring: &DmaRegion,
    cursor: &mut EventCursor,
    doorbell: u64,
    command_index: &mut usize,
    endpoint: lix_usb::BootHidEndpoint,
    supports_64_bit_dma: bool,
) -> Result<HidTransfer, &'static str> {
    let endpoint_number = endpoint.endpoint.address & 0x0f;
    let dci = endpoint_number
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .ok_or("invalid HID endpoint number")?;
    let minimum_report = match endpoint.kind {
        lix_usb::BootHidKind::Keyboard => 8,
        lix_usb::BootHidKind::Mouse => 3,
    };
    if endpoint_number == 0
        || dci > 31
        || endpoint.endpoint.max_packet_size == 0
        || endpoint.endpoint.max_packet_size < minimum_report
        || endpoint.endpoint.max_packet_size as usize > PAGE_SIZE
    {
        return Err("unsupported HID interrupt endpoint parameters");
    }
    let mut input =
        DmaRegion::allocate(PAGE_SIZE).map_err(|_| "HID input context allocation failed")?;
    let mut ring =
        DmaRegion::allocate(PAGE_SIZE).map_err(|_| "HID transfer ring allocation failed")?;
    let report =
        DmaRegion::allocate(PAGE_SIZE).map_err(|_| "HID report buffer allocation failed")?;
    if !supports_64_bit_dma
        && [&input, &ring, &report].iter().any(|region| {
            region
                .physical_address()
                .get()
                .saturating_add(region.capacity() as u64)
                > (1u64 << 32)
        })
    {
        return Err("xHCI without AC64 needs HID DMA memory below 4 GiB");
    }
    let context_bytes = device_context
        .as_slice(PAGE_SIZE)
        .map_err(|_| "invalid slot context")?;
    let slot_offset = context_size;
    let endpoint_offset = context_size * (dci as usize + 1);
    let input_bytes = input
        .as_mut_slice(PAGE_SIZE)
        .map_err(|_| "invalid HID input context")?;
    input_bytes[4..8].copy_from_slice(&((1u32) | (1u32 << dci)).to_le_bytes());
    let mut slot_dw0 = u32::from_le_bytes(
        context_bytes[0..4]
            .try_into()
            .map_err(|_| "invalid slot context")?,
    );
    slot_dw0 = (slot_dw0 & !(0x1f << 27)) | ((dci as u32) << 27);
    input_bytes[slot_offset..slot_offset + context_size]
        .copy_from_slice(&context_bytes[..context_size]);
    input_bytes[slot_offset..slot_offset + 4].copy_from_slice(&slot_dw0.to_le_bytes());
    let interval = if speed >= 3 {
        endpoint.endpoint.interval.saturating_sub(1).min(15) as u32
    } else {
        (endpoint.endpoint.interval.max(1) as u32 * 8)
            .next_power_of_two()
            .trailing_zeros()
            .min(15)
    };
    let ep_dw0 = interval << 16;
    let ep_type = 7u32;
    let ep_dw1 = (3 << 1) | (ep_type << 3) | ((endpoint.endpoint.max_packet_size as u32) << 16);
    let ring_phys = ring.physical_address().get();
    let ring_bytes = ring
        .as_mut_slice(PAGE_SIZE)
        .map_err(|_| "invalid HID ring")?;
    let link = (RING_TRBS - 1) * TRB_SIZE;
    ring_bytes[link..link + 8].copy_from_slice(&ring_phys.to_le_bytes());
    ring_bytes[link + 12..link + 16]
        .copy_from_slice(&((TRB_TYPE_LINK << 10) | LINK_TOGGLE_CYCLE | TRB_CYCLE).to_le_bytes());
    input_bytes[endpoint_offset..endpoint_offset + 4].copy_from_slice(&ep_dw0.to_le_bytes());
    input_bytes[endpoint_offset + 4..endpoint_offset + 8].copy_from_slice(&ep_dw1.to_le_bytes());
    input_bytes[endpoint_offset + 8..endpoint_offset + 16]
        .copy_from_slice(&(ring_phys | 1).to_le_bytes());
    input_bytes[endpoint_offset + 16..endpoint_offset + 20]
        .copy_from_slice(&(endpoint.endpoint.max_packet_size as u32).to_le_bytes());
    command_completion(
        command_ring,
        event_ring,
        cursor,
        doorbell,
        command_index,
        input.physical_address().get(),
        (TRB_TYPE_CONFIGURE_ENDPOINT << 10) | ((slot as u32) << 24),
    )?;
    let mut transfer = HidTransfer {
        device_id,
        dci,
        kind: endpoint.kind,
        max_packet: endpoint.endpoint.max_packet_size,
        ring,
        report,
        cycle: 1,
        enqueue_index: 1,
        keyboard: lix_usb::BootKeyboard::default(),
        mouse: lix_usb::BootMouse::default(),
    };
    write_trb(
        &mut transfer.ring,
        0,
        transfer.report.physical_address().get(),
        transfer.max_packet as u32,
        (1 << 10) | (1 << 5) | TRB_CYCLE,
    )
    .then_some(())
    .ok_or("unable to arm HID interrupt transfer")?;
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    unsafe {
        write32(doorbell, slot as usize * 4, dci as u32);
    }
    Ok(transfer)
}
fn configure_wifi_endpoints(
    slot: u8,
    speed: u8,
    context_size: usize,
    device_context: &DmaRegion,
    command_ring: &mut DmaRegion,
    event_ring: &DmaRegion,
    cursor: &mut EventCursor,
    doorbell: u64,
    command_index: &mut usize,
    endpoints: lix_wifi::ath9k_htc::HtcEndpointSet,
    supports_64_bit_dma: bool,
) -> Result<WifiTransfer, &'static str> {
    let wlan_in_ep = endpoints.wlan_in.address & 0x0f;
    let wlan_out_ep = endpoints.wlan_out.address & 0x0f;
    let reg_in_ep = endpoints.reg_in.address & 0x0f;
    let reg_out_ep = endpoints.reg_out.address & 0x0f;

    let wlan_in_dci = wlan_in_ep
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .ok_or("invalid wlan in ep")?;
    let wlan_out_dci = wlan_out_ep.checked_mul(2).ok_or("invalid wlan out ep")?;
    let reg_in_dci = reg_in_ep
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .ok_or("invalid reg in ep")?;
    let reg_out_dci = reg_out_ep.checked_mul(2).ok_or("invalid reg out ep")?;

    if wlan_in_dci > 31 || wlan_out_dci > 31 || reg_in_dci > 31 || reg_out_dci > 31 {
        return Err("wifi endpoint dci out of bounds");
    }

    let mut input = DmaRegion::allocate(PAGE_SIZE).map_err(|_| "wifi input context alloc")?;
    let mut wlan_in_ring = DmaRegion::allocate(PAGE_SIZE).map_err(|_| "wlan in ring alloc")?;
    let mut wlan_out_ring = DmaRegion::allocate(PAGE_SIZE).map_err(|_| "wlan out ring alloc")?;
    let mut reg_in_ring = DmaRegion::allocate(PAGE_SIZE).map_err(|_| "reg in ring alloc")?;
    let mut reg_out_ring = DmaRegion::allocate(PAGE_SIZE).map_err(|_| "reg out ring alloc")?;
    let wlan_rx_buf = DmaRegion::allocate(PAGE_SIZE).map_err(|_| "wlan rx buf alloc")?;
    let reg_rx_buf = DmaRegion::allocate(PAGE_SIZE).map_err(|_| "reg rx buf alloc")?;
    let wlan_tx_buf = DmaRegion::allocate(PAGE_SIZE).map_err(|_| "wlan tx buf alloc")?;
    let reg_tx_buf = DmaRegion::allocate(PAGE_SIZE).map_err(|_| "reg tx buf alloc")?;

    if !supports_64_bit_dma
        && [
            &input,
            &wlan_in_ring,
            &wlan_out_ring,
            &reg_in_ring,
            &reg_out_ring,
            &wlan_rx_buf,
            &reg_rx_buf,
            &wlan_tx_buf,
            &reg_tx_buf,
        ]
        .iter()
        .any(|region| {
            region.physical_address().get() + region.capacity() as u64 > (1u64 << 32)
        })
    {
        return Err("xHCI without AC64 needs wifi dma memory below 4 GiB");
    }

    let context_bytes = device_context
        .as_slice(PAGE_SIZE)
        .map_err(|_| "invalid slot context")?;
    let slot_offset = context_size;
    let input_bytes = input
        .as_mut_slice(PAGE_SIZE)
        .map_err(|_| "invalid wifi input context")?;

    let add_flags = 1u32
        | (1u32 << wlan_in_dci)
        | (1u32 << wlan_out_dci)
        | (1u32 << reg_in_dci)
        | (1u32 << reg_out_dci);
    input_bytes[4..8].copy_from_slice(&add_flags.to_le_bytes());

    let max_dci = wlan_in_dci
        .max(wlan_out_dci)
        .max(reg_in_dci)
        .max(reg_out_dci);
    let mut slot_dw0 = u32::from_le_bytes(
        context_bytes[0..4]
            .try_into()
            .map_err(|_| "invalid slot context")?,
    );
    slot_dw0 = (slot_dw0 & !(0x1f << 27)) | ((max_dci as u32) << 27);
    input_bytes[slot_offset..slot_offset + context_size]
        .copy_from_slice(&context_bytes[..context_size]);
    input_bytes[slot_offset..slot_offset + 4].copy_from_slice(&slot_dw0.to_le_bytes());

    let reg_in_interval = if speed >= 3 {
        endpoints.reg_in.interval.saturating_sub(1).min(15) as u32
    } else {
        (endpoints.reg_in.interval.max(1) as u32 * 8)
            .next_power_of_two()
            .trailing_zeros()
            .min(15)
    };
    let reg_out_interval = if speed >= 3 {
        endpoints.reg_out.interval.saturating_sub(1).min(15) as u32
    } else {
        (endpoints.reg_out.interval.max(1) as u32 * 8)
            .next_power_of_two()
            .trailing_zeros()
            .min(15)
    };

    let write_ep = |bytes: &mut [u8],
                    dci: u8,
                    ep_type: u32,
                    interval: u32,
                    max_packet: u16,
                    ring_phys: u64| {
        let offset = context_size * (dci as usize + 1);
        let ep_dw0 = interval << 16;
        let ep_dw1 = (3 << 1) | (ep_type << 3) | ((max_packet as u32) << 16);
        bytes[offset..offset + 4].copy_from_slice(&ep_dw0.to_le_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&ep_dw1.to_le_bytes());
        bytes[offset + 8..offset + 16].copy_from_slice(&(ring_phys | 1).to_le_bytes());
        bytes[offset + 16..offset + 20].copy_from_slice(&(max_packet as u32).to_le_bytes());
    };

    write_ep(
        input_bytes,
        wlan_in_dci,
        6,
        0,
        endpoints.wlan_in.max_packet_size,
        wlan_in_ring.physical_address().get(),
    );
    write_ep(
        input_bytes,
        wlan_out_dci,
        2,
        0,
        endpoints.wlan_out.max_packet_size,
        wlan_out_ring.physical_address().get(),
    );
    write_ep(
        input_bytes,
        reg_in_dci,
        7,
        reg_in_interval,
        endpoints.reg_in.max_packet_size,
        reg_in_ring.physical_address().get(),
    );
    write_ep(
        input_bytes,
        reg_out_dci,
        3,
        reg_out_interval,
        endpoints.reg_out.max_packet_size,
        reg_out_ring.physical_address().get(),
    );

    let setup_ring = |ring: &mut DmaRegion| -> Result<(), &'static str> {
        let phys = ring.physical_address().get();
        let bytes = ring.as_mut_slice(PAGE_SIZE).map_err(|_| "ring slice error")?;
        let link = (RING_TRBS - 1) * TRB_SIZE;
        bytes[link..link + 8].copy_from_slice(&phys.to_le_bytes());
        bytes[link + 12..link + 16].copy_from_slice(
            &((TRB_TYPE_LINK << 10) | LINK_TOGGLE_CYCLE | TRB_CYCLE).to_le_bytes(),
        );
        Ok(())
    };
    setup_ring(&mut wlan_in_ring)?;
    setup_ring(&mut wlan_out_ring)?;
    setup_ring(&mut reg_in_ring)?;
    setup_ring(&mut reg_out_ring)?;

    command_completion(
        command_ring,
        event_ring,
        cursor,
        doorbell,
        command_index,
        input.physical_address().get(),
        (TRB_TYPE_CONFIGURE_ENDPOINT << 10) | ((slot as u32) << 24),
    )?;

    write_trb(
        &mut wlan_in_ring,
        0,
        wlan_rx_buf.physical_address().get(),
        PAGE_SIZE as u32,
        (1 << 10) | (1 << 5) | TRB_CYCLE,
    );
    write_trb(
        &mut reg_in_ring,
        0,
        reg_rx_buf.physical_address().get(),
        PAGE_SIZE as u32,
        (1 << 10) | (1 << 5) | TRB_CYCLE,
    );
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    unsafe {
        write32(doorbell, slot as usize * 4, wlan_in_dci as u32);
        write32(doorbell, slot as usize * 4, reg_in_dci as u32);
    }

    Ok(WifiTransfer {
        _slot: slot,
        wlan_in_dci,
        wlan_out_dci,
        reg_in_dci,
        reg_out_dci,
        wlan_in_ring,
        wlan_out_ring,
        reg_in_ring,
        reg_out_ring,
        wlan_rx_buf,
        reg_rx_buf,
        wlan_tx_buf,
        reg_tx_buf,
        wlan_in_cycle: 1,
        wlan_in_enqueue: 1,
        reg_in_cycle: 1,
        reg_in_enqueue: 1,
        wlan_out_cycle: 1,
        wlan_out_enqueue: 0,
        reg_out_cycle: 1,
        reg_out_enqueue: 0,
    })
}
fn address_and_read_device_descriptor(
    dcbaa: &mut DmaRegion,
    command_ring: &mut DmaRegion,
    event_ring: &DmaRegion,
    cursor: &mut EventCursor,
    doorbell: u64,
    command_index: &mut usize,
    slot: u8,
    port: u8,
    speed: u8,
    context_size: usize,
    supports_64_bit_dma: bool,
) -> Result<
    (
        Option<lix_usb::DeviceDescriptor>,
        Option<lix_usb::BootHidEndpoint>,
        XhciDevice,
    ),
    &'static str,
> {
    let device_context =
        DmaRegion::allocate(PAGE_SIZE).map_err(|_| "xHCI device context allocation failed")?;
    let mut input_context =
        DmaRegion::allocate(PAGE_SIZE).map_err(|_| "xHCI input context allocation failed")?;
    let mut ep0_ring =
        DmaRegion::allocate(PAGE_SIZE).map_err(|_| "xHCI EP0 ring allocation failed")?;
    let mut descriptor_buffer =
        DmaRegion::allocate(PAGE_SIZE).map_err(|_| "USB descriptor buffer allocation failed")?;
    if !supports_64_bit_dma
        && [
            &device_context,
            &input_context,
            &ep0_ring,
            &descriptor_buffer,
        ]
        .iter()
        .any(|region| region.physical_address().get() + region.capacity() as u64 > (1u64 << 32))
    {
        return Err("xHCI without AC64 needs device contexts and buffers below 4 GiB");
    }
    let initial_packet = match speed {
        1 => 8u16,
        2 => 8,
        3 => 64,
        _ => 512,
    };
    let slot_offset = context_size;
    let ep0_offset = context_size * 2;
    let input = input_context
        .as_mut_slice(PAGE_SIZE)
        .map_err(|_| "invalid xHCI input context")?;
    input[4..8].copy_from_slice(&3u32.to_le_bytes());
    let slot_dw0 = ((speed as u32) << 20) | (1 << 27);
    let slot_dw1 = (port as u32) << 16;
    input[slot_offset..slot_offset + 4].copy_from_slice(&slot_dw0.to_le_bytes());
    input[slot_offset + 4..slot_offset + 8].copy_from_slice(&slot_dw1.to_le_bytes());
    let ep0_phys = ep0_ring.physical_address().get();
    let ring = ep0_ring
        .as_mut_slice(PAGE_SIZE)
        .map_err(|_| "invalid xHCI EP0 transfer ring")?;
    let link_offset = (RING_TRBS - 1) * TRB_SIZE;
    ring[link_offset..link_offset + 8].copy_from_slice(&ep0_phys.to_le_bytes());
    ring[link_offset + 12..link_offset + 16]
        .copy_from_slice(&((TRB_TYPE_LINK << 10) | LINK_TOGGLE_CYCLE | TRB_CYCLE).to_le_bytes());
    let ep0 = input_context
        .as_mut_slice(PAGE_SIZE)
        .map_err(|_| "invalid xHCI EP0 context")?;
    let ep0_dw1 = (3 << 1) | (4 << 3) | ((initial_packet as u32) << 16);
    ep0[ep0_offset + 4..ep0_offset + 8].copy_from_slice(&ep0_dw1.to_le_bytes());
    ep0[ep0_offset + 8..ep0_offset + 16].copy_from_slice(&(ep0_phys | 1).to_le_bytes());
    ep0[ep0_offset + 16..ep0_offset + 20].copy_from_slice(&8u32.to_le_bytes());
    let dcbaa_bytes = dcbaa
        .as_mut_slice(PAGE_SIZE)
        .map_err(|_| "invalid xHCI DCBAA")?;
    let dcbaa_offset = slot as usize * 8;
    dcbaa_bytes[dcbaa_offset..dcbaa_offset + 8]
        .copy_from_slice(&device_context.physical_address().get().to_le_bytes());
    let returned_slot = match command_completion(
        command_ring,
        event_ring,
        cursor,
        doorbell,
        command_index,
        input_context.physical_address().get(),
        (11 << 10) | ((slot as u32) << 24),
    ) {
        Ok(returned_slot) => returned_slot,
        Err(error) => {
            crate::kprintln!("[xhci] Address Device slot {} falló: {}", slot, error);
            return Ok((
                None,
                None,
                XhciDevice {
                    slot,
                    _port: port,
                    _device_context: device_context,
                    _ep0_ring: ep0_ring,
                    hid: None,
                    wifi: None,
                },
            ));
        }
    };
    if returned_slot != slot {
        return Ok((
            None,
            None,
            XhciDevice {
                slot,
                _port: port,
                _device_context: device_context,
                _ep0_ring: ep0_ring,
                hid: None,
                wifi: None,
            },
        ));
    }
    let mut transfer_index = 0;
    if control_in(
        slot,
        doorbell,
        &mut ep0_ring,
        event_ring,
        cursor,
        &mut transfer_index,
        &mut descriptor_buffer,
        6,
        0x0100,
        0,
        18,
    )
    .is_err()
    {
        crate::kprintln!("[xhci] GET_DESCRIPTOR(Device) falló en slot {}", slot);
        return Ok((
            None,
            None,
            XhciDevice {
                slot,
                _port: port,
                _device_context: device_context,
                _ep0_ring: ep0_ring,
                hid: None,
                wifi: None,
            },
        ));
    }
    let descriptor_bytes = descriptor_buffer
        .as_slice(18)
        .map_err(|_| "invalid USB descriptor buffer")?;
    let descriptor = lix_usb::DeviceDescriptor::parse(descriptor_bytes);
    if descriptor.is_none() {
        crate::kprintln!(
            "[xhci] Descriptor Device inválido en slot {}: {:02x?}",
            slot,
            &descriptor_bytes[..18]
        );
    }
    let (boot_hid, wifi_endpoints) = if descriptor.is_some()
        && control_in(
            slot,
            doorbell,
            &mut ep0_ring,
            event_ring,
            cursor,
            &mut transfer_index,
            &mut descriptor_buffer,
            6,
            0x0200,
            0,
            9,
        )
        .is_ok()
    {
        let total_length = descriptor_buffer
            .as_slice(9)
            .ok()
            .and_then(|bytes| bytes.get(2..4))
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]) as usize)
            .filter(|length| (9..=PAGE_SIZE).contains(length));
        total_length
            .and_then(|length| {
                if control_in(
                    slot,
                    doorbell,
                    &mut ep0_ring,
                    event_ring,
                    cursor,
                    &mut transfer_index,
                    &mut descriptor_buffer,
                    6,
                    0x0200,
                    0,
                    length as u16,
                )
                .is_err()
                {
                    return None;
                }
                let bytes = descriptor_buffer.as_slice(length).ok()?;
                if let Ok(Some(hci)) = lix_usb::find_bluetooth_hci_endpoints(bytes) {
                    crate::kprintln!(
                        "[usb] Interfaz Bluetooth HCI detectada: interface {}, eventos {:#04x}, ACL IN {:#04x}, ACL OUT {:#04x}; transporte xHCI pendiente",
                        hci.interface.number,
                        hci.event_in.address,
                        hci.acl_in.address,
                        hci.acl_out.address
                    );
                }
                let mut matched_wifi_ep = None;
                if let Some(device) = descriptor {
                    let wifi_matched = crate::drivers::wifi::probe_usb(
                        slot,
                        device.vendor_id,
                        device.product_id,
                        bytes,
                    );
                    if wifi_matched {
                        if let Ok(Some(endpoints)) = lix_wifi::ath9k_htc::find_htc_endpoints(bytes) {
                            let selected = endpoints.configuration_value != 0
                                && control_no_data(
                                    slot,
                                    doorbell,
                                    &mut ep0_ring,
                                    event_ring,
                                    cursor,
                                    &mut transfer_index,
                                    0x00,
                                    9,
                                    endpoints.configuration_value as u16,
                                    0,
                                )
                                .is_ok()
                                && (endpoints.alternate_setting == 0
                                    || control_no_data(
                                        slot,
                                        doorbell,
                                        &mut ep0_ring,
                                        event_ring,
                                        cursor,
                                        &mut transfer_index,
                                        0x01,
                                        11,
                                        endpoints.alternate_setting as u16,
                                        endpoints.interface_number as u16,
                                    )
                                    .is_ok());
                            if !selected {
                                crate::drivers::wifi::set_usb_state(
                                    slot,
                                    crate::drivers::wifi::DeviceState::Failed,
                                );
                                crate::kprintln!(
                                    "[wifi] no se pudo seleccionar configuración/interfaz USB HIF"
                                );
                                return None;
                            }
                            matched_wifi_ep = Some(endpoints);
                        }
                    }
                    if lix_wifi::ath9k_htc::match_usb(device.vendor_id, device.product_id)
                        == Some(lix_wifi::ath9k_htc::Chip::Ar9271)
                    {
                        match DmaRegion::allocate(PAGE_SIZE) {
                            Ok(mut firmware_dma) => match download_ar9271_firmware(
                                slot,
                                doorbell,
                                &mut ep0_ring,
                                event_ring,
                                cursor,
                                &mut transfer_index,
                                &mut firmware_dma,
                            ) {
                                Ok(()) => {
                                    crate::drivers::wifi::set_usb_state(
                                        slot,
                                        crate::drivers::wifi::DeviceState::HtcStarting,
                                    );
                                    crate::kprintln!(
                                        "[wifi] AR9271 firmware uploaded; HTC startup awaits bulk transport"
                                    );
                                }
                                Err(error) => {
                                    crate::drivers::wifi::set_usb_state(
                                        slot,
                                        crate::drivers::wifi::DeviceState::Failed,
                                    );
                                    crate::kprintln!(
                                        "[wifi] AR9271 firmware upload failed: {}",
                                        error
                                    );
                                }
                            },
                            Err(_) => crate::kprintln!(
                                "[wifi] no DMA memory available for AR9271 firmware"
                            ),
                        }
                    }
                }
                let hid = lix_usb::find_boot_hid_endpoint(bytes).ok().flatten();
                Some((hid, matched_wifi_ep))
            })
            .unwrap_or((None, None))
    } else {
        (None, None)
    };
    let hid_transfer = if let Some(hid) = boot_hid {
        let configured = control_no_data(
            slot,
            doorbell,
            &mut ep0_ring,
            event_ring,
            cursor,
            &mut transfer_index,
            0x00,
            9,
            hid.configuration_value as u16,
            0,
        )
        .is_ok()
            && control_no_data(
                slot,
                doorbell,
                &mut ep0_ring,
                event_ring,
                cursor,
                &mut transfer_index,
                0x21,
                0x0b,
                0,
                hid.interface.number as u16,
            )
            .is_ok();
        if configured {
            match configure_boot_hid(
                slot,
                slot as u16,
                speed,
                context_size,
                &device_context,
                command_ring,
                event_ring,
                cursor,
                doorbell,
                command_index,
                hid,
                supports_64_bit_dma,
            ) {
                Ok(transfer) => Some(transfer),
                Err(error) => {
                    crate::kprintln!("[usb] HID slot {}: {}", slot, error);
                    None
                }
            }
        } else {
            crate::kprintln!(
                "[usb] No se pudo seleccionar/configurar protocolo HID en slot {}",
                slot
            );
            None
        }
    } else {
        None
    };
    let wifi_transfer = if let Some(endpoints) = wifi_endpoints {
        match configure_wifi_endpoints(
            slot,
            speed,
            context_size,
            &device_context,
            command_ring,
            event_ring,
            cursor,
            doorbell,
            command_index,
            endpoints,
            supports_64_bit_dma,
        ) {
            Ok(transfer) => Some(transfer),
            Err(error) => {
                crate::kprintln!("[wifi] error configurando endpoints xHCI: {}", error);
                None
            }
        }
    } else {
        None
    };
    Ok((
        descriptor,
        boot_hid,
        XhciDevice {
            slot,
            _port: port,
            _device_context: device_context,
            _ep0_ring: ep0_ring,
            hid: hid_transfer,
            wifi: wifi_transfer,
        },
    ))
}
static CONTROLLER: IrqSafeSpinlock<Option<XhciController>> = IrqSafeSpinlock::new(None);
struct ControllerLease(Option<XhciController>);
impl Drop for ControllerLease {
    fn drop(&mut self) {
        if let Some(controller) = self.0.take() {
            *CONTROLLER.lock() = Some(controller);
        }
    }
}
unsafe fn read32(base: u64, offset: usize) -> u32 {
    read_volatile((base + offset as u64) as *const u32)
}
unsafe fn write32(base: u64, offset: usize, value: u32) {
    write_volatile((base + offset as u64) as *mut u32, value);
}
unsafe fn write64(base: u64, offset: usize, value: u64) {
    write_volatile((base + offset as u64) as *mut u64, value);
}
fn wait_until(mut predicate: impl FnMut() -> bool, spins: usize) -> bool {
    for _ in 0..spins {
        if predicate() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}
struct ControllerRegisters {
    operational: u64,
    interrupter0: u64,
    doorbell: u64,
    max_slots: u8,
    ports: u8,
    context_size: usize,
    supports_64_bit_dma: bool,
}
fn map_registers(bus: u8, slot: u8, function: u8) -> Result<ControllerRegisters, &'static str> {
    let bar_low = crate::drivers::pci::pci_read_dword(bus, slot, function, 0x10);
    if bar_low & 1 != 0 {
        return Err("xHCI BAR0 is I/O space, expected MMIO");
    }
    let bar_type = (bar_low >> 1) & 3;
    let bar_high = if bar_type == 2 {
        crate::drivers::pci::pci_read_dword(bus, slot, function, 0x14)
    } else {
        0
    };
    let physical = ((bar_high as u64) << 32) | (bar_low as u64 & !0xf);
    if physical == 0 {
        return Err("xHCI BAR0 is unassigned");
    }
    crate::drivers::pci::enable_memory_and_bus_master(bus, slot, function);
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
    }
    let pml4_frame = ((cr3 & 0x000f_ffff_ffff_f000) / PAGE_SIZE as u64) as usize;
    let mmio = unsafe {
        crate::memory::vmm::map_mmio_region(pml4_frame, PhysAddr::new(physical), MMIO_LENGTH)
    }
    .map_err(|_| "unable to map xHCI registers")?
    .get();
    let cap_length = unsafe { read_volatile(mmio as *const u8) } as usize;
    let hcs1 = unsafe { read32(mmio, 0x04) };
    let hcc1 = unsafe { read32(mmio, 0x10) };
    let runtime_offset = (unsafe { read32(mmio, 0x18) } & !0x1f) as usize;
    let doorbell_offset = (unsafe { read32(mmio, 0x14) } & !0x3) as usize;
    let max_slots = (hcs1 & 0xff) as u8;
    let ports = ((hcs1 >> 24) & 0xff) as u8;
    if cap_length < 0x20 || max_slots == 0 || ports == 0 {
        return Err("invalid xHCI capability registers");
    }
    let supports_64_bit_dma = hcc1 & 1 != 0;
    let context_size = if hcc1 & (1 << 2) != 0 { 64 } else { 32 };
    let operational = mmio + cap_length as u64;
    let runtime = mmio + runtime_offset as u64;
    Ok(ControllerRegisters {
        operational,
        interrupter0: runtime + 0x20,
        doorbell: mmio + doorbell_offset as u64,
        max_slots,
        ports,
        context_size,
        supports_64_bit_dma,
    })
}
fn reset_controller(operational: u64) -> Result<(), &'static str> {
    let command = unsafe { read32(operational, 0) };
    unsafe {
        write32(operational, 0, command & !1);
    }
    if !wait_until(|| unsafe { read32(operational, 4) & 1 != 0 }, 10_000_000) {
        return Err("xHCI did not halt");
    }
    unsafe {
        write32(operational, 0, (command & !1) | (1 << 1));
    }
    if !wait_until(
        || unsafe { read32(operational, 0) & (1 << 1) == 0 },
        10_000_000,
    ) {
        return Err("xHCI reset timed out");
    }
    if !wait_until(
        || unsafe { read32(operational, 4) & (1 << 11) == 0 },
        10_000_000,
    ) {
        return Err("xHCI controller is not ready after reset");
    }
    Ok(())
}
struct ControllerRings {
    dcbaa: DmaRegion,
    command_ring: DmaRegion,
    event_ring: DmaRegion,
    erst: DmaRegion,
}
fn allocate_rings(supports_64_bit_dma: bool) -> Result<ControllerRings, &'static str> {
    let dcbaa = DmaRegion::allocate(PAGE_SIZE).map_err(|_| "xHCI DCBAA allocation failed")?;
    let mut command_ring =
        DmaRegion::allocate(PAGE_SIZE).map_err(|_| "xHCI command ring allocation failed")?;
    let event_ring =
        DmaRegion::allocate(PAGE_SIZE).map_err(|_| "xHCI event ring allocation failed")?;
    let mut erst = DmaRegion::allocate(PAGE_SIZE).map_err(|_| "xHCI ERST allocation failed")?;
    if !supports_64_bit_dma {
        let dma_addresses = [
            dcbaa.physical_address().get(),
            command_ring.physical_address().get(),
            event_ring.physical_address().get(),
            erst.physical_address().get(),
        ];
        if dma_addresses
            .iter()
            .any(|address| *address + PAGE_SIZE as u64 > (1u64 << 32))
        {
            return Err("xHCI without AC64 needs DMA rings below 4 GiB");
        }
    }
    let link_offset = (RING_TRBS - 1) * TRB_SIZE;
    let command_ring_physical = command_ring.physical_address().get();
    {
        let ring = command_ring
            .as_mut_slice(PAGE_SIZE)
            .map_err(|_| "invalid xHCI command ring")?;
        ring[link_offset..link_offset + 8].copy_from_slice(&command_ring_physical.to_le_bytes());
        ring[link_offset + 8..link_offset + 12].copy_from_slice(&0u32.to_le_bytes());
        ring[link_offset + 12..link_offset + 16].copy_from_slice(
            &((TRB_TYPE_LINK << 10) | LINK_TOGGLE_CYCLE | TRB_CYCLE).to_le_bytes(),
        );
    }
    {
        let table = erst
            .as_mut_slice(PAGE_SIZE)
            .map_err(|_| "invalid xHCI ERST")?;
        table[0..8].copy_from_slice(&event_ring.physical_address().get().to_le_bytes());
        table[8..12].copy_from_slice(&(RING_TRBS as u32).to_le_bytes());
    }
    Ok(ControllerRings {
        dcbaa,
        command_ring,
        event_ring,
        erst,
    })
}
fn start_controller(
    registers: &ControllerRegisters,
    rings: &ControllerRings,
) -> Result<(), &'static str> {
    let command_ring_physical = rings.command_ring.physical_address().get();
    unsafe {
        write64(
            registers.operational,
            0x30,
            rings.dcbaa.physical_address().get(),
        );
        write32(
            registers.operational,
            0x38,
            registers.max_slots.min(8) as u32,
        );
        write64(registers.operational, 0x18, command_ring_physical | 1);
        write32(registers.interrupter0, 0x08, 1);
        write64(
            registers.interrupter0,
            0x10,
            rings.erst.physical_address().get(),
        );
        write64(
            registers.interrupter0,
            0x18,
            rings.event_ring.physical_address().get(),
        );
        write32(registers.interrupter0, 0x00, 0);
        write32(registers.operational, 0, 1);
    }
    if !wait_until(
        || unsafe { read32(registers.operational, 4) & 1 == 0 },
        10_000_000,
    ) {
        return Err("xHCI failed to enter the running state");
    }
    Ok(())
}
pub fn init(bus: u8, slot: u8, function: u8) -> Result<(), &'static str> {
    if CONTROLLER.lock().is_some() {
        return Err("an xHCI controller is already active");
    }
    let registers = map_registers(bus, slot, function)?;
    reset_controller(registers.operational)?;
    let mut rings = allocate_rings(registers.supports_64_bit_dma)?;
    start_controller(&registers, &rings)?;
    let enumeration = enumerate_ports(&registers, &mut rings);
    *CONTROLLER.lock() = Some(XhciController {
        _dcbaa: rings.dcbaa,
        _command_ring: rings.command_ring,
        _event_ring: rings.event_ring,
        _erst: rings.erst,
        operational: registers.operational,
        ports: registers.ports,
        _doorbell: registers.doorbell,
        _devices: enumeration.devices,
        event_cursor: enumeration.event_cursor,
        command_index: enumeration.command_index,
        context_size: registers.context_size,
        supports_64_bit_dma: registers.supports_64_bit_dma,
        max_slots: registers.max_slots.min(8),
    });
    crate::kprintln!(
        "[xhci] Controlador iniciado; {} puertos disponibles, {} conectados, {} slots asignados",
        registers.ports,
        enumeration.connected_ports,
        enumeration.allocated_slots
    );
    Ok(())
}
struct Enumeration {
    devices: Vec<XhciDevice>,
    event_cursor: EventCursor,
    command_index: usize,
    connected_ports: u8,
    allocated_slots: usize,
}
fn enumerate_ports(registers: &ControllerRegisters, rings: &mut ControllerRings) -> Enumeration {
    let ControllerRegisters {
        operational,
        doorbell,
        interrupter0,
        max_slots,
        ports,
        context_size,
        supports_64_bit_dma,
    } = *registers;
    let dcbaa = &mut rings.dcbaa;
    let command_ring = &mut rings.command_ring;
    let event_ring = &rings.event_ring;
    let mut connected_ports = 0u8;
    let mut enabled_slots = Vec::new();
    let mut devices = Vec::new();
    let mut event_cursor = EventCursor {
        index: 0,
        cycle: 1,
        physical: event_ring.physical_address().get(),
        interrupter: interrupter0,
    };
    let mut command_index = 0;
    for port in 0..ports {
        let port_offset = 0x400 + port as usize * 0x10;
        let portsc = unsafe { read32(operational, port_offset) };
        if portsc & PORTSC_CONNECTED != 0 {
            connected_ports = connected_ports.saturating_add(1);
            unsafe {
                write32(
                    operational,
                    port_offset,
                    (portsc & !PORTSC_CHANGE_BITS) | PORTSC_RESET,
                );
            }
            let reset_done = wait_until(
                || unsafe { read32(operational, port_offset) & PORTSC_RESET == 0 },
                10_000_000,
            );
            if !reset_done || unsafe { read32(operational, port_offset) } & PORTSC_ENABLED == 0 {
                crate::kprintln!("[xhci] Puerto USB {} no completó reset", port + 1);
                continue;
            }
            if enabled_slots.len() >= max_slots.min(8) as usize {
                crate::kprintln!(
                    "[xhci] Límite de slots ocupados; puerto {} omitido",
                    port + 1
                );
                continue;
            }
            match enable_slot(
                command_ring,
                event_ring,
                &mut event_cursor,
                doorbell,
                &mut command_index,
            ) {
                Ok(device_slot) => {
                    enabled_slots.push((device_slot, port + 1));
                    crate::kprintln!(
                        "[xhci] Slot {} asignado al puerto {}",
                        device_slot,
                        port + 1
                    );
                    let speed = ((unsafe { read32(operational, port_offset) } >> 10) & 0xf) as u8;
                    match address_and_read_device_descriptor(
                        dcbaa,
                        command_ring,
                        event_ring,
                        &mut event_cursor,
                        doorbell,
                        &mut command_index,
                        device_slot,
                        port + 1,
                        speed,
                        context_size,
                        supports_64_bit_dma,
                    ) {
                        Ok((descriptor, boot_hid, device)) => {
                            if let Some(descriptor) = descriptor {
                                crate::drivers::usb::register(lix_sys::UsbDeviceInfo {
                                    id: device_slot as u16,
                                    root_port: port + 1,
                                    slot: device_slot,
                                    vendor_id: descriptor.vendor_id,
                                    product_id: descriptor.product_id,
                                    device_class: if descriptor.device_class == 0
                                        && boot_hid.is_some()
                                    {
                                        lix_usb::CLASS_HID
                                    } else {
                                        descriptor.device_class
                                    },
                                    device_subclass: descriptor.device_subclass,
                                    device_protocol: descriptor.device_protocol,
                                    reserved: 0,
                                    state: crate::drivers::usb::DEVICE_CONNECTED,
                                });
                                crate::kprintln!(
                                    "[usb] Enumerado {:04x}:{:04x} en puerto {} (slot {})",
                                    descriptor.vendor_id,
                                    descriptor.product_id,
                                    port + 1,
                                    device_slot
                                );
                                if let Some(hid) = boot_hid {
                                    crate::kprintln!(
                                        "[usb] HID boot {:?}, endpoint {:#04x}",
                                        hid.kind,
                                        hid.endpoint.address
                                    );
                                }
                            } else {
                                crate::kprintln!(
                                    "[usb] Slot {} direccionado sin descriptor válido",
                                    device_slot
                                );
                            }
                            devices.push(device);
                        }
                        Err(error) => crate::kprintln!("[usb] Slot {}: {}", device_slot, error),
                    }
                }
                Err(error) => crate::kprintln!("[xhci] Puerto {}: {}", port + 1, error),
            }
            crate::kprintln!(
                "[xhci] Puerto USB {} conectado (velocidad {})",
                port + 1,
                (unsafe { read32(operational, port_offset) } >> 10) & 0xf
            );
        }
    }
    Enumeration {
        devices,
        event_cursor,
        command_index,
        connected_ports,
        allocated_slots: enabled_slots.len(),
    }
}
pub fn active() -> bool {
    CONTROLLER.lock().is_some()
}
fn acknowledge_port_changes(controller: &XhciController, offset: usize, portsc: u32) {
    let changes = portsc & PORTSC_CHANGE_BITS;
    if changes != 0 {
        unsafe {
            write32(
                controller.operational,
                offset,
                (portsc & !(PORTSC_CHANGE_BITS | PORTSC_RESET)) | changes,
            );
        }
    }
}
fn poll_port(controller: &mut XhciController, port: u8) {
    let offset = 0x400 + (port as usize - 1) * 0x10;
    let mut portsc = unsafe { read32(controller.operational, offset) };
    let connected = portsc & PORTSC_CONNECTED != 0;
    let device_index = controller
        ._devices
        .iter()
        .position(|device| device._port == port);
    if !connected {
        if let Some(index) = device_index {
            let slot = controller._devices[index].slot;
            match disable_slot(
                &mut controller._command_ring,
                &controller._event_ring,
                &mut controller.event_cursor,
                controller._doorbell,
                &mut controller.command_index,
                slot,
            ) {
                Ok(()) => {
                    crate::drivers::usb::remove(slot as u16);
                    crate::drivers::wifi::remove_usb(slot);
                    controller._devices.remove(index);
                    crate::kprintln!("[usb] Desconectado puerto {}, slot {} liberado", port, slot);
                }
                Err(error) => crate::kprintln!("[usb] No se pudo liberar slot {}: {}", slot, error),
            }
        }
        acknowledge_port_changes(controller, offset, portsc);
        return;
    }
    if device_index.is_some() {
        acknowledge_port_changes(controller, offset, portsc);
        return;
    }
    if controller._devices.len() >= controller.max_slots as usize {
        crate::kprintln!("[usb] Sin slots disponibles para el puerto {}", port);
        return;
    }
    acknowledge_port_changes(controller, offset, portsc);
    portsc = unsafe { read32(controller.operational, offset) };
    unsafe {
        write32(
            controller.operational,
            offset,
            (portsc & !PORTSC_CHANGE_BITS) | PORTSC_RESET,
        );
    }
    if !wait_until(
        || unsafe { read32(controller.operational, offset) & PORTSC_RESET == 0 },
        10_000_000,
    ) {
        crate::kprintln!("[usb] Timeout al resetear puerto {}", port);
        return;
    }
    portsc = unsafe { read32(controller.operational, offset) };
    if portsc & (PORTSC_CONNECTED | PORTSC_ENABLED) != PORTSC_CONNECTED | PORTSC_ENABLED {
        acknowledge_port_changes(controller, offset, portsc);
        return;
    }
    let speed = ((portsc >> 10) & 0xf) as u8;
    let slot = match enable_slot(
        &mut controller._command_ring,
        &controller._event_ring,
        &mut controller.event_cursor,
        controller._doorbell,
        &mut controller.command_index,
    ) {
        Ok(slot) => slot,
        Err(error) => {
            crate::kprintln!("[usb] Enable Slot en puerto {}: {}", port, error);
            return;
        }
    };
    match address_and_read_device_descriptor(
        &mut controller._dcbaa,
        &mut controller._command_ring,
        &controller._event_ring,
        &mut controller.event_cursor,
        controller._doorbell,
        &mut controller.command_index,
        slot,
        port,
        speed,
        controller.context_size,
        controller.supports_64_bit_dma,
    ) {
        Ok((descriptor, boot_hid, device)) => {
            if let Some(descriptor) = descriptor {
                crate::drivers::usb::register(lix_sys::UsbDeviceInfo {
                    id: slot as u16,
                    root_port: port,
                    slot,
                    vendor_id: descriptor.vendor_id,
                    product_id: descriptor.product_id,
                    device_class: if descriptor.device_class == 0 && boot_hid.is_some() {
                        lix_usb::CLASS_HID
                    } else {
                        descriptor.device_class
                    },
                    device_subclass: descriptor.device_subclass,
                    device_protocol: descriptor.device_protocol,
                    reserved: 0,
                    state: crate::drivers::usb::DEVICE_CONNECTED,
                });
                crate::kprintln!(
                    "[usb] Conectado {:04x}:{:04x} en puerto {} (slot {})",
                    descriptor.vendor_id,
                    descriptor.product_id,
                    port,
                    slot
                );
            }
            controller._devices.push(device);
        }
        Err(error) => {
            crate::kprintln!("[usb] Enumeración de puerto {} falló: {}", port, error);
            let _ = disable_slot(
                &mut controller._command_ring,
                &controller._event_ring,
                &mut controller.event_cursor,
                controller._doorbell,
                &mut controller.command_index,
                slot,
            );
        }
    }
    acknowledge_port_changes(controller, offset, unsafe {
        read32(controller.operational, offset)
    });
}
pub fn poll() {
    let mut lease = ControllerLease(CONTROLLER.lock().take());
    if let Some(controller) = lease.0.as_mut() {
        let status = unsafe { read32(controller.operational, 4) };
        if status & (1 << 2) != 0 {
            crate::kprintln!("[xhci] ERROR: el controlador notificó un error del host");
            unsafe {
                write32(controller.operational, 4, 1 << 2);
            }
        }
        for _ in 0..MAX_EVENT_POLLS {
            let Some((parameter_low, parameter_high, event_status, control)) =
                controller.event_cursor.try_next(&controller._event_ring)
            else {
                break;
            };
            let event_type = (control >> 10) & 0x3f;
            if event_type != TRB_TYPE_TRANSFER_EVENT {
                continue;
            }
            let slot = (control >> 24) as u8;
            let dci = ((control >> 16) & 0x1f) as u8;
            let Some(device) = controller
                ._devices
                .iter_mut()
                .find(|device| device.slot == slot)
            else {
                continue;
            };
            if let Some(wifi) = device.wifi.as_mut() {
                if wifi.wlan_in_dci == dci {
                    let completion = event_status >> 24;
                    if completion == 1 {
                        let residual = (event_status & 0x00ff_ffff) as usize;
                        let length = PAGE_SIZE.saturating_sub(residual);
                        if let Ok(data) = wifi.wlan_rx_buf.as_slice(length) {
                            crate::drivers::wifi::on_wlan_rx(slot, data);
                        }
                    }
                    let ring_cycle = if wifi.wlan_in_cycle != 0 { TRB_CYCLE } else { 0 };
                    let _ = write_trb(
                        &mut wifi.wlan_in_ring,
                        wifi.wlan_in_enqueue,
                        wifi.wlan_rx_buf.physical_address().get(),
                        PAGE_SIZE as u32,
                        (1 << 10) | (1 << 5) | ring_cycle,
                    );
                    wifi.wlan_in_enqueue += 1;
                    if wifi.wlan_in_enqueue >= RING_TRBS - 1 {
                        let link = (RING_TRBS - 1) * TRB_SIZE;
                        if let Ok(bytes) = wifi.wlan_in_ring.as_mut_slice(PAGE_SIZE) {
                            let link_control =
                                (TRB_TYPE_LINK << 10) | LINK_TOGGLE_CYCLE | ring_cycle;
                            bytes[link + 12..link + 16]
                                .copy_from_slice(&link_control.to_le_bytes());
                        }
                        wifi.wlan_in_cycle ^= 1;
                        wifi.wlan_in_enqueue = 0;
                    }
                    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
                    unsafe {
                        write32(controller._doorbell, slot as usize * 4, dci as u32);
                    }
                    let _ = (parameter_low, parameter_high);
                    continue;
                }
                if wifi.reg_in_dci == dci {
                    let completion = event_status >> 24;
                    if completion == 1 {
                        let residual = (event_status & 0x00ff_ffff) as usize;
                        let length = PAGE_SIZE.saturating_sub(residual);
                        if let Ok(data) = wifi.reg_rx_buf.as_slice(length) {
                            crate::drivers::wifi::on_control_rx(slot, data);
                        }
                    }
                    let ring_cycle = if wifi.reg_in_cycle != 0 { TRB_CYCLE } else { 0 };
                    let _ = write_trb(
                        &mut wifi.reg_in_ring,
                        wifi.reg_in_enqueue,
                        wifi.reg_rx_buf.physical_address().get(),
                        PAGE_SIZE as u32,
                        (1 << 10) | (1 << 5) | ring_cycle,
                    );
                    wifi.reg_in_enqueue += 1;
                    if wifi.reg_in_enqueue >= RING_TRBS - 1 {
                        let link = (RING_TRBS - 1) * TRB_SIZE;
                        if let Ok(bytes) = wifi.reg_in_ring.as_mut_slice(PAGE_SIZE) {
                            let link_control =
                                (TRB_TYPE_LINK << 10) | LINK_TOGGLE_CYCLE | ring_cycle;
                            bytes[link + 12..link + 16]
                                .copy_from_slice(&link_control.to_le_bytes());
                        }
                        wifi.reg_in_cycle ^= 1;
                        wifi.reg_in_enqueue = 0;
                    }
                    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
                    unsafe {
                        write32(controller._doorbell, slot as usize * 4, dci as u32);
                    }
                    let _ = (parameter_low, parameter_high);
                    continue;
                }
            }
            let Some(hid) = device.hid.as_mut().filter(|hid| hid.dci == dci) else {
                continue;
            };
            let completion = event_status >> 24;
            if completion == 1 {
                let residual = (event_status & 0x00ff_ffff) as usize;
                let length = (hid.max_packet as usize).saturating_sub(residual);
                if let Ok(report) = hid.report.as_slice(length) {
                    match hid.kind {
                        lix_usb::BootHidKind::Keyboard => {
                            let (events, count) = hid.keyboard.push_report(report);
                            for event in events.into_iter().take(count) {
                                crate::drivers::input::push_usb(hid.device_id, event);
                                if event.kind == lix_usb::INPUT_KEY {
                                    if event.value != 0 {
                                        if let Some(decoded) =
                                            crate::drivers::keyboard::process_hid_key(
                                                event.code, true,
                                            )
                                        {
                                            crate::drivers::keyboard::dispatch_decoded_key(decoded);
                                        }
                                    } else {
                                        let _ = crate::drivers::keyboard::process_hid_key(
                                            event.code, false,
                                        );
                                    }
                                }
                            }
                        }
                        lix_usb::BootHidKind::Mouse => {
                            let (events, count) = hid.mouse.push_report(report);
                            for event in events.into_iter().take(count) {
                                crate::drivers::input::push_usb(hid.device_id, event);
                            }
                        }
                    }
                }
            } else {
                crate::kprintln!(
                    "[usb] Transfer HID falló: slot {}, endpoint {}, código {}",
                    slot,
                    dci,
                    completion
                );
            }
            let ring_cycle = if hid.cycle != 0 { TRB_CYCLE } else { 0 };
            let _ = write_trb(
                &mut hid.ring,
                hid.enqueue_index,
                hid.report.physical_address().get(),
                hid.max_packet as u32,
                (1 << 10) | (1 << 5) | ring_cycle,
            );
            hid.enqueue_index += 1;
            if hid.enqueue_index >= RING_TRBS - 1 {
                let link = (RING_TRBS - 1) * TRB_SIZE;
                if let Ok(bytes) = hid.ring.as_mut_slice(PAGE_SIZE) {
                    let link_control = (TRB_TYPE_LINK << 10) | LINK_TOGGLE_CYCLE | ring_cycle;
                    bytes[link + 12..link + 16].copy_from_slice(&link_control.to_le_bytes());
                }
                hid.cycle ^= 1;
                hid.enqueue_index = 0;
            }
            core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
            unsafe {
                write32(controller._doorbell, slot as usize * 4, dci as u32);
            }
            let _ = (parameter_low, parameter_high);
        }
        for port in 1..=controller.ports {
            poll_port(controller, port);
        }
    }
}
pub fn wifi_send_wlan(slot: u8, packet: &[u8]) -> Result<(), &'static str> {
    let mut lease = ControllerLease(CONTROLLER.lock().take());
    let Some(controller) = lease.0.as_mut() else {
        return Err("xHCI controller unavailable");
    };
    let Some(device) = controller._devices.iter_mut().find(|d| d.slot == slot) else {
        return Err("device not found");
    };
    let Some(wifi) = device.wifi.as_mut() else {
        return Err("wifi not initialized on slot");
    };
    if packet.len() > PAGE_SIZE {
        return Err("packet too large");
    }
    let tx_slice = wifi
        .wlan_tx_buf
        .as_mut_slice(PAGE_SIZE)
        .map_err(|_| "dma buffer slice error")?;
    tx_slice[..packet.len()].copy_from_slice(packet);
    let cycle = if wifi.wlan_out_cycle != 0 {
        TRB_CYCLE
    } else {
        0
    };
    let _ = write_trb(
        &mut wifi.wlan_out_ring,
        wifi.wlan_out_enqueue,
        wifi.wlan_tx_buf.physical_address().get(),
        packet.len() as u32,
        (1 << 10) | (1 << 5) | cycle,
    );
    wifi.wlan_out_enqueue += 1;
    if wifi.wlan_out_enqueue >= RING_TRBS - 1 {
        let link = (RING_TRBS - 1) * TRB_SIZE;
        if let Ok(bytes) = wifi.wlan_out_ring.as_mut_slice(PAGE_SIZE) {
            let link_control = (TRB_TYPE_LINK << 10) | LINK_TOGGLE_CYCLE | cycle;
            bytes[link + 12..link + 16].copy_from_slice(&link_control.to_le_bytes());
        }
        wifi.wlan_out_cycle ^= 1;
        wifi.wlan_out_enqueue = 0;
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    unsafe {
        write32(controller._doorbell, slot as usize * 4, wifi.wlan_out_dci as u32);
    }
    Ok(())
}
pub fn wifi_send_control(slot: u8, packet: &[u8]) -> Result<(), &'static str> {
    let mut lease = ControllerLease(CONTROLLER.lock().take());
    let Some(controller) = lease.0.as_mut() else {
        return Err("xHCI controller unavailable");
    };
    let Some(device) = controller._devices.iter_mut().find(|d| d.slot == slot) else {
        return Err("device not found");
    };
    let Some(wifi) = device.wifi.as_mut() else {
        return Err("wifi not initialized on slot");
    };
    if packet.len() > PAGE_SIZE {
        return Err("packet too large");
    }
    let tx_slice = wifi
        .reg_tx_buf
        .as_mut_slice(PAGE_SIZE)
        .map_err(|_| "dma buffer slice error")?;
    tx_slice[..packet.len()].copy_from_slice(packet);
    let cycle = if wifi.reg_out_cycle != 0 {
        TRB_CYCLE
    } else {
        0
    };
    let _ = write_trb(
        &mut wifi.reg_out_ring,
        wifi.reg_out_enqueue,
        wifi.reg_tx_buf.physical_address().get(),
        packet.len() as u32,
        (1 << 10) | (1 << 5) | cycle,
    );
    wifi.reg_out_enqueue += 1;
    if wifi.reg_out_enqueue >= RING_TRBS - 1 {
        let link = (RING_TRBS - 1) * TRB_SIZE;
        if let Ok(bytes) = wifi.reg_out_ring.as_mut_slice(PAGE_SIZE) {
            let link_control = (TRB_TYPE_LINK << 10) | LINK_TOGGLE_CYCLE | cycle;
            bytes[link + 12..link + 16].copy_from_slice(&link_control.to_le_bytes());
        }
        wifi.reg_out_cycle ^= 1;
        wifi.reg_out_enqueue = 0;
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    unsafe {
        write32(controller._doorbell, slot as usize * 4, wifi.reg_out_dci as u32);
    }
    Ok(())
}
