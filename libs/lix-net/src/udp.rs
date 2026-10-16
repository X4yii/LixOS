use crate::checksum::{ipv4_pseudo_header, ones_complement_sum};
use crate::ethernet::MacAddress;
use crate::ipv4::Ipv4Address;
const UDP_PROTOCOL: u8 = 17;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UdpDatagram<'a> {
    pub source_port: u16,
    pub destination_port: u16,
    pub payload: &'a [u8],
}
impl<'a> UdpDatagram<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < 8 {
            return None;
        }
        let length = u16::from_be_bytes(bytes[4..6].try_into().ok()?) as usize;
        if length < 8 || length > bytes.len() {
            return None;
        }
        Some(Self {
            source_port: u16::from_be_bytes(bytes[0..2].try_into().ok()?),
            destination_port: u16::from_be_bytes(bytes[2..4].try_into().ok()?),
            payload: &bytes[8..length],
        })
    }
    pub fn parse_ipv4(
        bytes: &'a [u8],
        source: Ipv4Address,
        destination: Ipv4Address,
    ) -> Option<Self> {
        let datagram = Self::parse(bytes)?;
        let length = u16::from_be_bytes(bytes[4..6].try_into().ok()?) as usize;
        if length != bytes.len() {
            return None;
        }
        let checksum = u16::from_be_bytes(bytes[6..8].try_into().ok()?);
        if checksum != 0 && udp_ipv4_checksum(source, destination, bytes) != 0 {
            return None;
        }
        Some(datagram)
    }
}
fn udp_ipv4_checksum(source: Ipv4Address, destination: Ipv4Address, udp: &[u8]) -> u16 {
    let pseudo = ipv4_pseudo_header(source, destination, UDP_PROTOCOL, udp.len() as u16);
    ones_complement_sum([pseudo.as_slice(), udp])
}
pub fn build_ipv4_udp_frame(
    source_mac: MacAddress,
    destination_mac: MacAddress,
    source_ip: Ipv4Address,
    destination_ip: Ipv4Address,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
    identification: u16,
    ttl: u8,
    output: &mut [u8],
) -> Option<usize> {
    let udp_length = 8usize.checked_add(payload.len())?;
    let ip_length = 20usize.checked_add(udp_length)?;
    let frame_length = 14usize.checked_add(ip_length)?;
    if ttl == 0 || udp_length > u16::MAX as usize || ip_length > u16::MAX as usize {
        return None;
    }
    if output.len() < frame_length {
        return None;
    }
    output[..frame_length].fill(0);
    output[..6].copy_from_slice(&destination_mac.0);
    output[6..12].copy_from_slice(&source_mac.0);
    output[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    let ip = &mut output[14..34];
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&(ip_length as u16).to_be_bytes());
    ip[4..6].copy_from_slice(&identification.to_be_bytes());
    ip[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    ip[8] = ttl;
    ip[9] = UDP_PROTOCOL;
    ip[12..16].copy_from_slice(&source_ip.0);
    ip[16..20].copy_from_slice(&destination_ip.0);
    let header_checksum = ones_complement_sum([ip as &[u8]]);
    ip[10..12].copy_from_slice(&header_checksum.to_be_bytes());
    let udp = &mut output[34..frame_length];
    udp[0..2].copy_from_slice(&source_port.to_be_bytes());
    udp[2..4].copy_from_slice(&destination_port.to_be_bytes());
    udp[4..6].copy_from_slice(&(udp_length as u16).to_be_bytes());
    udp[8..].copy_from_slice(payload);
    let checksum = udp_ipv4_checksum(source_ip, destination_ip, udp);
    udp[6..8].copy_from_slice(&(if checksum == 0 { 0xffff } else { checksum }).to_be_bytes());
    Some(frame_length)
}
