use crate::drivers::block::{DmaBuffer, DmaRegion};
use crate::drivers::net::{self, NetworkDevice, NetworkError};
use crate::memory::pmm::PAGE_SIZE;
use crate::sync::IrqSafeSpinlock;
use alloc::vec::Vec;
use core::sync::atomic::{compiler_fence, Ordering};
use core::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
use lix_net::MacAddress;
use lix_net::{DhcpMessage, DhcpMessageType, Ipv4Address, Ipv4Packet, UdpDatagram};
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;
const STATUS_FAILED: u8 = 128;
const FEATURE_MAC: u32 = 1 << 5;
const QUEUE_SIZE_LIMIT: u16 = 1024;
const RX_BUFFER_LENGTH: usize = 2048;
const VIRTIO_NET_HEADER_LENGTH: usize = 10;
fn in8(port: u16) -> u8 {
    unsafe {
        let value: u8;
        core::arch::asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
        value
    }
}
fn in16(port: u16) -> u16 {
    unsafe {
        let value: u16;
        core::arch::asm!("in ax, dx", out("ax") value, in("dx") port, options(nomem, nostack, preserves_flags));
        value
    }
}
fn in32(port: u16) -> u32 {
    unsafe {
        let value: u32;
        core::arch::asm!("in eax, dx", out("eax") value, in("dx") port, options(nomem, nostack, preserves_flags));
        value
    }
}
fn out8(port: u16, value: u8) {
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
    }
}
fn out16(port: u16, value: u16) {
    unsafe {
        core::arch::asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
    }
}
fn out32(port: u16, value: u32) {
    unsafe {
        core::arch::asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
    }
}
struct VirtQueue {
    memory: DmaRegion,
    size: u16,
    available_index: u16,
    used_index: u16,
}
impl VirtQueue {
    fn new(size: u16) -> Result<Self, &'static str> {
        if size == 0 || !size.is_power_of_two() || size > QUEUE_SIZE_LIMIT {
            return Err("virtio queue has unsupported size");
        }
        let available_offset = 16usize
            .checked_mul(size as usize)
            .ok_or("virtio queue size overflow")?;
        let used_offset =
            (available_offset + 4 + 2 * size as usize + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let required = used_offset
            .checked_add(4 + 8 * size as usize)
            .ok_or("virtio queue size overflow")?;
        Ok(Self {
            memory: DmaRegion::allocate(required).map_err(|_| "virtio DMA allocation failed")?,
            size,
            available_index: 0,
            used_index: 0,
        })
    }
    fn avail_offset(&self) -> usize {
        16 * self.size as usize
    }
    fn used_offset(&self) -> usize {
        (self.avail_offset() + 4 + 2 * self.size as usize + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
    }
    fn set_descriptor(
        &mut self,
        id: usize,
        address: u64,
        length: u32,
        flags: u16,
        next: u16,
    ) -> Result<(), &'static str> {
        let descriptor_limit = self.avail_offset();
        let bytes = self
            .memory
            .as_mut_slice(self.memory.capacity())
            .map_err(|_| "invalid virtio queue memory")?;
        let offset = id.checked_mul(16).ok_or("virtio descriptor overflow")?;
        if offset + 16 > descriptor_limit {
            return Err("virtio descriptor index outside queue");
        }
        bytes[offset..offset + 8].copy_from_slice(&address.to_le_bytes());
        bytes[offset + 8..offset + 12].copy_from_slice(&length.to_le_bytes());
        bytes[offset + 12..offset + 14].copy_from_slice(&flags.to_le_bytes());
        bytes[offset + 14..offset + 16].copy_from_slice(&next.to_le_bytes());
        Ok(())
    }
    fn publish(&mut self, id: u16) -> Result<(), &'static str> {
        let slot = self.available_index % self.size;
        let offset = self.avail_offset();
        let bytes = self
            .memory
            .as_mut_slice(self.memory.capacity())
            .map_err(|_| "invalid virtio queue memory")?;
        let ring_entry = offset + 4 + 2 * slot as usize;
        bytes[ring_entry..ring_entry + 2].copy_from_slice(&id.to_le_bytes());
        compiler_fence(Ordering::Release);
        self.available_index = self.available_index.wrapping_add(1);
        bytes[offset + 2..offset + 4].copy_from_slice(&self.available_index.to_le_bytes());
        Ok(())
    }
    fn read_used(&mut self) -> Result<Option<(u32, u32)>, &'static str> {
        let used_offset = self.used_offset();
        let bytes = self
            .memory
            .as_slice(self.memory.capacity())
            .map_err(|_| "invalid virtio queue memory")?;
        let index = u16::from_le_bytes(
            bytes[used_offset + 2..used_offset + 4]
                .try_into()
                .map_err(|_| "bad used index")?,
        );
        compiler_fence(Ordering::Acquire);
        if index == self.used_index {
            return Ok(None);
        }
        let slot = self.used_index % self.size;
        let entry = used_offset + 4 + slot as usize * 8;
        let id = u32::from_le_bytes(
            bytes[entry..entry + 4]
                .try_into()
                .map_err(|_| "bad used id")?,
        );
        let length = u32::from_le_bytes(
            bytes[entry + 4..entry + 8]
                .try_into()
                .map_err(|_| "bad used length")?,
        );
        self.used_index = self.used_index.wrapping_add(1);
        Ok(Some((id, length)))
    }
}
struct VirtioNet {
    io_base: u16,
    mac: MacAddress,
    rx_queue: VirtQueue,
    tx_queue: VirtQueue,
    rx_buffers: Vec<DmaBuffer>,
    tx_buffers: Vec<DmaBuffer>,
    tx_pending: u16,
    next_tx: u16,
}
static DEVICE: IrqSafeSpinlock<Option<VirtioNet>> = IrqSafeSpinlock::new(None);
static DEVICE_HANDLE: VirtioNetHandle = VirtioNetHandle;
static DHCP_XID: AtomicU32 = AtomicU32::new(0);
static DHCP_STATE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
const DHCP_DISCOVER_SENT: u8 = 1;
const DHCP_REQUEST_SENT: u8 = 2;
const DHCP_BOUND: u8 = 3;
struct VirtioNetHandle;
impl NetworkDevice for VirtioNetHandle {
    fn mac_address(&self) -> MacAddress {
        DEVICE
            .lock()
            .as_ref()
            .map_or(MacAddress([0; 6]), |device| device.mac)
    }
    fn transmit(&self, frame: &[u8]) -> Result<(), NetworkError> {
        let mut state = DEVICE.lock();
        let device = state.as_mut().ok_or(NetworkError::NoDevice)?;
        enqueue_frame(device, frame)
    }
}
fn enqueue_frame(device: &mut VirtioNet, frame: &[u8]) -> Result<(), NetworkError> {
    if frame.len() > RX_BUFFER_LENGTH - VIRTIO_NET_HEADER_LENGTH {
        return Err(NetworkError::FrameTooLarge);
    }
    reap_transmit(device).map_err(|_| NetworkError::DeviceFault)?;
    if device.tx_pending >= device.tx_queue.size {
        return Err(NetworkError::QueueFull);
    }
    let id = device.next_tx % device.tx_queue.size;
    let buffer = &mut device.tx_buffers[id as usize];
    let bytes = buffer
        .as_mut_slice(PAGE_SIZE)
        .map_err(|_| NetworkError::DeviceFault)?;
    bytes[..VIRTIO_NET_HEADER_LENGTH].fill(0);
    bytes[VIRTIO_NET_HEADER_LENGTH..VIRTIO_NET_HEADER_LENGTH + frame.len()].copy_from_slice(frame);
    let physical = buffer.physical_address().get();
    device
        .tx_queue
        .set_descriptor(
            id as usize,
            physical,
            (VIRTIO_NET_HEADER_LENGTH + frame.len()) as u32,
            0,
            0,
        )
        .map_err(|_| NetworkError::DeviceFault)?;
    device
        .tx_queue
        .publish(id)
        .map_err(|_| NetworkError::DeviceFault)?;
    device.tx_pending += 1;
    device.next_tx = device.next_tx.wrapping_add(1);
    out16(device.io_base + 16, 1);
    Ok(())
}
fn reap_transmit(device: &mut VirtioNet) -> Result<(), &'static str> {
    while device.tx_queue.read_used()?.is_some() {
        device.tx_pending = device.tx_pending.saturating_sub(1);
    }
    Ok(())
}
fn configure_queue(io: u16, index: u16, queue: &VirtQueue) -> Result<(), &'static str> {
    out16(io + 14, index);
    let max_size = in16(io + 12);
    if max_size != queue.size {
        return Err("virtio queue size changed during setup");
    }
    if queue.memory.physical_address().get() & 0xfff != 0 {
        return Err("virtio queue is not page aligned");
    }
    let pfn = u32::try_from(queue.memory.physical_address().get() >> 12)
        .map_err(|_| "legacy virtio queue is above 4 GiB")?;
    out32(io + 8, pfn);
    Ok(())
}
#[derive(Clone, Copy)]
enum DhcpEvent {
    Offer {
        address: Ipv4Address,
        server: Ipv4Address,
    },
    Ack(net::Ipv4Config),
    Nak,
}
fn parse_dhcp_frame(frame: &[u8]) -> Option<DhcpEvent> {
    let ethernet = lix_net::EthernetFrame::parse(frame)?;
    if ethernet.ether_type != 0x0800 {
        return None;
    }
    let ip = Ipv4Packet::parse(ethernet.payload)?;
    if ip.protocol != 17 {
        return None;
    }
    let udp = UdpDatagram::parse(ip.payload)?;
    if udp.source_port != 67 || udp.destination_port != 68 {
        return None;
    }
    let dhcp = DhcpMessage::parse(udp.payload)?;
    if dhcp.operation != 2 || dhcp.transaction_id != DHCP_XID.load(AtomicOrdering::Acquire) {
        return None;
    }
    match dhcp.message_type? {
        DhcpMessageType::Offer => Some(DhcpEvent::Offer {
            address: dhcp.offered_address,
            server: dhcp.server_id?,
        }),
        DhcpMessageType::Ack => Some(DhcpEvent::Ack(net::Ipv4Config {
            address: dhcp.offered_address,
            subnet_mask: dhcp.subnet_mask,
            gateway: dhcp.router,
            dns_server: dhcp.dns_server,
        })),
        DhcpMessageType::Nak => Some(DhcpEvent::Nak),
        _ => None,
    }
}
fn handle_dhcp_event(device: &mut VirtioNet, event: DhcpEvent) {
    match event {
        DhcpEvent::Offer { address, server }
            if DHCP_STATE
                .compare_exchange(
                    DHCP_DISCOVER_SENT,
                    DHCP_REQUEST_SENT,
                    AtomicOrdering::AcqRel,
                    AtomicOrdering::Acquire,
                )
                .is_ok() =>
        {
            let mut request = [0u8; 320];
            if let Some(length) = lix_net::build_dhcp_request_frame(
                DHCP_XID.load(AtomicOrdering::Acquire),
                device.mac,
                address,
                server,
                &mut request,
            ) {
                if enqueue_frame(device, &request[..length]).is_ok() {
                    crate::kprintln!(
                        "[dhcp] DHCPREQUEST enviado para {}.{}.{}.{}",
                        address.0[0],
                        address.0[1],
                        address.0[2],
                        address.0[3]
                    );
                }
            }
        }
        DhcpEvent::Ack(config)
            if DHCP_STATE
                .compare_exchange(
                    DHCP_REQUEST_SENT,
                    DHCP_BOUND,
                    AtomicOrdering::AcqRel,
                    AtomicOrdering::Acquire,
                )
                .is_ok() =>
        {
            net::set_ipv4_config(config);
            crate::kprintln!(
                "[dhcp] Lease IPv4 adquirido: {}.{}.{}.{}",
                config.address.0[0],
                config.address.0[1],
                config.address.0[2],
                config.address.0[3]
            );
        }
        DhcpEvent::Nak => {
            DHCP_STATE.store(0, AtomicOrdering::Release);
            crate::kprintln!("[dhcp] El servidor rechazo la solicitud de lease");
        }
        _ => {}
    }
}
pub fn init(bus: u8, slot: u8, function: u8) -> Result<(), &'static str> {
    let bar = crate::drivers::pci::pci_read_dword(bus, slot, function, 0x10);
    if bar & 1 == 0 {
        return Err("virtio-net legacy I/O BAR not found");
    }
    let io = (bar & !3) as u16;
    if io == 0 {
        return Err("virtio-net I/O BAR is zero");
    }
    crate::drivers::pci::enable_io_and_bus_master(bus, slot, function);
    out8(io + 18, 0);
    out8(io + 18, STATUS_ACKNOWLEDGE);
    out8(io + 18, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
    let host_features = in32(io);
    if host_features & FEATURE_MAC == 0 {
        out8(io + 18, STATUS_FAILED);
        return Err("virtio-net device does not offer a MAC address");
    }
    out32(io + 4, FEATURE_MAC);
    let mut mac_bytes = [0u8; 6];
    for (index, byte) in mac_bytes.iter_mut().enumerate() {
        *byte = in8(io + 20 + index as u16);
    }
    out16(io + 14, 0);
    let rx_size = in16(io + 12);
    out16(io + 14, 1);
    let tx_size = in16(io + 12);
    if rx_size == 0 || tx_size == 0 || rx_size > QUEUE_SIZE_LIMIT || tx_size > QUEUE_SIZE_LIMIT {
        out8(io + 18, STATUS_FAILED);
        return Err("virtio network queue size is unsupported");
    }
    let mut rx_queue = VirtQueue::new(rx_size)?;
    let tx_queue = VirtQueue::new(tx_size)?;
    configure_queue(io, 0, &rx_queue)?;
    configure_queue(io, 1, &tx_queue)?;
    let mut rx_buffers = Vec::with_capacity(rx_queue.size as usize);
    let mut tx_buffers = Vec::with_capacity(tx_queue.size as usize);
    for id in 0..rx_queue.size {
        let buffer = DmaBuffer::allocate().map_err(|_| "virtio RX buffer allocation failed")?;
        rx_queue.set_descriptor(
            id as usize,
            buffer.physical_address().get(),
            RX_BUFFER_LENGTH as u32,
            2,
            0,
        )?;
        rx_queue.publish(id)?;
        rx_buffers.push(buffer);
        tx_buffers.push(DmaBuffer::allocate().map_err(|_| "virtio TX buffer allocation failed")?);
    }
    compiler_fence(Ordering::Release);
    out16(io + 16, 0);
    out8(
        io + 18,
        STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
    );
    *DEVICE.lock() = Some(VirtioNet {
        io_base: io,
        mac: MacAddress(mac_bytes),
        rx_queue,
        tx_queue,
        rx_buffers,
        tx_buffers,
        tx_pending: 0,
        next_tx: 0,
    });
    net::register(&DEVICE_HANDLE);
    let xid = 0x4c69_7800 ^ u32::from_le_bytes(mac_bytes[..4].try_into().unwrap_or([0; 4]));
    DHCP_XID.store(xid, AtomicOrdering::Release);
    DHCP_STATE.store(DHCP_DISCOVER_SENT, AtomicOrdering::Release);
    let mut discover = [0u8; 320];
    if let Some(length) =
        lix_net::build_dhcp_discover_frame(xid, MacAddress(mac_bytes), &mut discover)
    {
        if net::transmit(&discover[..length]).is_ok() {
            crate::kprintln!("[dhcp] DHCPDISCOVER transmitido (xid={:#010x})", xid);
        }
    }
    crate::kprintln!(
        "[virtio-net] Legacy PCI listo en {}:{}:{} ({:02x?})",
        bus,
        slot,
        function,
        mac_bytes
    );
    Ok(())
}
pub fn poll() {
    let mut state = DEVICE.lock();
    let Some(device) = state.as_mut() else {
        return;
    };
    let _ = reap_transmit(device);
    loop {
        let used = match device.rx_queue.read_used() {
            Ok(Some(used)) => used,
            _ => break,
        };
        let (id, length) = used;
        if id >= device.rx_queue.size as u32 {
            break;
        }
        let buffer = &mut device.rx_buffers[id as usize];
        let received_length = (length as usize).min(RX_BUFFER_LENGTH);
        let event = if received_length >= VIRTIO_NET_HEADER_LENGTH {
            if let Ok(bytes) = buffer.as_slice(received_length) {
                let frame = &bytes[VIRTIO_NET_HEADER_LENGTH..received_length];
                let _ = net::receive(frame);
                parse_dhcp_frame(frame)
            } else {
                None
            }
        } else {
            None
        };
        let physical = buffer.physical_address().get();
        if device
            .rx_queue
            .set_descriptor(id as usize, physical, RX_BUFFER_LENGTH as u32, 2, 0)
            .is_err()
            || device.rx_queue.publish(id as u16).is_err()
        {
            break;
        }
        if let Some(event) = event {
            handle_dhcp_event(device, event);
        }
    }
    out16(device.io_base + 16, 0);
}
