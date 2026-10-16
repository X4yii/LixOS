#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MacAddress(pub [u8; 6]);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EthernetFrame<'a> {
    pub destination: MacAddress,
    pub source: MacAddress,
    pub ether_type: u16,
    pub payload: &'a [u8],
}
impl<'a> EthernetFrame<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < 14 {
            return None;
        }
        Some(Self {
            destination: MacAddress(bytes[0..6].try_into().ok()?),
            source: MacAddress(bytes[6..12].try_into().ok()?),
            ether_type: u16::from_be_bytes(bytes[12..14].try_into().ok()?),
            payload: &bytes[14..],
        })
    }
}
