use crate::checksum::ones_complement_sum;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ipv4Address(pub [u8; 4]);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ipv4Packet<'a> {
    pub source: Ipv4Address,
    pub destination: Ipv4Address,
    pub protocol: u8,
    pub payload: &'a [u8],
}
impl<'a> Ipv4Packet<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < 20 || bytes[0] >> 4 != 4 {
            return None;
        }
        let header_len = ((bytes[0] & 0x0f) as usize).checked_mul(4)?;
        if header_len < 20 || header_len > bytes.len() {
            return None;
        }
        let total_len = u16::from_be_bytes(bytes[2..4].try_into().ok()?) as usize;
        if total_len < header_len || total_len > bytes.len() {
            return None;
        }
        if ones_complement_sum([&bytes[..header_len]]) != 0 {
            return None;
        }
        Some(Self {
            source: Ipv4Address(bytes[12..16].try_into().ok()?),
            destination: Ipv4Address(bytes[16..20].try_into().ok()?),
            protocol: bytes[9],
            payload: &bytes[header_len..total_len],
        })
    }
}
