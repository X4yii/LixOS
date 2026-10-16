use crate::ipv4::Ipv4Address;
pub fn ones_complement_sum<I, B>(parts: I) -> u16
where
    I: IntoIterator<Item = B>,
    B: AsRef<[u8]>,
{
    let mut sum = 0u32;
    for part in parts {
        let part = part.as_ref();
        let mut chunks = part.chunks_exact(2);
        for pair in &mut chunks {
            sum += u32::from(u16::from_be_bytes([pair[0], pair[1]]));
        }
        if let Some(&last) = chunks.remainder().first() {
            sum += u32::from(last) << 8;
        }
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}
pub(crate) fn ipv4_pseudo_header(
    source: Ipv4Address,
    destination: Ipv4Address,
    protocol: u8,
    length: u16,
) -> [u8; 12] {
    let mut header = [0u8; 12];
    header[..4].copy_from_slice(&source.0);
    header[4..8].copy_from_slice(&destination.0);
    header[8] = protocol;
    header[10..12].copy_from_slice(&length.to_be_bytes());
    header
}
