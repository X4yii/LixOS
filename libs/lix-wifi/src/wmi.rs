use crate::htc::HtcError;
pub const WMI_HEADER_LEN: usize = 4;
pub const WMI_MAX_MESSAGE: usize = 256;
pub const WMI_EVENT_MASK: u16 = 0x1000;
pub const WMI_ECHO_CMDID: u16 = 0x0001;
pub const WMI_GET_FW_VERSION_CMDID: u16 = 0x0003;
pub const WMI_REG_READ_CMDID: u16 = 0x0014;
pub const WMI_REG_WRITE_CMDID: u16 = 0x0015;
pub const WMI_TGT_RDY_EVENTID: u16 = 0x1001;
pub const WMI_REG_READ_RESP_EVENTID: u16 = 0x1014;
pub const WMI_CONTROL_REG: u32 = 0x4020;
pub const FW_VERSION_MAJOR: u32 = 1;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WmiError {
    Truncated,
    InvalidCommand,
    SequenceMismatch,
    UnexpectedResponse,
    CommandFailed,
    Busy,
    Expired,
    BufferTooLarge,
}
impl From<HtcError> for WmiError {
    fn from(error: HtcError) -> Self {
        match error {
            HtcError::PayloadTooLarge => WmiError::BufferTooLarge,
            HtcError::Truncated => WmiError::Truncated,
            _ => WmiError::InvalidCommand,
        }
    }
}
pub fn encode_command(
    command_id: u16,
    sequence: u16,
    body: &[u8],
    out: &mut [u8],
) -> Result<usize, WmiError> {
    let total = WMI_HEADER_LEN
        .checked_add(body.len())
        .ok_or(WmiError::BufferTooLarge)?;
    if total > WMI_MAX_MESSAGE {
        return Err(WmiError::BufferTooLarge);
    }
    if out.len() < total {
        return Err(WmiError::BufferTooLarge);
    }
    out[0..2].copy_from_slice(&command_id.to_be_bytes());
    out[2..4].copy_from_slice(&sequence.to_be_bytes());
    out[WMI_HEADER_LEN..total].copy_from_slice(body);
    Ok(total)
}
pub fn parse_header(payload: &[u8]) -> Result<(u16, u16), WmiError> {
    if payload.len() < WMI_HEADER_LEN {
        return Err(WmiError::Truncated);
    }
    Ok((
        u16::from_be_bytes([payload[0], payload[1]]),
        u16::from_be_bytes([payload[2], payload[3]]),
    ))
}
pub fn is_event(command_id: u16) -> bool {
    command_id & WMI_EVENT_MASK != 0
}
pub fn decode_reg_value(payload: &[u8]) -> Result<u32, WmiError> {
    if payload.len() < WMI_HEADER_LEN + 4 {
        return Err(WmiError::Truncated);
    }
    Ok(u32::from_be_bytes(
        payload[WMI_HEADER_LEN..WMI_HEADER_LEN + 4]
            .try_into()
            .map_err(|_| WmiError::Truncated)?,
    ))
}
pub fn encode_reg_write(address: u32, value: u32, body: &mut [u8]) -> Result<usize, WmiError> {
    if body.len() < 8 {
        return Err(WmiError::BufferTooLarge);
    }
    body[0..4].copy_from_slice(&address.to_be_bytes());
    body[4..8].copy_from_slice(&value.to_be_bytes());
    Ok(8)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WmiEvent {
    TargetReady,
    RegisterValue(u32),
    Unknown { command_id: u16 },
}
pub fn classify(payload: &[u8]) -> Result<WmiEvent, WmiError> {
    let (command_id, _sequence) = parse_header(payload)?;
    if !is_event(command_id) {
        return Err(WmiError::InvalidCommand);
    }
    match command_id {
        WMI_TGT_RDY_EVENTID => Ok(WmiEvent::TargetReady),
        WMI_REG_READ_RESP_EVENTID => Ok(WmiEvent::RegisterValue(decode_reg_value(payload)?)),
        _ => Ok(WmiEvent::Unknown { command_id }),
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WmiCommandKind {
    GetFirmwareVersion,
    ReadRegister,
    WriteRegister,
}
impl WmiCommandKind {
    const fn command_id(self) -> u16 {
        match self {
            WmiCommandKind::GetFirmwareVersion => WMI_GET_FW_VERSION_CMDID,
            WmiCommandKind::ReadRegister => WMI_REG_READ_CMDID,
            WmiCommandKind::WriteRegister => WMI_REG_WRITE_CMDID,
        }
    }
}
pub struct WmiClient {
    sequence: u16,
    pending: Option<(u16, WmiCommandKind)>,
    deadline: u64,
    timeout_ticks: u64,
}
impl WmiClient {
    pub const fn new(timeout_ticks: u64) -> Self {
        Self {
            sequence: 0,
            pending: None,
            deadline: 0,
            timeout_ticks,
        }
    }
    pub const fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub const fn pending_command(&self) -> Option<WmiCommandKind> {
        match self.pending {
            Some((_, kind)) => Some(kind),
            None => None,
        }
    }
    pub fn begin(
        &mut self,
        kind: WmiCommandKind,
        body: &[u8],
        now: u64,
        out: &mut [u8],
    ) -> Result<(u16, usize), WmiError> {
        if self.pending.is_some() {
            return Err(WmiError::Busy);
        }
        let sequence = self.sequence;
        let length = encode_command(kind.command_id(), sequence, body, out)?;
        self.sequence = self.sequence.wrapping_add(1);
        self.pending = Some((sequence, kind));
        self.deadline = now.wrapping_add(self.timeout_ticks);
        Ok((sequence, length))
    }
    pub fn expired(&self, now: u64) -> bool {
        self.pending.is_some() && now >= self.deadline
    }
    pub fn on_reply(&mut self, payload: &[u8]) -> Result<WmiReply, WmiError> {
        let (command_id, sequence) = parse_header(payload)?;
        if let Some((expected, kind)) = self.pending {
            if sequence != expected {
                return Err(WmiError::SequenceMismatch);
            }
            self.pending = None;
            if is_event(command_id) {
                return Ok(WmiReply::Event(classify(payload)?));
            }
            return Ok(WmiReply::Command(kind));
        }
        if is_event(command_id) {
            return Ok(WmiReply::Event(classify(payload)?));
        }
        Err(WmiError::UnexpectedResponse)
    }
    pub fn complete(&mut self) {
        self.pending = None;
    }
    pub fn fail(&mut self) {
        self.pending = None;
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WmiReply {
    Command(WmiCommandKind),
    Event(WmiEvent),
}
pub fn parse_firmware_version(payload: &[u8]) -> Result<(u32, u32, u32, u32), WmiError> {
    if payload.len() < WMI_HEADER_LEN + 16 {
        return Err(WmiError::Truncated);
    }
    let word = |index: usize| -> Result<u32, WmiError> {
        let start = WMI_HEADER_LEN + index * 4;
        Ok(u32::from_be_bytes(
            payload[start..start + 4]
                .try_into()
                .map_err(|_| WmiError::Truncated)?,
        ))
    };
    Ok((word(0)?, word(1)?, word(2)?, word(3)?))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_framing_is_big_endian_header() {
        let mut out = [0u8; WMI_MAX_MESSAGE];
        let (sequence, length) = WmiClient::new(10)
            .begin(WmiCommandKind::GetFirmwareVersion, &[], 0, &mut out)
            .unwrap();
        assert_eq!(sequence, 0);
        assert_eq!(length, 4);
        assert_eq!(&out[..4], &[0x00, 0x03, 0x00, 0x00]);
    }
    #[test]
    fn sequences_increment_and_busy_rejects() {
        let mut client = WmiClient::new(10);
        let mut out = [0u8; WMI_MAX_MESSAGE];
        let (first, _) = client
            .begin(WmiCommandKind::GetFirmwareVersion, &[], 0, &mut out)
            .unwrap();
        assert_eq!(
            client.begin(WmiCommandKind::ReadRegister, &[0; 8], 0, &mut out),
            Err(WmiError::Busy)
        );
        client.complete();
        let (second, _) = client
            .begin(WmiCommandKind::ReadRegister, &[0; 8], 0, &mut out)
            .unwrap();
        assert_eq!(first, 0);
        assert_eq!(second, 1);
    }
    #[test]
    fn replies_are_matched_by_sequence() {
        let mut client = WmiClient::new(10);
        let mut out = [0u8; WMI_MAX_MESSAGE];
        let (_, _) = client
            .begin(WmiCommandKind::ReadRegister, &[0; 8], 0, &mut out)
            .unwrap();
        let mut reply = [0u8; WMI_HEADER_LEN + 4];
        reply[0..2].copy_from_slice(&WMI_REG_READ_RESP_EVENTID.to_be_bytes());
        reply[2..4].copy_from_slice(&0u16.to_be_bytes());
        reply[4..8].copy_from_slice(&0xdeadbeefu32.to_be_bytes());
        assert_eq!(
            client.on_reply(&reply),
            Ok(WmiReply::Event(WmiEvent::RegisterValue(0xdeadbeef)))
        );
        assert!(!client.busy());
    }
    #[test]
    fn wrong_sequence_is_rejected() {
        let mut client = WmiClient::new(10);
        let mut out = [0u8; WMI_MAX_MESSAGE];
        client
            .begin(WmiCommandKind::ReadRegister, &[0; 8], 0, &mut out)
            .unwrap();
        let mut reply = [0u8; WMI_HEADER_LEN + 4];
        reply[0..2].copy_from_slice(&WMI_REG_READ_RESP_EVENTID.to_be_bytes());
        reply[2..4].copy_from_slice(&9u16.to_be_bytes());
        assert_eq!(client.on_reply(&reply), Err(WmiError::SequenceMismatch));
    }
    #[test]
    fn deadlines_expire_pending_commands() {
        let mut client = WmiClient::new(10);
        let mut out = [0u8; WMI_MAX_MESSAGE];
        client
            .begin(WmiCommandKind::GetFirmwareVersion, &[], 100, &mut out)
            .unwrap();
        assert!(!client.expired(109));
        assert!(client.expired(111));
    }
    #[test]
    fn firmware_version_decodes() {
        let mut payload = [0u8; WMI_HEADER_LEN + 16];
        payload[0..2].copy_from_slice(&WMI_GET_FW_VERSION_CMDID.to_be_bytes());
        payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        payload[8..12].copy_from_slice(&9u32.to_be_bytes());
        payload[12..16].copy_from_slice(&2u32.to_be_bytes());
        payload[16..20].copy_from_slice(&7u32.to_be_bytes());
        assert_eq!(parse_firmware_version(&payload), Ok((1, 9, 2, 7)));
        assert_eq!(
            parse_firmware_version(&payload[..10]),
            Err(WmiError::Truncated)
        );
    }
    #[test]
    fn reg_write_body_is_big_endian() {
        let mut body = [0u8; 8];
        assert_eq!(encode_reg_write(0x4008, 1, &mut body), Ok(8));
        assert_eq!(&body, &[0x00, 0x00, 0x40, 0x08, 0x00, 0x00, 0x00, 0x01]);
    }
    #[test]
    fn data_frames_too_large_are_rejected() {
        let mut out = [0u8; 64];
        assert_eq!(
            encode_command(0x0003, 0, &[0u8; 300], &mut out),
            Err(WmiError::BufferTooLarge)
        );
        assert_eq!(
            encode_command(0x0003, 0, &[0u8; 16], &mut out[..10]),
            Err(WmiError::BufferTooLarge)
        );
    }
}
