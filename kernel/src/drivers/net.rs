use crate::sync::IrqSafeSpinlock;
use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use lix_net::{
    decide_route, is_broadcast, subnet_broadcast, ArpPacket, EthernetFrame, Ipv4Address,
    Ipv4Packet, MacAddress, Route, UdpDatagram,
};
pub trait NetworkDevice: Send + Sync {
    fn mac_address(&self) -> MacAddress;
    fn transmit(&self, frame: &[u8]) -> Result<(), NetworkError>;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkError {
    NoDevice,
    InvalidFrame,
    FrameTooLarge,
    QueueFull,
    PortInUse,
    AddressPending,
    DeviceFault,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ipv4Config {
    pub address: Ipv4Address,
    pub subnet_mask: Option<Ipv4Address>,
    pub gateway: Option<Ipv4Address>,
    pub dns_server: Option<Ipv4Address>,
}
static DEVICES: IrqSafeSpinlock<Vec<&'static dyn NetworkDevice>> = IrqSafeSpinlock::new(Vec::new());
static RECEIVED_FRAMES: AtomicUsize = AtomicUsize::new(0);
static IPV4_CONFIG: IrqSafeSpinlock<Option<Ipv4Config>> = IrqSafeSpinlock::new(None);
const RX_FRAME_CAPACITY: usize = 32;
const MAX_ETHERNET_FRAME: usize = 2048;
struct QueuedFrame {
    length: usize,
    bytes: [u8; MAX_ETHERNET_FRAME],
}
static RX_QUEUE: IrqSafeSpinlock<VecDeque<QueuedFrame>> = IrqSafeSpinlock::new(VecDeque::new());
static DROPPED_FRAMES: AtomicUsize = AtomicUsize::new(0);
pub fn register(device: &'static dyn NetworkDevice) {
    DEVICES.lock().push(device);
    crate::kprintln!(
        "[net] Interfaz Ethernet registrada: {:02x?}",
        device.mac_address().0
    );
}
pub fn device_count() -> usize {
    DEVICES.lock().len()
}
pub fn transmit(frame: &[u8]) -> Result<(), NetworkError> {
    let device = DEVICES
        .lock()
        .first()
        .copied()
        .ok_or(NetworkError::NoDevice)?;
    device.transmit(frame)
}
pub fn receive(frame: &[u8]) -> Result<EthernetFrame<'_>, NetworkError> {
    let parsed = EthernetFrame::parse(frame).ok_or(NetworkError::InvalidFrame)?;
    if frame.len() > MAX_ETHERNET_FRAME {
        return Err(NetworkError::FrameTooLarge);
    }
    let mut queue = RX_QUEUE.lock();
    if queue.len() == RX_FRAME_CAPACITY {
        DROPPED_FRAMES.fetch_add(1, Ordering::Relaxed);
        return Err(NetworkError::QueueFull);
    }
    let mut queued = QueuedFrame {
        length: frame.len(),
        bytes: [0; MAX_ETHERNET_FRAME],
    };
    queued.bytes[..frame.len()].copy_from_slice(frame);
    queue.push_back(queued);
    RECEIVED_FRAMES.fetch_add(1, Ordering::Relaxed);
    Ok(parsed)
}
pub fn received_frames() -> usize {
    RECEIVED_FRAMES.load(Ordering::Relaxed)
}
pub fn dropped_frames() -> usize {
    DROPPED_FRAMES.load(Ordering::Relaxed)
}
pub fn receive_next(output: &mut [u8]) -> Result<Option<usize>, NetworkError> {
    let mut queue = RX_QUEUE.lock();
    let Some(frame) = queue.front() else {
        return Ok(None);
    };
    if output.len() < frame.length {
        return Err(NetworkError::FrameTooLarge);
    }
    let length = frame.length;
    output[..length].copy_from_slice(&frame.bytes[..length]);
    queue.pop_front();
    Ok(Some(length))
}
pub fn set_ipv4_config(config: Ipv4Config) {
    *IPV4_CONFIG.lock() = Some(config);
}
pub fn ipv4_config() -> Option<Ipv4Config> {
    *IPV4_CONFIG.lock()
}
const UDP_QUEUE_CAPACITY: usize = 32;
const UDP_MAX_PAYLOAD: usize = 1472;
#[derive(Clone)]
pub struct UdpMessage {
    pub source: Ipv4Address,
    pub source_port: u16,
    pub length: usize,
    pub bytes: [u8; UDP_MAX_PAYLOAD],
}
struct UdpState {
    port: u16,
    queue: VecDeque<UdpMessage>,
}
pub struct UdpSocket {
    state: IrqSafeSpinlock<UdpState>,
}
impl core::fmt::Debug for UdpSocket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UdpSocket")
            .field("port", &self.state.lock().port)
            .finish()
    }
}
static UDP_SOCKETS: IrqSafeSpinlock<Vec<Weak<UdpSocket>>> = IrqSafeSpinlock::new(Vec::new());
static ARP_CACHE: IrqSafeSpinlock<Vec<(Ipv4Address, MacAddress)>> =
    IrqSafeSpinlock::new(Vec::new());
