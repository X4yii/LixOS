use crate::ipv4::Ipv4Address;
pub const MAX_DNS_NAME: usize = 255;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DnsAResponse {
    pub transaction_id: u16,
    pub address: Ipv4Address,
    pub ttl: u32,
}
pub fn valid_dns_name(name: &str) -> bool {
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty() || name.len() > 253 {
        return false;
    }
    let mut labels = 0usize;
    for label in name.split('.') {
        if label.is_empty() || label.len() > 63 || !label.is_ascii() {
            return false;
        }
        for byte in label.bytes() {
            let valid = byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_';
            if !valid {
                return false;
            }
        }
        let edge_start = label.as_bytes()[0];
        let edge_end = label.as_bytes()[label.len() - 1];
        let edge_valid = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
        if !edge_valid(edge_start) || !edge_valid(edge_end) {
            return false;
        }
        labels += 1;
    }
    labels > 0
}
pub fn build_dns_a_query(name: &str, transaction_id: u16, output: &mut [u8]) -> Option<usize> {
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty() || output.len() < 17 {
        return None;
    }
    output[..12].fill(0);
    output[0..2].copy_from_slice(&transaction_id.to_be_bytes());
    output[2..4].copy_from_slice(&0x0100u16.to_be_bytes());
    output[4..6].copy_from_slice(&1u16.to_be_bytes());
    let mut cursor = 12;
    for label in name.split('.') {
        if label.is_empty()
            || label.len() > 63
            || !label.is_ascii()
            || cursor + label.len() + 1 >= output.len()
        {
            return None;
        }
        output[cursor] = label.len() as u8;
        cursor += 1;
        output[cursor..cursor + label.len()].copy_from_slice(label.as_bytes());
        cursor += label.len();
    }
    if cursor + 5 > output.len() {
        return None;
    }
    output[cursor] = 0;
    cursor += 1;
    output[cursor..cursor + 4].copy_from_slice(&[0, 1, 0, 1]);
    Some(cursor + 4)
}
fn dns_skip_name(bytes: &[u8], cursor: &mut usize) -> Option<()> {
    let mut position = *cursor;
    let mut resume = None;
    let mut jumps = 0;
    loop {
        let length = *bytes.get(position)?;
        if length & 0xc0 == 0xc0 {
            let second = *bytes.get(position + 1)?;
            let target = (((length as usize & 0x3f) << 8) | second as usize);
            if target >= bytes.len() || jumps >= 16 {
                return None;
            }
            if resume.is_none() {
                resume = Some(position + 2);
            }
            position = target;
            jumps += 1;
        } else if length == 0 {
            *cursor = resume.unwrap_or(position + 1);
            return Some(());
        } else if length & 0xc0 != 0 || length > 63 {
            return None;
        } else {
            position = position.checked_add(1 + length as usize)?;
            if position > bytes.len() {
                return None;
            }
        }
    }
}
pub fn parse_dns_a_response(bytes: &[u8]) -> Option<DnsAResponse> {
    if bytes.len() < 12
        || u16::from_be_bytes([bytes[2], bytes[3]]) & 0x8000 == 0
        || u16::from_be_bytes([bytes[2], bytes[3]]) & 0x000f != 0
    {
        return None;
    }
    let transaction_id = u16::from_be_bytes([bytes[0], bytes[1]]);
    let questions = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let answers = u16::from_be_bytes([bytes[6], bytes[7]]) as usize;
    let mut cursor = 12;
    for _ in 0..questions {
        dns_skip_name(bytes, &mut cursor)?;
        cursor = cursor.checked_add(4)?;
        if cursor > bytes.len() {
            return None;
        }
    }
    for _ in 0..answers {
        dns_skip_name(bytes, &mut cursor)?;
        let fixed_end = cursor.checked_add(10)?;
        let fixed = bytes.get(cursor..fixed_end)?;
        let record_type = u16::from_be_bytes([fixed[0], fixed[1]]);
        let class = u16::from_be_bytes([fixed[2], fixed[3]]);
        let ttl = u32::from_be_bytes(fixed[4..8].try_into().ok()?);
        let data_len = u16::from_be_bytes([fixed[8], fixed[9]]) as usize;
        cursor = fixed_end;
        let data = bytes.get(cursor..cursor.checked_add(data_len)?)?;
        if record_type == 1 && class == 1 && data_len == 4 {
            return Some(DnsAResponse {
                transaction_id,
                address: Ipv4Address(data.try_into().ok()?),
                ttl,
            });
        }
        cursor += data_len;
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_well_formed_names() {
        assert!(valid_dns_name("example.com"));
        assert!(valid_dns_name("a-b_c.example.com."));
        assert!(valid_dns_name("xn--liux-os.example"));
        assert!(valid_dns_name("localhost"));
    }
    #[test]
    fn rejects_malformed_names() {
        assert!(!valid_dns_name(""));
        assert!(!valid_dns_name("."));
        assert!(!valid_dns_name("foo..bar"));
        assert!(!valid_dns_name("-bad.example"));
        assert!(!valid_dns_name("bad!.example"));
        assert!(!valid_dns_name("no spaces.example"));
        assert!(!valid_dns_name("trevor.ñ.com"));
        let long_label = "a".repeat(64);
        assert!(!valid_dns_name(&alloc::format!("{long_label}.example")));
        let long_name = alloc::format!("{}.example", "a".repeat(63));
        assert!(long_name.len() <= 253);
        assert!(!valid_dns_name(&alloc::format!("{long_name}{long_name}")));
    }
    #[test]
    fn parses_compressed_a_response() {
        let mut query = [0u8; 512];
        let query_len = build_dns_a_query("example.com", 0x1234, &mut query).unwrap();
        let mut response = [0u8; 512];
        response[..query_len].copy_from_slice(&query[..query_len]);
        response[2] = 0x81;
        response[3] = 0x80;
        response[6] = 0;
        response[7] = 1;
        let mut cursor = query_len;
        response[cursor..cursor + 2].copy_from_slice(&[0xc0, 0x0c]);
        cursor += 2;
        response[cursor..cursor + 2].copy_from_slice(&[0, 1]);
        response[cursor + 2..cursor + 4].copy_from_slice(&[0, 1]);
        response[cursor + 4..cursor + 8].copy_from_slice(&60u32.to_be_bytes());
        response[cursor + 8..cursor + 10].copy_from_slice(&[0, 4]);
        cursor += 10;
        response[cursor..cursor + 4].copy_from_slice(&[93, 184, 216, 34]);
        cursor += 4;
        let parsed = parse_dns_a_response(&response[..cursor]).unwrap();
        assert_eq!(parsed.transaction_id, 0x1234);
        assert_eq!(parsed.address, Ipv4Address([93, 184, 216, 34]));
        assert_eq!(parsed.ttl, 60);
    }
    #[test]
    fn rejects_error_responses_and_truncated_answers() {
        let mut response = [0u8; 64];
        response[0] = 0x12;
        response[1] = 0x34;
        response[2] = 0x80;
        response[3] = 0x03;
        response[5] = 1;
        assert!(parse_dns_a_response(&response[..12]).is_none());
        let mut query = [0u8; 64];
        let query_len = build_dns_a_query("example.com", 1, &mut query).unwrap();
        assert!(parse_dns_a_response(&query[..query_len]).is_none());
    }
    #[test]
    fn rejects_pointer_loops_in_names() {
        let mut response = [0u8; 32];
        response[2] = 0x81;
        response[3] = 0x80;
        response[5] = 1;
        response[7] = 1;
        response[12] = 0xc0;
        response[13] = 0x0c;
        assert!(parse_dns_a_response(&response[..14]).is_none());
    }
}
