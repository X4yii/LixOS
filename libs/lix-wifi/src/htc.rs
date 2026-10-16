use crate::FrameError;
pub const HTC_HEADER_LEN: usize = 8;
pub const HTC_FLAGS_RECV_TRAILER: u8 = 1 << 1;
pub const HTC_MAX_CONTROL_MESSAGE_LENGTH: usize = 255;
pub const HTC_MAX_ENDPOINTS: u8 = 22;
pub const HTC_MSG_READY_ID: u16 = 1;
pub const HTC_MSG_CONNECT_SERVICE_ID: u16 = 2;
pub const HTC_MSG_CONNECT_SERVICE_RESPONSE_ID: u16 = 3;
pub const HTC_MSG_SETUP_COMPLETE_ID: u16 = 4;
pub const RSVD_SERVICE_GROUP: u16 = 0;
pub const WMI_SERVICE_GROUP: u16 = 1;
pub const HTC_CTRL_RSVD_SVC: u16 = (RSVD_SERVICE_GROUP << 8) | 1;
pub const WMI_CONTROL_SVC: u16 = (WMI_SERVICE_GROUP << 8) | 0;
pub const WMI_BEACON_SVC: u16 = (WMI_SERVICE_GROUP << 8) | 1;
pub const WMI_MGMT_SVC: u16 = (WMI_SERVICE_GROUP << 8) | 4;
pub const WMI_DATA_VO_SVC: u16 = (WMI_SERVICE_GROUP << 8) | 5;
pub const WMI_DATA_VI_SVC: u16 = (WMI_SERVICE_GROUP << 8) | 6;
pub const WMI_DATA_BE_SVC: u16 = (WMI_SERVICE_GROUP << 8) | 7;
pub const WMI_DATA_BK_SVC: u16 = (WMI_SERVICE_GROUP << 8) | 8;
pub const HTC_SERVICE_SUCCESS: u8 = 0;
pub const HTC_SERVICE_NOT_FOUND: u8 = 1;
pub const HTC_SERVICE_FAILED: u8 = 2;
pub const HTC_SERVICE_NO_RESOURCES: u8 = 3;
pub const HTC_SERVICE_NO_MORE_EP: u8 = 4;
pub const UL_PIPE_CONTROL: u8 = 0;
pub const UL_PIPE_DATA: u8 = 1;
pub const DL_PIPE_DATA: u8 = 2;
pub const DL_PIPE_CONTROL: u8 = 3;
pub const UL_PIPE_WMI: u8 = 4;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtcError {
    Truncated,
    InvalidMessage,
    BufferTooSmall,
    UnexpectedState,
    EndpointOutOfRange,
    PayloadTooLarge,
}
impl From<FrameError> for HtcError {
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::Truncated => HtcError::Truncated,
            FrameError::InvalidTransportFrame => HtcError::InvalidMessage,
            _ => HtcError::InvalidMessage,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HtcFrame<'a> {
    pub endpoint_id: u8,
    pub flags: u8,
    pub payload: &'a [u8],
}
pub fn encode_frame(
    endpoint_id: u8,
    flags: u8,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, HtcError> {
    if endpoint_id >= HTC_MAX_ENDPOINTS {
        return Err(HtcError::EndpointOutOfRange);
    }
    let length = u16::try_from(payload.len()).map_err(|_| HtcError::PayloadTooLarge)?;
    let total = HTC_HEADER_LEN
        .checked_add(payload.len())
        .ok_or(HtcError::PayloadTooLarge)?;
    if out.len() < total {
        return Err(HtcError::BufferTooSmall);
    }
    out[0] = endpoint_id;
    out[1] = flags;
    out[2..4].copy_from_slice(&length.to_be_bytes());
    out[4..8].fill(0);
    out[8..total].copy_from_slice(payload);
    Ok(total)
}
pub fn parse_frame(bytes: &[u8]) -> Result<HtcFrame<'_>, HtcError> {
    if bytes.len() < HTC_HEADER_LEN {
        return Err(HtcError::Truncated);
    }
    let endpoint_id = bytes[0];
    if endpoint_id >= HTC_MAX_ENDPOINTS {
        return Err(HtcError::EndpointOutOfRange);
    }
    let length = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
    let total = HTC_HEADER_LEN
        .checked_add(length)
        .ok_or(HtcError::InvalidMessage)?;
    if bytes.len() < total {
        return Err(HtcError::Truncated);
    }
    Ok(HtcFrame {
        endpoint_id,
        flags: bytes[1],
        payload: &bytes[HTC_HEADER_LEN..total],
    })
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadyMessage {
    pub credits: u16,
    pub credit_size: u16,
    pub max_endpoints: u8,
}
pub fn parse_ready(payload: &[u8]) -> Result<ReadyMessage, HtcError> {
    if payload.len() < 8 {
        return Err(HtcError::Truncated);
    }
    if u16::from_be_bytes([payload[0], payload[1]]) != HTC_MSG_READY_ID {
        return Err(HtcError::InvalidMessage);
    }
    Ok(ReadyMessage {
        credits: u16::from_be_bytes([payload[2], payload[3]]),
        credit_size: u16::from_be_bytes([payload[4], payload[5]]),
        max_endpoints: payload[6],
    })
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectResponse {
    pub service_id: u16,
    pub status: u8,
    pub endpoint_id: u8,
    pub max_message_len: u16,
}
pub fn parse_connect_response(payload: &[u8]) -> Result<ConnectResponse, HtcError> {
    if payload.len() < 10 {
        return Err(HtcError::Truncated);
    }
    if u16::from_be_bytes([payload[0], payload[1]]) != HTC_MSG_CONNECT_SERVICE_RESPONSE_ID {
        return Err(HtcError::InvalidMessage);
    }
    Ok(ConnectResponse {
        service_id: u16::from_be_bytes([payload[2], payload[3]]),
        status: payload[4],
        endpoint_id: payload[5],
        max_message_len: u16::from_be_bytes([payload[6], payload[7]]),
    })
}
pub fn encode_connect_service(service_id: u16, out: &mut [u8]) -> Result<usize, HtcError> {
    if out.len() < 10 {
        return Err(HtcError::BufferTooSmall);
    }
    out[0..2].copy_from_slice(&HTC_MSG_CONNECT_SERVICE_ID.to_be_bytes());
    out[2..4].copy_from_slice(&service_id.to_be_bytes());
    out[4..6].copy_from_slice(&0u16.to_be_bytes());
    out[6] = if service_id == WMI_CONTROL_SVC {
        DL_PIPE_CONTROL
    } else {
        DL_PIPE_DATA
    };
    out[7] = match service_id {
        WMI_CONTROL_SVC => UL_PIPE_WMI,
        HTC_CTRL_RSVD_SVC => UL_PIPE_CONTROL,
        _ => UL_PIPE_DATA,
    };
    out[8] = 0;
    out[9] = 0;
    Ok(10)
}
pub fn encode_setup_complete(out: &mut [u8]) -> Result<usize, HtcError> {
    if out.len() < 2 {
        return Err(HtcError::BufferTooSmall);
    }
    out[0..2].copy_from_slice(&HTC_MSG_SETUP_COMPLETE_ID.to_be_bytes());
    Ok(2)
}
fn message_id(payload: &[u8]) -> Result<u16, HtcError> {
    let head = payload.get(..2).ok_or(HtcError::Truncated)?;
    Ok(u16::from_be_bytes([head[0], head[1]]))
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtcEvent {
    TargetReady {
        credits: u16,
        credit_size: u16,
        max_endpoints: u8,
    },
    ServiceConnected {
        service_id: u16,
        endpoint_id: u8,
        max_message_len: u16,
    },
    ServiceFailed {
        service_id: u16,
        status: u8,
    },
    DataMessage {
        endpoint_id: u8,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtcState {
    Idle,
    AwaitingReady,
    Ready,
    ServicePending,
    ServiceAccepted,
    SetupSent,
    Running,
    Failed,
}
pub struct HtcEngine {
    state: HtcState,
    max_endpoints: u8,
    credit_size: u16,
    wmi_endpoint: Option<u8>,
    pending_service: u16,
    deadline: u64,
    errors: u32,
}
impl HtcEngine {
    pub const fn new() -> Self {
        Self {
            state: HtcState::Idle,
            max_endpoints: 0,
            credit_size: 0,
            wmi_endpoint: None,
            pending_service: 0,
            deadline: 0,
            errors: 0,
        }
    }
    pub const fn state(&self) -> HtcState {
        self.state
    }
    pub const fn wmi_endpoint(&self) -> Option<u8> {
        self.wmi_endpoint
    }
    pub const fn error_count(&self) -> u32 {
        self.errors
    }
    pub fn begin(&mut self, now: u64, timeout_ticks: u64) {
        self.state = HtcState::AwaitingReady;
        self.deadline = now.wrapping_add(timeout_ticks);
        self.wmi_endpoint = None;
        self.pending_service = 0;
    }
    pub fn expired(&self, now: u64) -> bool {
        match self.state {
            HtcState::AwaitingReady
            | HtcState::ServicePending
            | HtcState::ServiceAccepted
            | HtcState::SetupSent => now >= self.deadline,
            _ => false,
        }
    }
    pub fn request_service(
        &mut self,
        service_id: u16,
        now: u64,
        timeout_ticks: u64,
        out: &mut [u8],
    ) -> Result<usize, HtcError> {
        if !matches!(self.state, HtcState::Ready | HtcState::Running) {
            return Err(HtcError::UnexpectedState);
        }
        let length = encode_connect_service(service_id, out)?;
        self.state = HtcState::ServicePending;
        self.pending_service = service_id;
        self.deadline = now.wrapping_add(timeout_ticks);
        Ok(length)
    }
    pub fn setup_complete(&mut self, out: &mut [u8]) -> Result<usize, HtcError> {
        if self.state != HtcState::ServiceAccepted {
            return Err(HtcError::UnexpectedState);
        }
        let length = encode_setup_complete(out)?;
        self.state = HtcState::SetupSent;
        Ok(length)
    }
    pub fn mark_running(&mut self) -> Result<(), HtcError> {
        if self.state != HtcState::SetupSent {
            return Err(HtcError::UnexpectedState);
        }
        self.state = HtcState::Running;
        Ok(())
    }
    pub fn on_control_message(&mut self, payload: &[u8]) -> Result<Option<HtcEvent>, HtcError> {
        match message_id(payload)? {
            HTC_MSG_READY_ID => {
                let ready = parse_ready(payload)?;
                if ready.max_endpoints == 0 || ready.max_endpoints > HTC_MAX_ENDPOINTS {
                    self.errors = self.errors.saturating_add(1);
                    return Err(HtcError::InvalidMessage);
                }
                if self.state != HtcState::AwaitingReady {
                    self.errors = self.errors.saturating_add(1);
                    return Err(HtcError::UnexpectedState);
                }
                self.state = HtcState::Ready;
                self.max_endpoints = ready.max_endpoints;
                self.credit_size = ready.credit_size;
                Ok(Some(HtcEvent::TargetReady {
                    credits: ready.credits,
                    credit_size: ready.credit_size,
                    max_endpoints: ready.max_endpoints,
                }))
            }
            HTC_MSG_CONNECT_SERVICE_RESPONSE_ID => {
                let response = parse_connect_response(payload)?;
                if self.state != HtcState::ServicePending
                    || response.service_id != self.pending_service
                {
                    self.errors = self.errors.saturating_add(1);
                    return Err(HtcError::UnexpectedState);
                }
                if response.status != HTC_SERVICE_SUCCESS {
                    self.state = HtcState::Ready;
                    self.pending_service = 0;
                    return Ok(Some(HtcEvent::ServiceFailed {
                        service_id: response.service_id,
                        status: response.status,
                    }));
                }
                if response.endpoint_id == 0 || response.endpoint_id >= self.max_endpoints {
                    self.state = HtcState::Ready;
                    self.pending_service = 0;
                    self.errors = self.errors.saturating_add(1);
                    return Err(HtcError::EndpointOutOfRange);
                }
                self.state = HtcState::ServiceAccepted;
                if response.service_id == WMI_CONTROL_SVC {
                    self.wmi_endpoint = Some(response.endpoint_id);
                }
                Ok(Some(HtcEvent::ServiceConnected {
                    service_id: response.service_id,
                    endpoint_id: response.endpoint_id,
                    max_message_len: response.max_message_len,
                }))
            }
            _ => Ok(None),
        }
    }
    pub fn on_data_message(&mut self, endpoint_id: u8) -> Result<HtcEvent, HtcError> {
        if self.state != HtcState::Running {
            return Err(HtcError::UnexpectedState);
        }
        if endpoint_id == 0 || endpoint_id >= HTC_MAX_ENDPOINTS {
            return Err(HtcError::EndpointOutOfRange);
        }
        Ok(HtcEvent::DataMessage { endpoint_id })
    }
    pub fn fail(&mut self) {
        self.state = HtcState::Failed;
        self.errors = self.errors.saturating_add(1);
        self.wmi_endpoint = None;
        self.pending_service = 0;
    }
    pub fn endpoint_for(&self, service_id: u16) -> Option<u8> {
        if service_id == WMI_CONTROL_SVC {
            self.wmi_endpoint
        } else {
            None
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn hex(bytes: &[u8]) -> alloc::string::String {
        let mut text = alloc::string::String::new();
        for byte in bytes {
            text.push_str(&alloc::format!("{byte:02x}"));
        }
        text
    }
    #[test]
    fn frame_round_trip_is_little_endian_length_big_endian() {
        let mut out = [0u8; 32];
        let written = encode_frame(3, 0, b"payload", &mut out).unwrap();
        assert_eq!(written, 15);
        assert_eq!(hex(&out[..HTC_HEADER_LEN]), "0300000700000000");
        let frame = parse_frame(&out[..written]).unwrap();
        assert_eq!(frame.endpoint_id, 3);
        assert_eq!(frame.payload, b"payload");
    }
    #[test]
    fn frame_rejects_truncated_and_bad_endpoint() {
        assert_eq!(parse_frame(&[0u8; 7]), Err(HtcError::Truncated));
        let mut out = [0u8; 32];
        let written = encode_frame(3, 0, b"payload", &mut out).unwrap();
        assert_eq!(parse_frame(&out[..written - 1]), Err(HtcError::Truncated));
        assert_eq!(
            encode_frame(HTC_MAX_ENDPOINTS, 0, b"x", &mut out),
            Err(HtcError::EndpointOutOfRange)
        );
        let mut tiny = [0u8; 4];
        assert_eq!(
            encode_frame(1, 0, b"toolongpayload", &mut tiny),
            Err(HtcError::BufferTooSmall)
        );
    }
    #[test]
    fn parses_ready_and_connect_response() {
        let ready = [0x00, 0x01, 0x00, 0x20, 0x01, 0x00, 0x08, 0x00];
        assert_eq!(
            parse_ready(&ready).unwrap(),
            ReadyMessage {
                credits: 32,
                credit_size: 256,
                max_endpoints: 8
            }
        );
        let response = [0x00, 0x03, 0x01, 0x00, 0x00, 0x02, 0x04, 0x00, 0x00, 0x00];
        assert_eq!(
            parse_connect_response(&response).unwrap(),
            ConnectResponse {
                service_id: WMI_CONTROL_SVC,
                status: HTC_SERVICE_SUCCESS,
                endpoint_id: 2,
                max_message_len: 1024
            }
        );
        assert_eq!(
            parse_ready(&[0x00, 0x02, 0, 0, 0, 0, 0, 0]),
            Err(HtcError::InvalidMessage)
        );
    }
    #[test]
    fn connect_service_message_uses_documented_pipe_ids() {
        let mut out = [0u8; 10];
        let length = encode_connect_service(WMI_CONTROL_SVC, &mut out).unwrap();
        assert_eq!(length, 10);
        assert_eq!(&out[0..2], &[0x00, 0x02]);
        assert_eq!(&out[2..4], &[0x01, 0x00]);
        assert_eq!(out[6], DL_PIPE_CONTROL);
        assert_eq!(out[7], UL_PIPE_WMI);
        let length = encode_connect_service(WMI_DATA_BE_SVC, &mut out).unwrap();
        assert_eq!(length, 10);
        assert_eq!(&out[2..4], &[0x01, 0x07]);
        assert_eq!(out[6], DL_PIPE_DATA);
        assert_eq!(out[7], UL_PIPE_DATA);
    }
    #[test]
    fn engine_follows_ready_connect_setup_sequence() {
        let mut engine = HtcEngine::new();
        engine.begin(100, 50);
        assert_eq!(engine.state(), HtcState::AwaitingReady);
        assert!(!engine.expired(149));
        assert!(engine.expired(151));
        let ready = [0x00, 0x01, 0x00, 0x20, 0x01, 0x00, 0x08, 0x00];
        let event = engine.on_control_message(&ready).unwrap().unwrap();
        assert_eq!(
            event,
            HtcEvent::TargetReady {
                credits: 32,
                credit_size: 256,
                max_endpoints: 8
            }
        );
        let mut request = [0u8; 16];
        let length = engine
            .request_service(WMI_CONTROL_SVC, 200, 50, &mut request)
            .unwrap();
        assert_eq!(length, 10);
        assert_eq!(engine.state(), HtcState::ServicePending);
        let response = [0x00, 0x03, 0x01, 0x00, 0x00, 0x02, 0x04, 0x00, 0x00, 0x00];
        let event = engine.on_control_message(&response).unwrap().unwrap();
        assert_eq!(
            event,
            HtcEvent::ServiceConnected {
                service_id: WMI_CONTROL_SVC,
                endpoint_id: 2,
                max_message_len: 1024
            }
        );
        assert_eq!(engine.wmi_endpoint(), Some(2));
        let mut setup = [0u8; 8];
        assert_eq!(engine.setup_complete(&mut setup).unwrap(), 2);
        assert_eq!(&setup[..2], &[0x00, 0x04]);
        engine.mark_running().unwrap();
        assert_eq!(engine.state(), HtcState::Running);
        assert_eq!(
            engine.on_data_message(2).unwrap(),
            HtcEvent::DataMessage { endpoint_id: 2 }
        );
    }
    #[test]
    fn engine_rejects_out_of_sequence_messages() {
        let mut engine = HtcEngine::new();
        assert_eq!(
            engine.on_control_message(&[0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x08, 0x00]),
            Err(HtcError::UnexpectedState)
        );
        engine.begin(0, 10);
        assert_eq!(
            engine.on_control_message(&[0x00, 0x03, 0x01, 0x00, 0x00, 0x02, 0x04, 0x00, 0, 0]),
            Err(HtcError::UnexpectedState)
        );
        let ready = [0x00, 0x01, 0x00, 0x20, 0x01, 0x00, 0x08, 0x00];
        engine.on_control_message(&ready).unwrap();
        assert_eq!(
            engine.on_control_message(&ready),
            Err(HtcError::UnexpectedState)
        );
        assert_eq!(
            engine.on_control_message(&[0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]),
            Err(HtcError::InvalidMessage)
        );
    }
    #[test]
    fn engine_reports_service_rejection() {
        let mut engine = HtcEngine::new();
        engine.begin(0, 10);
        let ready = [0x00, 0x01, 0x00, 0x20, 0x01, 0x00, 0x08, 0x00];
        engine.on_control_message(&ready).unwrap();
        let mut request = [0u8; 16];
        engine
            .request_service(WMI_CONTROL_SVC, 1, 10, &mut request)
            .unwrap();
        let response = [0x00, 0x03, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(
            engine.on_control_message(&response).unwrap().unwrap(),
            HtcEvent::ServiceFailed {
                service_id: WMI_CONTROL_SVC,
                status: HTC_SERVICE_FAILED
            }
        );
        assert_eq!(engine.state(), HtcState::Ready);
        engine.fail();
        assert_eq!(engine.state(), HtcState::Failed);
        assert_eq!(engine.error_count(), 1);
        assert_eq!(engine.wmi_endpoint(), None);
    }
}
