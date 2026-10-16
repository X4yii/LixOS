use alloc::string::String;
use alloc::vec::Vec;
pub const PROTOCOL_MAGIC: [u8; 4] = *b"LIXP";
pub const PROTOCOL_VERSION: u16 = 1;
pub const MESSAGE_HEADER_BYTES: usize = 16;
pub const MAX_MESSAGE_BYTES: usize = 4096;
pub const MAX_PAYLOAD_BYTES: usize = MAX_MESSAGE_BYTES - MESSAGE_HEADER_BYTES;
pub const MAX_CREDENTIAL_BYTES: usize = 1024;
pub const MAX_NAME_BYTES: usize = 32;
pub mod kind {
    pub const HELLO_REQUEST: u16 = 1;
    pub const HELLO_RESPONSE: u16 = 2;
    pub const LOGIN_REQUEST: u16 = 3;
    pub const LOGIN_RESPONSE: u16 = 4;
    pub const ENROLL_REQUEST: u16 = 5;
    pub const ENROLL_RESPONSE: u16 = 6;
    pub const ELEVATE_REQUEST: u16 = 7;
    pub const ELEVATE_RESPONSE: u16 = 8;
    pub const ERROR_RESPONSE: u16 = 9;
}
pub mod service_id {
    pub const AUTHD: u8 = 1;
    pub const NETMGR: u8 = 2;
    pub const BLUETOOTHD: u8 = 3;
    pub const SESSIOND: u8 = 4;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    MessageTooLarge { length: usize },
    PayloadTooLarge { length: usize },
    BadMagic,
    VersionMismatch { found: u16 },
    UnsupportedKind { kind: u16 },
    Truncated,
    ChecksumMismatch,
    InvalidField { field: &'static str },
}
impl ProtocolError {
    pub const fn code(&self) -> u16 {
        match self {
            Self::MessageTooLarge { .. } | Self::PayloadTooLarge { .. } => 1,
            Self::BadMagic => 2,
            Self::VersionMismatch { .. } => 3,
            Self::UnsupportedKind { .. } => 4,
            Self::Truncated => 5,
            Self::ChecksumMismatch => 6,
            Self::InvalidField { .. } => 7,
        }
    }
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
const fn supported_kind(kind: u16) -> bool {
    matches!(
        kind,
        kind::HELLO_REQUEST
            | kind::HELLO_RESPONSE
            | kind::LOGIN_REQUEST
            | kind::LOGIN_RESPONSE
            | kind::ENROLL_REQUEST
            | kind::ENROLL_RESPONSE
            | kind::ELEVATE_REQUEST
            | kind::ELEVATE_RESPONSE
            | kind::ERROR_RESPONSE
    )
}
pub fn encode_message(kind: u16, payload: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if !supported_kind(kind) {
        return Err(ProtocolError::UnsupportedKind { kind });
    }
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(ProtocolError::PayloadTooLarge {
            length: payload.len(),
        });
    }
    let total = MESSAGE_HEADER_BYTES + payload.len();
    if total > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::MessageTooLarge { length: total });
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&PROTOCOL_MAGIC);
    out.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    let mut checksum_input = out.clone();
    checksum_input.extend_from_slice(payload);
    out.extend_from_slice(&crc32(&checksum_input).to_le_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}
