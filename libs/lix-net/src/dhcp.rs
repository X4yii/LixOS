use crate::checksum::ones_complement_sum;
use crate::ethernet::MacAddress;
use crate::ipv4::Ipv4Address;
const DHCP_MAGIC: [u8; 4] = [99, 130, 83, 99];
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DhcpMessageType {
    Discover = 1,
    Offer = 2,
    Request = 3,
    Decline = 4,
    Ack = 5,
    Nak = 6,
    Release = 7,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpMessage<'a> {
    pub operation: u8,
    pub transaction_id: u32,
    pub offered_address: Ipv4Address,
    pub client_mac: MacAddress,
    pub message_type: Option<DhcpMessageType>,
    pub server_id: Option<Ipv4Address>,
    pub subnet_mask: Option<Ipv4Address>,
    pub router: Option<Ipv4Address>,
    pub dns_server: Option<Ipv4Address>,
    pub lease_time: Option<u32>,
    pub renewal_time: Option<u32>,
    pub rebinding_time: Option<u32>,
    pub options: &'a [u8],
}
impl<'a> DhcpMessage<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < 240 || bytes[236..240] != DHCP_MAGIC {
            return None;
        }
        if bytes[2] < 6 {
            return None;
        }
        let options = &bytes[240..];
        let mut message_type = None;
        let mut server_id = None;
        let mut subnet_mask = None;
        let mut router = None;
        let mut dns_server = None;
        let mut lease_time = None;
        let mut renewal_time = None;
        let mut rebinding_time = None;
        let mut cursor = 0usize;
        let mut ended = false;
        while cursor < options.len() {
            let tag = options[cursor];
            cursor += 1;
            match tag {
                0 => continue,
                255 => {
                    ended = true;
                    break;
                }
                _ => {
                    let length = *options.get(cursor)? as usize;
                    cursor += 1;
                    let end = cursor.checked_add(length)?;
                    let value = options.get(cursor..end)?;
                    if tag == 53 {
                        if length != 1 {
                            return None;
                        }
                        message_type = Some(match value[0] {
                            1 => DhcpMessageType::Discover,
                            2 => DhcpMessageType::Offer,
                            3 => DhcpMessageType::Request,
                            4 => DhcpMessageType::Decline,
                            5 => DhcpMessageType::Ack,
                            6 => DhcpMessageType::Nak,
                            7 => DhcpMessageType::Release,
                            _ => return None,
                        });
                    } else if tag == 54 {
                        if length != 4 {
                            return None;
                        }
                        server_id = Some(Ipv4Address(value.try_into().ok()?));
                    } else if tag == 1 {
                        if length != 4 {
                            return None;
                        }
                        subnet_mask = Some(Ipv4Address(value.try_into().ok()?));
                    } else if tag == 3 || tag == 6 {
                        if length < 4 || !length.is_multiple_of(4) {
                            return None;
                        }
                        let address = Ipv4Address(value[..4].try_into().ok()?);
                        if tag == 3 {
                            router = Some(address);
                        } else {
                            dns_server = Some(address);
                        }
                    } else if tag == 51 || tag == 58 || tag == 59 {
                        if length != 4 {
                            return None;
                        }
                        let seconds = u32::from_be_bytes(value.try_into().ok()?);
                        if tag == 51 {
                            lease_time = Some(seconds);
                        } else if tag == 58 {
                            renewal_time = Some(seconds);
                        } else {
                            rebinding_time = Some(seconds);
                        }
                    }
                    cursor = end;
                }
            }
        }
        if !ended {
            return None;
        }
        Some(Self {
            operation: bytes[0],
            transaction_id: u32::from_be_bytes(bytes[4..8].try_into().ok()?),
            offered_address: Ipv4Address(bytes[16..20].try_into().ok()?),
            client_mac: MacAddress(bytes[28..34].try_into().ok()?),
            message_type,
            server_id,
            subnet_mask,
            router,
            dns_server,
            lease_time,
            renewal_time,
            rebinding_time,
            options,
        })
    }
}
pub fn build_dhcp_discover(xid: u32, mac: MacAddress, output: &mut [u8]) -> Option<usize> {
    const MIN_LENGTH: usize = 259;
    if output.len() < MIN_LENGTH {
        return None;
    }
    output[..MIN_LENGTH].fill(0);
    output[0] = 1;
    output[1] = 1;
    output[2] = 6;
    output[4..8].copy_from_slice(&xid.to_be_bytes());
    output[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
    output[28..34].copy_from_slice(&mac.0);
    output[236..240].copy_from_slice(&DHCP_MAGIC);
    let options = &mut output[240..];
    options[..19].copy_from_slice(&[
        53, 1, 1, 61, 7, 1, mac.0[0], mac.0[1], mac.0[2], mac.0[3], mac.0[4], mac.0[5], 55, 4, 1,
        3, 6, 15, 255,
    ]);
    Some(259)
}
pub fn build_dhcp_discover_frame(xid: u32, mac: MacAddress, output: &mut [u8]) -> Option<usize> {
    const DHCP_LENGTH: usize = 259;
    const FRAME_LENGTH: usize = 14 + 20 + 8 + DHCP_LENGTH;
    if output.len() < FRAME_LENGTH {
        return None;
    }
    output[..FRAME_LENGTH].fill(0);
    output[..6].fill(0xff);
    output[6..12].copy_from_slice(&mac.0);
    output[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    let ip = &mut output[14..34];
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&((20 + 8 + DHCP_LENGTH) as u16).to_be_bytes());
    ip[8] = 64;
    ip[9] = 17;
    ip[16..20].copy_from_slice(&[255, 255, 255, 255]);
    let checksum = ones_complement_sum([ip as &[u8]]);
    ip[10..12].copy_from_slice(&checksum.to_be_bytes());
    let udp = &mut output[34..42];
    udp[0..2].copy_from_slice(&68u16.to_be_bytes());
    udp[2..4].copy_from_slice(&67u16.to_be_bytes());
    udp[4..6].copy_from_slice(&((8 + DHCP_LENGTH) as u16).to_be_bytes());
    let written = build_dhcp_discover(xid, mac, &mut output[42..FRAME_LENGTH])?;
    if written != DHCP_LENGTH {
        return None;
    }
    Some(FRAME_LENGTH)
}
pub fn build_dhcp_request_frame(
    xid: u32,
    mac: MacAddress,
    requested_address: Ipv4Address,
    server_id: Ipv4Address,
    output: &mut [u8],
) -> Option<usize> {
    const DHCP_LENGTH: usize = 262;
    const FRAME_LENGTH: usize = 14 + 20 + 8 + DHCP_LENGTH;
    if output.len() < FRAME_LENGTH {
        return None;
    }
    output[..FRAME_LENGTH].fill(0);
    output[..6].fill(0xff);
    output[6..12].copy_from_slice(&mac.0);
    output[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    let ip = &mut output[14..34];
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&((20 + 8 + DHCP_LENGTH) as u16).to_be_bytes());
    ip[8] = 64;
    ip[9] = 17;
    ip[16..20].copy_from_slice(&[255; 4]);
    let checksum = ones_complement_sum([ip as &[u8]]);
    ip[10..12].copy_from_slice(&checksum.to_be_bytes());
    let udp = &mut output[34..42];
    udp[0..2].copy_from_slice(&68u16.to_be_bytes());
    udp[2..4].copy_from_slice(&67u16.to_be_bytes());
    udp[4..6].copy_from_slice(&((8 + DHCP_LENGTH) as u16).to_be_bytes());
    let dhcp = &mut output[42..FRAME_LENGTH];
    dhcp[0] = 1;
    dhcp[1] = 1;
    dhcp[2] = 6;
    dhcp[4..8].copy_from_slice(&xid.to_be_bytes());
    dhcp[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
    dhcp[28..34].copy_from_slice(&mac.0);
    dhcp[236..240].copy_from_slice(&DHCP_MAGIC);
    dhcp[240..].copy_from_slice(&[
        53,
        1,
        3,
        50,
        4,
        requested_address.0[0],
        requested_address.0[1],
        requested_address.0[2],
        requested_address.0[3],
        54,
        4,
        server_id.0[0],
        server_id.0[1],
        server_id.0[2],
        server_id.0[3],
        55,
        4,
        1,
        3,
        6,
        15,
        255,
    ]);
    Some(FRAME_LENGTH)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_lease_and_renewal_options() {
        let mac = MacAddress([2, 0, 0, 0, 0, 1]);
        let mut ack = [0u8; 262];
        ack[0] = 2;
        ack[1] = 1;
        ack[2] = 6;
        ack[4..8].copy_from_slice(&0x1111_2222u32.to_be_bytes());
        ack[16..20].copy_from_slice(&[10, 0, 2, 15]);
        ack[28..34].copy_from_slice(&mac.0);
        ack[236..240].copy_from_slice(&[99, 130, 83, 99]);
        ack[240..].copy_from_slice(&[
            53, 1, 5, 51, 4, 0, 0, 0x0e, 0x10, 58, 4, 0, 0, 0x07, 0x08, 59, 4, 0, 0, 0x0c, 0x10,
            255,
        ]);
        let parsed = DhcpMessage::parse(&ack).unwrap();
        assert_eq!(parsed.message_type, Some(DhcpMessageType::Ack));
        assert_eq!(parsed.lease_time, Some(3600));
        assert_eq!(parsed.renewal_time, Some(1800));
        assert_eq!(parsed.rebinding_time, Some(3088));
    }
    #[test]
    fn rejects_malformed_lease_option_length() {
        let mut packet = [0u8; 247];
        packet[1] = 1;
        packet[2] = 6;
        packet[236..240].copy_from_slice(&[99, 130, 83, 99]);
        packet[240..].copy_from_slice(&[53, 1, 5, 51, 2, 0, 255]);
        assert!(DhcpMessage::parse(&packet).is_none());
    }
}
