use crate::checksum::{ipv4_pseudo_header, ones_complement_sum};
use crate::ethernet::MacAddress;
use crate::ipv4::Ipv4Address;
const TCP_PROTOCOL: u8 = 6;
pub const TCP_FLAG_FIN: u16 = 0x001;
pub const TCP_FLAG_SYN: u16 = 0x002;
pub const TCP_FLAG_RST: u16 = 0x004;
pub const TCP_FLAG_PSH: u16 = 0x008;
pub const TCP_FLAG_ACK: u16 = 0x010;
pub const TCP_FLAG_URG: u16 = 0x020;
pub const TCP_HEADER_LENGTH: usize = 20;
pub const TCP_MSS: usize = 1460;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TcpSegment<'a> {
    pub source_port: u16,
    pub destination_port: u16,
    pub sequence: u32,
    pub acknowledgment: u32,
    pub flags: u16,
    pub window: u16,
    pub payload: &'a [u8],
}
impl<'a> TcpSegment<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < TCP_HEADER_LENGTH || bytes.len() > u16::MAX as usize {
            return None;
        }
        let header_len = ((bytes[12] >> 4) as usize).checked_mul(4)?;
        if header_len < TCP_HEADER_LENGTH || header_len > bytes.len() {
            return None;
        }
        Some(Self {
            source_port: u16::from_be_bytes([bytes[0], bytes[1]]),
            destination_port: u16::from_be_bytes([bytes[2], bytes[3]]),
            sequence: u32::from_be_bytes(bytes[4..8].try_into().ok()?),
            acknowledgment: u32::from_be_bytes(bytes[8..12].try_into().ok()?),
            flags: u16::from_be_bytes([bytes[12] & 1, bytes[13]]),
            window: u16::from_be_bytes([bytes[14], bytes[15]]),
            payload: &bytes[header_len..],
        })
    }
    pub fn parse_ipv4(
        bytes: &'a [u8],
        source: Ipv4Address,
        destination: Ipv4Address,
    ) -> Option<Self> {
        let segment = Self::parse(bytes)?;
        let pseudo = ipv4_pseudo_header(source, destination, TCP_PROTOCOL, bytes.len() as u16);
        if ones_complement_sum([pseudo.as_slice(), bytes]) != 0 {
            return None;
        }
        Some(segment)
    }
}
pub fn build_tcp_segment(
    source: Ipv4Address,
    destination: Ipv4Address,
    source_port: u16,
    destination_port: u16,
    sequence: u32,
    acknowledgment: u32,
    flags: u16,
    window: u16,
    payload: &[u8],
    output: &mut [u8],
) -> Option<usize> {
    let length = TCP_HEADER_LENGTH.checked_add(payload.len())?;
    if length > u16::MAX as usize
        || output.len() < length
        || flags & TCP_FLAG_SYN != 0 && !payload.is_empty()
    {
        return None;
    }
    output[..length].fill(0);
    output[0..2].copy_from_slice(&source_port.to_be_bytes());
    output[2..4].copy_from_slice(&destination_port.to_be_bytes());
    output[4..8].copy_from_slice(&sequence.to_be_bytes());
    output[8..12].copy_from_slice(&acknowledgment.to_be_bytes());
    output[12] = (((TCP_HEADER_LENGTH / 4) as u8) << 4) | ((flags >> 8) as u8 & 1);
    output[13] = flags as u8;
    output[14..16].copy_from_slice(&window.to_be_bytes());
    output[TCP_HEADER_LENGTH..length].copy_from_slice(payload);
    let checksum = tcp_ipv4_checksum(source, destination, &output[..length]);
    output[16..18].copy_from_slice(&(if checksum == 0 { 0xffff } else { checksum }).to_be_bytes());
    Some(length)
}
pub fn build_ipv4_tcp_frame(
    source_mac: MacAddress,
    destination_mac: MacAddress,
    source_ip: Ipv4Address,
    destination_ip: Ipv4Address,
    source_port: u16,
    destination_port: u16,
    sequence: u32,
    acknowledgment: u32,
    flags: u16,
    window: u16,
    payload: &[u8],
    identification: u16,
    ttl: u8,
    output: &mut [u8],
) -> Option<usize> {
    let ip_length = 20usize
        .checked_add(TCP_HEADER_LENGTH)?
        .checked_add(payload.len())?;
    let frame_length = 14usize.checked_add(ip_length)?;
    if ttl == 0 || ip_length > u16::MAX as usize || output.len() < frame_length {
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
    ip[9] = TCP_PROTOCOL;
    ip[12..16].copy_from_slice(&source_ip.0);
    ip[16..20].copy_from_slice(&destination_ip.0);
    let header_checksum = ones_complement_sum([ip as &[u8]]);
    ip[10..12].copy_from_slice(&header_checksum.to_be_bytes());
    let written = build_tcp_segment(
        source_ip,
        destination_ip,
        source_port,
        destination_port,
        sequence,
        acknowledgment,
        flags,
        window,
        payload,
        &mut output[34..frame_length],
    )?;
    if written != frame_length - 34 {
        return None;
    }
    Some(frame_length)
}
fn tcp_ipv4_checksum(source: Ipv4Address, destination: Ipv4Address, tcp: &[u8]) -> u16 {
    let pseudo = ipv4_pseudo_header(source, destination, TCP_PROTOCOL, tcp.len() as u16);
    ones_complement_sum([pseudo.as_slice(), tcp])
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pair() -> (Ipv4Address, Ipv4Address) {
        (Ipv4Address([10, 0, 2, 15]), Ipv4Address([10, 0, 2, 2]))
    }
    #[test]
    fn builds_segment_that_parses_back_with_valid_checksum() {
        let (source, destination) = pair();
        let mut segment = [0u8; 64];
        let length = build_tcp_segment(
            source,
            destination,
            1234,
            80,
            100,
            200,
            TCP_FLAG_SYN | TCP_FLAG_ACK,
            4096,
            &[],
            &mut segment,
        )
        .unwrap();
        assert_eq!(length, TCP_HEADER_LENGTH);
        let parsed = TcpSegment::parse_ipv4(&segment[..length], source, destination).unwrap();
        assert_eq!(parsed.source_port, 1234);
        assert_eq!(parsed.destination_port, 80);
        assert_eq!(parsed.sequence, 100);
        assert_eq!(parsed.acknowledgment, 200);
        assert_eq!(parsed.flags, TCP_FLAG_SYN | TCP_FLAG_ACK);
        assert_eq!(parsed.window, 4096);
        assert!(parsed.payload.is_empty());
    }
    #[test]
    fn rejects_segment_with_corrupted_checksum() {
        let (source, destination) = pair();
        let mut segment = [0u8; 64];
        let length = build_tcp_segment(
            source,
            destination,
            1,
            2,
            3,
            4,
            TCP_FLAG_ACK,
            100,
            &[9, 9],
            &mut segment,
        )
        .unwrap();
        segment[length - 1] ^= 0xff;
        assert!(TcpSegment::parse_ipv4(&segment[..length], source, destination).is_none());
    }
    #[test]
    fn rejects_segment_with_wrong_pseudo_header() {
        let (source, destination) = pair();
        let mut segment = [0u8; 64];
        let length = build_tcp_segment(
            source,
            destination,
            1,
            2,
            3,
            4,
            TCP_FLAG_ACK,
            100,
            &[],
            &mut segment,
        )
        .unwrap();
        assert!(
            TcpSegment::parse_ipv4(&segment[..length], source, Ipv4Address([10, 0, 2, 99]))
                .is_none()
        );
    }
    #[test]
    fn rejects_short_headers_and_oversized_data_offset() {
        assert!(TcpSegment::parse(&[0u8; 19]).is_none());
        let mut segment = [0u8; 24];
        segment[12] = 15 << 4;
        assert!(TcpSegment::parse(&segment).is_none());
    }
    #[test]
    fn builds_frame_with_valid_ipv4_and_tcp_checksums() {
        let (source, destination) = pair();
        let mac = MacAddress([2, 0, 0, 0, 0, 1]);
        let peer = MacAddress([2, 0, 0, 0, 0, 2]);
        let mut frame = [0u8; 128];
        let length = build_ipv4_tcp_frame(
            mac,
            peer,
            source,
            destination,
            5000,
            443,
            7,
            8,
            TCP_FLAG_PSH | TCP_FLAG_ACK,
            2048,
            &[1, 2, 3],
            42,
            64,
            &mut frame,
        )
        .unwrap();
        assert_eq!(length, 14 + 20 + TCP_HEADER_LENGTH + 3);
        assert_eq!(ones_complement_sum([&frame[14..34]]), 0);
        let parsed = TcpSegment::parse_ipv4(&frame[34..length], source, destination).unwrap();
        assert_eq!(parsed.payload, &[1, 2, 3]);
        assert_eq!(parsed.flags, TCP_FLAG_PSH | TCP_FLAG_ACK);
        assert!(
            build_ipv4_tcp_frame(
                mac,
                peer,
                source,
                destination,
                5000,
                443,
                7,
                8,
                TCP_FLAG_ACK,
                2048,
                &[],
                42,
                64,
                &mut [0u8; 40]
            )
            .is_none()
        );
    }
}