pub fn decode_message(bytes: &[u8]) -> Result<(u16, Vec<u8>), ProtocolError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::MessageTooLarge {
            length: bytes.len(),
        });
    }
    if bytes.len() < MESSAGE_HEADER_BYTES {
        return Err(ProtocolError::Truncated);
    }
    if bytes[..4] != PROTOCOL_MAGIC {
        return Err(ProtocolError::BadMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != PROTOCOL_VERSION {
        return Err(ProtocolError::VersionMismatch { found: version });
    }
    let kind = u16::from_le_bytes([bytes[6], bytes[7]]);
    if !supported_kind(kind) {
        return Err(ProtocolError::UnsupportedKind { kind });
    }
    let length = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    if length > MAX_PAYLOAD_BYTES {
        return Err(ProtocolError::PayloadTooLarge { length });
    }
    let stored = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    if bytes.len() != MESSAGE_HEADER_BYTES + length {
        return Err(ProtocolError::Truncated);
    }
    let mut checksum_input = bytes[..12].to_vec();
    checksum_input.extend_from_slice(&bytes[MESSAGE_HEADER_BYTES..]);
    if crc32(&checksum_input) != stored {
        return Err(ProtocolError::ChecksumMismatch);
    }
    Ok((kind, bytes[MESSAGE_HEADER_BYTES..].to_vec()))
}
struct Writer {
    out: Vec<u8>,
}
impl Writer {
    const fn new() -> Self {
        Self { out: Vec::new() }
    }
    fn u8(&mut self, value: u8) {
        self.out.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }
    fn bytes(&mut self, value: &[u8]) {
        self.out.extend_from_slice(value);
    }
    fn finish(self) -> Vec<u8> {
        self.out
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(ProtocolError::Truncated)?;
        if end > self.bytes.len() {
            return Err(ProtocolError::Truncated);
        }
        let slice = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ProtocolError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }
    fn u32(&mut self) -> Result<u32, ProtocolError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    fn u64(&mut self) -> Result<u64, ProtocolError> {
        let bytes = self.take(8)?;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(buf))
    }
    const fn done(&self) -> bool {
        self.offset == self.bytes.len()
    }
}
fn read_label(reader: &mut Reader<'_>, max: usize) -> Result<String, ProtocolError> {
    let length = reader.u8()? as usize;
    if length == 0 || length > max {
        return Err(ProtocolError::InvalidField { field: "label" });
    }
    let bytes = reader.take(length)?;
    let text =
        core::str::from_utf8(bytes).map_err(|_| ProtocolError::InvalidField { field: "label" })?;
    Ok(String::from(text))
}
fn read_blob(reader: &mut Reader<'_>, max: usize) -> Result<Vec<u8>, ProtocolError> {
    let length = reader.u16()? as usize;
    if length > max {
        return Err(ProtocolError::InvalidField { field: "blob" });
    }
    Ok(reader.take(length)?.to_vec())
}
fn write_label(out: &mut Writer, value: &str) -> Result<(), ProtocolError> {
    if value.is_empty() || value.len() > MAX_NAME_BYTES {
        return Err(ProtocolError::InvalidField { field: "label" });
    }
    out.u8(value.len() as u8);
    out.bytes(value.as_bytes());
    Ok(())
}
fn write_blob(out: &mut Writer, value: &[u8], max: usize) -> Result<(), ProtocolError> {
    if value.len() > max {
        return Err(ProtocolError::InvalidField { field: "blob" });
    }
    out.u16(value.len() as u16);
    out.bytes(value);
    Ok(())
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelloRequest {
    pub client_version: u16,
}
impl HelloRequest {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut writer = Writer::new();
        writer.u16(self.client_version);
        encode_message(kind::HELLO_REQUEST, &writer.finish())
    }
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(payload);
        let request = Self {
            client_version: reader.u16()?,
        };
        if !reader.done() {
            return Err(ProtocolError::InvalidField { field: "trailing" });
        }
        Ok(request)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelloResponse {
    pub server_version: u16,
    pub service: u8,
    pub max_message_bytes: u32,
}
impl HelloResponse {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut writer = Writer::new();
        writer.u16(self.server_version);
        writer.u8(self.service);
        writer.u32(self.max_message_bytes);
        encode_message(kind::HELLO_RESPONSE, &writer.finish())
    }
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(payload);
        let response = Self {
            server_version: reader.u16()?,
            service: reader.u8()?,
            max_message_bytes: reader.u32()?,
        };
        if !reader.done() {
            return Err(ProtocolError::InvalidField { field: "trailing" });
        }
        Ok(response)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginRequest {
    pub user: u64,
    pub credential: Vec<u8>,
}
impl LoginRequest {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut writer = Writer::new();
        writer.u64(self.user);
        write_blob(&mut writer, &self.credential, MAX_CREDENTIAL_BYTES)?;
        encode_message(kind::LOGIN_REQUEST, &writer.finish())
    }
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(payload);
        let request = Self {
            user: reader.u64()?,
            credential: read_blob(&mut reader, MAX_CREDENTIAL_BYTES)?,
        };
        if !reader.done() {
            return Err(ProtocolError::InvalidField { field: "trailing" });
        }
        Ok(request)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginResponse {
    pub granted: bool,
    pub session: u64,
}
impl LoginResponse {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut writer = Writer::new();
        writer.u8(u8::from(self.granted));
        writer.u64(self.session);
        encode_message(kind::LOGIN_RESPONSE, &writer.finish())
    }
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(payload);
        let granted = reader.u8()?;
        if granted > 1 {
            return Err(ProtocolError::InvalidField { field: "granted" });
        }
        let response = Self {
            granted: granted == 1,
            session: reader.u64()?,
        };
        if !reader.done() {
            return Err(ProtocolError::InvalidField { field: "trailing" });
        }
        Ok(response)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnrollRequest {
    pub user: u64,
    pub name: String,
    pub administrator: bool,
    pub credential: Vec<u8>,
}
impl EnrollRequest {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut writer = Writer::new();
        writer.u64(self.user);
        write_label(&mut writer, &self.name)?;
        writer.u8(u8::from(self.administrator));
        write_blob(&mut writer, &self.credential, MAX_CREDENTIAL_BYTES)?;
        encode_message(kind::ENROLL_REQUEST, &writer.finish())
    }
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(payload);
        let user = reader.u64()?;
        let name = read_label(&mut reader, MAX_NAME_BYTES)?;
        let administrator = reader.u8()?;
        if administrator > 1 {
            return Err(ProtocolError::InvalidField {
                field: "administrator",
            });
        }
        let credential = read_blob(&mut reader, MAX_CREDENTIAL_BYTES)?;
        if !reader.done() {
            return Err(ProtocolError::InvalidField { field: "trailing" });
        }
        Ok(Self {
            user,
            name,
            administrator: administrator == 1,
            credential,
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnrollResponse {
    pub accepted: bool,
}
impl EnrollResponse {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut writer = Writer::new();
        writer.u8(u8::from(self.accepted));
        encode_message(kind::ENROLL_RESPONSE, &writer.finish())
    }
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(payload);
        let accepted = reader.u8()?;
        if accepted > 1 || !reader.done() {
            return Err(ProtocolError::InvalidField { field: "accepted" });
        }
        Ok(Self {
            accepted: accepted == 1,
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ElevateRequest {
    pub user: u64,
    pub program: u64,
    pub scope: u8,
    pub lifetime_seconds: u32,
    pub credential: Vec<u8>,
}
impl ElevateRequest {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut writer = Writer::new();
        writer.u64(self.user);
        writer.u64(self.program);
        writer.u8(self.scope);
        writer.u32(self.lifetime_seconds);
        write_blob(&mut writer, &self.credential, MAX_CREDENTIAL_BYTES)?;
        encode_message(kind::ELEVATE_REQUEST, &writer.finish())
    }
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(payload);
        let request = Self {
            user: reader.u64()?,
            program: reader.u64()?,
            scope: reader.u8()?,
            lifetime_seconds: reader.u32()?,
            credential: read_blob(&mut reader, MAX_CREDENTIAL_BYTES)?,
        };
        if !reader.done() {
            return Err(ProtocolError::InvalidField { field: "trailing" });
        }
        if request.scope > 4 {
            return Err(ProtocolError::InvalidField { field: "scope" });
        }
        Ok(request)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ElevateResponse {
    pub granted: bool,
    pub revoker: u64,
    pub expires_at: u64,
}
impl ElevateResponse {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut writer = Writer::new();
        writer.u8(u8::from(self.granted));
        writer.u64(self.revoker);
        writer.u64(self.expires_at);
        encode_message(kind::ELEVATE_RESPONSE, &writer.finish())
    }
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(payload);
        let granted = reader.u8()?;
        if granted > 1 {
            return Err(ProtocolError::InvalidField { field: "granted" });
        }
        let response = Self {
            granted: granted == 1,
            revoker: reader.u64()?,
            expires_at: reader.u64()?,
        };
        if !reader.done() {
            return Err(ProtocolError::InvalidField { field: "trailing" });
        }
        Ok(response)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorResponse {
    pub code: u16,
}
impl ErrorResponse {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut writer = Writer::new();
        writer.u16(self.code);
        encode_message(kind::ERROR_RESPONSE, &writer.finish())
    }
    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(payload);
        let response = Self {
            code: reader.u16()?,
        };
        if !reader.done() {
            return Err(ProtocolError::InvalidField { field: "trailing" });
        }
        Ok(response)
    }
}
pub const fn reject_incompatible(client_version: u16) -> Result<HelloResponse, ProtocolError> {
    if client_version != PROTOCOL_VERSION {
        return Err(ProtocolError::VersionMismatch {
            found: client_version,
        });
    }
    Ok(HelloResponse {
        server_version: PROTOCOL_VERSION,
        service: service_id::AUTHD,
        max_message_bytes: MAX_MESSAGE_BYTES as u32,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn roundtrip(kind_value: u16, payload: &[u8]) -> (u16, Vec<u8>) {
        let encoded = encode_message(kind_value, payload).unwrap();
        decode_message(&encoded).unwrap()
    }
    #[test]
    fn message_roundtrip_preserves_kind_and_payload() {
        let payload = Vec::from(&b"hola"[..]);
        assert_eq!(
            roundtrip(kind::HELLO_REQUEST, &payload),
            (kind::HELLO_REQUEST, payload)
        );
    }
    #[test]
    fn hello_handshake_rejects_incompatible_client() {
        let request = HelloRequest {
            client_version: PROTOCOL_VERSION,
        };
        let bytes = request.encode().unwrap();
        let (kind_value, payload) = decode_message(&bytes).unwrap();
        assert_eq!(kind_value, kind::HELLO_REQUEST);
        let decoded = HelloRequest::decode(&payload).unwrap();
        assert_eq!(decoded, request);
        assert!(reject_incompatible(decoded.client_version).is_ok());
        assert_eq!(
            reject_incompatible(PROTOCOL_VERSION + 1),
            Err(ProtocolError::VersionMismatch {
                found: PROTOCOL_VERSION + 1
            })
        );
    }
    #[test]
    fn typed_messages_roundtrip() {
        let login = LoginRequest {
            user: 7,
            credential: Vec::from(&b"contrasena-larga"[..]),
        };
        let bytes = login.encode().unwrap();
        let (_, payload) = decode_message(&bytes).unwrap();
        assert_eq!(LoginRequest::decode(&payload).unwrap(), login);
        let enroll = EnrollRequest {
            user: 1,
            name: String::from("ana"),
            administrator: true,
            credential: Vec::from(&b"otra-contrasena"[..]),
        };
        let bytes = enroll.encode().unwrap();
        let (_, payload) = decode_message(&bytes).unwrap();
        assert_eq!(EnrollRequest::decode(&payload).unwrap(), enroll);
        let elevate = ElevateRequest {
            user: 1,
            program: 9,
            scope: 1,
            lifetime_seconds: 60,
            credential: Vec::from(&b"clave-temporal"[..]),
        };
        let bytes = elevate.encode().unwrap();
        let (_, payload) = decode_message(&bytes).unwrap();
        assert_eq!(ElevateRequest::decode(&payload).unwrap(), elevate);
        let response = ElevateResponse {
            granted: true,
            revoker: 3,
            expires_at: 1000,
        };
        let bytes = response.encode().unwrap();
        let (kind_value, payload) = decode_message(&bytes).unwrap();
        assert_eq!(kind_value, kind::ELEVATE_RESPONSE);
        assert_eq!(ElevateResponse::decode(&payload).unwrap(), response);
    }
    #[test]
    fn malformed_messages_are_rejected_without_panicking() {
        let encoded = encode_message(kind::LOGIN_REQUEST, &[1, 2, 3]).unwrap();
        let mut wrong_magic = encoded.clone();
        wrong_magic[0] = b'X';
        assert_eq!(decode_message(&wrong_magic), Err(ProtocolError::BadMagic));
        let mut wrong_version = encoded.clone();
        wrong_version[4] = 9;
        assert_eq!(
            decode_message(&wrong_version),
            Err(ProtocolError::VersionMismatch { found: 9 })
        );
        let mut corrupt = encoded.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0x40;
        assert_eq!(
            decode_message(&corrupt),
            Err(ProtocolError::ChecksumMismatch)
        );
        assert_eq!(
            decode_message(&encoded[..MESSAGE_HEADER_BYTES - 1]),
            Err(ProtocolError::Truncated)
        );
        assert_eq!(
            decode_message(&encoded[..encoded.len() - 1]),
            Err(ProtocolError::Truncated)
        );
        assert_eq!(decode_message(&[0u8; 64]), Err(ProtocolError::BadMagic));
    }
    #[test]
    fn declared_length_is_validated_before_use() {
        let mut bytes = encode_message(kind::HELLO_REQUEST, &[1, 2]).unwrap();
        bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            decode_message(&bytes),
            Err(ProtocolError::PayloadTooLarge {
                length: u32::MAX as usize
            })
        );
        let mut short = encode_message(kind::HELLO_REQUEST, &[1, 2, 3, 4]).unwrap();
        short[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(decode_message(&short), Err(ProtocolError::Truncated));
    }
    #[test]
    fn oversized_payload_is_refused_on_encode() {
        let payload = vec![0u8; MAX_PAYLOAD_BYTES + 1];
        assert_eq!(
            encode_message(kind::LOGIN_REQUEST, &payload),
            Err(ProtocolError::PayloadTooLarge {
                length: MAX_PAYLOAD_BYTES + 1
            })
        );
    }
    #[test]
    fn unknown_kind_is_refused() {
        assert_eq!(
            encode_message(250, &[]),
            Err(ProtocolError::UnsupportedKind { kind: 250 })
        );
        let mut bytes = encode_message(kind::HELLO_REQUEST, &[]).unwrap();
        bytes[6] = 250;
        let recomputed = crc32(&bytes[..MESSAGE_HEADER_BYTES]);
        bytes[12..16].copy_from_slice(&recomputed.to_le_bytes());
        assert_eq!(
            decode_message(&bytes),
            Err(ProtocolError::UnsupportedKind { kind: 250 })
        );
    }
    #[test]
    fn credential_over_limit_is_refused() {
        let request = LoginRequest {
            user: 1,
            credential: vec![7u8; MAX_CREDENTIAL_BYTES + 1],
        };
        assert_eq!(
            request.encode(),
            Err(ProtocolError::InvalidField { field: "blob" })
        );
    }
    #[test]
    fn trailing_bytes_are_rejected() {
        let mut writer = Writer::new();
        writer.u16(PROTOCOL_VERSION);
        let payload = writer.finish();
        let mut extended = payload.clone();
        extended.push(0);
        assert_eq!(
            HelloRequest::decode(&extended),
            Err(ProtocolError::InvalidField { field: "trailing" })
        );
        assert!(HelloRequest::decode(&payload).is_ok());
    }
}
