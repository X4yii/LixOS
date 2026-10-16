use crate::ethernet::MacAddress;
use crate::ipv4::Ipv4Address;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArpPacket {
    pub operation: u16,
    pub sender_mac: MacAddress,
    pub sender_ipv4: Ipv4Address,
    pub target_mac: MacAddress,
    pub target_ipv4: Ipv4Address,
}
impl ArpPacket {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 28
            || u16::from_be_bytes([bytes[0], bytes[1]]) != 1
            || u16::from_be_bytes([bytes[2], bytes[3]]) != 0x0800
            || bytes[4] != 6
            || bytes[5] != 4
        {
            return None;
        }
        let operation = u16::from_be_bytes([bytes[6], bytes[7]]);
        if operation != 1 && operation != 2 {
            return None;
        }
        Some(Self {
            operation,
            sender_mac: MacAddress(bytes[8..14].try_into().ok()?),
            sender_ipv4: Ipv4Address(bytes[14..18].try_into().ok()?),
            target_mac: MacAddress(bytes[18..24].try_into().ok()?),
            target_ipv4: Ipv4Address(bytes[24..28].try_into().ok()?),
        })
    }
}
pub fn build_arp_request(
    mac: MacAddress,
    address: Ipv4Address,
    target: Ipv4Address,
    output: &mut [u8],
) -> Option<usize> {
    if output.len() < 42 {
        return None;
    }
    output[..42].fill(0);
    output[..6].fill(0xff);
    output[6..12].copy_from_slice(&mac.0);
    output[12..14].copy_from_slice(&0x0806u16.to_be_bytes());
    output[14..16].copy_from_slice(&1u16.to_be_bytes());
    output[16..18].copy_from_slice(&0x0800u16.to_be_bytes());
    output[18] = 6;
    output[19] = 4;
    output[20..22].copy_from_slice(&1u16.to_be_bytes());
    output[22..28].copy_from_slice(&mac.0);
    output[28..32].copy_from_slice(&address.0);
    output[38..42].copy_from_slice(&target.0);
    Some(42)
}
pub fn build_arp_reply(
    mac: MacAddress,
    address: Ipv4Address,
    request: &ArpPacket,
    output: &mut [u8],
) -> Option<usize> {
    if request.operation != 1 || output.len() < 42 {
        return None;
    }
    output[..42].fill(0);
    output[..6].copy_from_slice(&request.sender_mac.0);
    output[6..12].copy_from_slice(&mac.0);
    output[12..14].copy_from_slice(&0x0806u16.to_be_bytes());
    output[14..16].copy_from_slice(&1u16.to_be_bytes());
    output[16..18].copy_from_slice(&0x0800u16.to_be_bytes());
    output[18] = 6;
    output[19] = 4;
    output[20..22].copy_from_slice(&2u16.to_be_bytes());
    output[22..28].copy_from_slice(&mac.0);
    output[28..32].copy_from_slice(&address.0);
    output[32..38].copy_from_slice(&request.sender_mac.0);
    output[38..42].copy_from_slice(&request.sender_ipv4.0);
    Some(42)
}