static NEXT_IP_ID: AtomicU16 = AtomicU16::new(1);
static NEXT_EPHEMERAL_PORT: AtomicU16 = AtomicU16::new(49152);
fn allocate_ephemeral_port() -> u16 {
    let sockets = UDP_SOCKETS.lock();
    loop {
        let mut port = NEXT_EPHEMERAL_PORT.fetch_add(1, Ordering::Relaxed);
        if port < 49152 {
            NEXT_EPHEMERAL_PORT.store(49153, Ordering::Relaxed);
            port = 49152;
        }
        let in_use = sockets
            .iter()
            .filter_map(Weak::upgrade)
            .any(|s| s.state.lock().port == port);
        if !in_use {
            return port;
        }
    }
}
impl UdpSocket {
    pub fn new() -> Arc<Self> {
        let sock = Arc::new(Self {
            state: IrqSafeSpinlock::new(UdpState {
                port: 0,
                queue: VecDeque::new(),
            }),
        });
        let mut sockets = UDP_SOCKETS.lock();
        sockets.retain(|entry| entry.strong_count() > 0);
        sockets.push(Arc::downgrade(&sock));
        sock
    }
    pub fn bind(self: &Arc<Self>, port: u16) -> Result<(), NetworkError> {
        if port == 0 {
            return Err(NetworkError::InvalidFrame);
        }
        let sockets = UDP_SOCKETS.lock();
        for entry in sockets.iter().filter_map(Weak::upgrade) {
            if Arc::ptr_eq(&entry, self) {
                continue;
            }
            if entry.state.lock().port == port {
                return Err(NetworkError::PortInUse);
            }
        }
        let mut state = self.state.lock();
        if state.port != 0 && state.port != port {
            return Err(NetworkError::PortInUse);
        }
        state.port = port;
        Ok(())
    }
    pub fn receive(
        &self,
        output: &mut [u8],
    ) -> Result<Option<(Ipv4Address, u16, usize)>, NetworkError> {
        let mut state = self.state.lock();
        let Some(message) = state.queue.front() else {
            return Ok(None);
        };
        if output.len() < message.length {
            return Err(NetworkError::FrameTooLarge);
        }
        let (source, port, length) = (message.source, message.source_port, message.length);
        output[..length].copy_from_slice(&message.bytes[..length]);
        state.queue.pop_front();
        Ok(Some((source, port, length)))
    }
    pub fn send_to(
        &self,
        destination: Ipv4Address,
        port: u16,
        payload: &[u8],
    ) -> Result<(), NetworkError> {
        if payload.len() > UDP_MAX_PAYLOAD || port == 0 {
            return Err(NetworkError::FrameTooLarge);
        }
        let (source_port, source_ip) = {
            let mut state = self.state.lock();
            if state.port == 0 {
                state.port = allocate_ephemeral_port();
            }
            let config = ipv4_config().ok_or(NetworkError::NoDevice)?;
            (state.port, config.address)
        };
        let device = DEVICES
            .lock()
            .first()
            .copied()
            .ok_or(NetworkError::NoDevice)?;
        let config = ipv4_config().ok_or(NetworkError::NoDevice)?;
        let (route, next_hop) =
            decide_route(source_ip, config.subnet_mask, config.gateway, destination)
                .ok_or(NetworkError::NoDevice)?;
        let mac = if route == Route::Broadcast {
            MacAddress([0xff; 6])
        } else {
            let cached = ARP_CACHE
                .lock()
                .iter()
                .find(|entry| entry.0 == next_hop)
                .map(|entry| entry.1);
            let Some(mac) = cached else {
                let mut request = [0u8; 42];
                if let Some(n) = lix_net::build_arp_request(
                    device.mac_address(),
                    source_ip,
                    next_hop,
                    &mut request,
                ) {
                    device.transmit(&request[..n])?;
                }
                return Err(NetworkError::AddressPending);
            };
            mac
        };
        let mut frame = [0u8; 1514];
        let length = lix_net::build_ipv4_udp_frame(
            device.mac_address(),
            mac,
            source_ip,
            destination,
            source_port,
            port,
            payload,
            NEXT_IP_ID.fetch_add(1, Ordering::Relaxed),
            64,
            &mut frame,
        )
        .ok_or(NetworkError::FrameTooLarge)?;
        device.transmit(&frame[..length])
    }
}
pub fn poll_protocols() {
    let mut bytes = [0u8; MAX_ETHERNET_FRAME];
    loop {
        let length = match receive_next(&mut bytes) {
            Ok(Some(n)) => n,
            _ => break,
        };
        let Some(frame) = EthernetFrame::parse(&bytes[..length]) else {
            continue;
        };
        if frame.ether_type == 0x0806 {
            if let Some(arp) = ArpPacket::parse(frame.payload) {
                if arp.sender_ipv4.0 != [0; 4] {
                    let mut cache = ARP_CACHE.lock();
                    if let Some(entry) = cache.iter_mut().find(|entry| entry.0 == arp.sender_ipv4) {
                        entry.1 = arp.sender_mac;
                    } else {
                        if cache.len() == 64 {
                            cache.remove(0);
                        }
                        cache.push((arp.sender_ipv4, arp.sender_mac));
                    }
                }
                if arp.operation == 1 {
                    if let Some(config) = ipv4_config().filter(|c| c.address == arp.target_ipv4) {
                        if let Some(device) = DEVICES.lock().first().copied() {
                            let mut reply = [0u8; 42];
                            if let Some(n) = lix_net::build_arp_reply(
                                device.mac_address(),
                                config.address,
                                &arp,
                                &mut reply,
                            ) {
                                let _ = device.transmit(&reply[..n]);
                            }
                        }
                    }
                }
            }
            continue;
        }
        if frame.ether_type != 0x0800 {
            continue;
        }
        let Some(ip) = Ipv4Packet::parse(frame.payload) else {
            continue;
        };
        if ip.protocol != 17 {
            continue;
        }
        let matches_dest = ipv4_config().is_some_and(|c| {
            c.address == ip.destination
                || is_broadcast(ip.destination)
                || c.subnet_mask
                    .is_some_and(|m| subnet_broadcast(c.address, m) == ip.destination)
        });
        if !matches_dest {
            continue;
        }
        let Some(udp) = UdpDatagram::parse_ipv4(ip.payload, ip.source, ip.destination) else {
            continue;
        };
        let sockets: Vec<Arc<UdpSocket>> = UDP_SOCKETS
            .lock()
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        for socket in sockets {
            let mut state = socket.state.lock();
            if state.port != udp.destination_port
                || state.queue.len() >= UDP_QUEUE_CAPACITY
                || udp.payload.len() > UDP_MAX_PAYLOAD
            {
                continue;
            }
            let mut message = UdpMessage {
                source: ip.source,
                source_port: udp.source_port,
                length: udp.payload.len(),
                bytes: [0; UDP_MAX_PAYLOAD],
            };
            message.bytes[..udp.payload.len()].copy_from_slice(udp.payload);
            state.queue.push_back(message);
            break;
        }
    }
}
