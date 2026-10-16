use alloc::vec::Vec;
use crate::crypto::{HmacSha1, hmac_sha1, pbkdf2_sha1};
const EAPOL_TYPE_KEY: u8 = 3;
const DESC_RSN: u8 = 2;
const KEY_INFO_VERSION_MASK: u16 = 0x0007;
const KEY_INFO_VERSION_2: u16 = 0x0002;
const KEY_INFO_PAIRWISE: u16 = 0x0008;
const KEY_INFO_ACK: u16 = 0x0080;
const KEY_INFO_MIC: u16 = 0x0100;
const KEY_INFO_SECURE: u16 = 0x0200;
const KEY_INFO_ERROR: u16 = 0x0400;
const KEY_INFO_REQUEST: u16 = 0x0800;
const EAPOL_HEADER: usize = 4;
const MIC_OFFSET: usize = 63;
const KEY_DATA_LEN_OFFSET: usize = 79;
const KEY_DATA_OFFSET: usize = 81;
const KEY_LEN_CCMP: u16 = 16;
const PTK_LEN: usize = 64;
const KCK_LEN: usize = 16;
const KEK_LEN: usize = 16;
const TK_LEN: usize = 16;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wpa2Error {
    InvalidPassphrase,
    InvalidFrame,
    DowngradeRejected,
    UnexpectedMessage,
    MicMismatch,
    ReplayMismatch,
    NonceMismatch,
    NotComplete,
}
pub fn derive_psk(passphrase: &str, ssid: &[u8]) -> Result<[u8; 32], Wpa2Error> {
    let bytes = passphrase.as_bytes();
    if bytes.len() < 8 || bytes.len() > 63 {
        return Err(Wpa2Error::InvalidPassphrase);
    }
    if ssid.is_empty() || ssid.len() > 32 {
        return Err(Wpa2Error::InvalidPassphrase);
    }
    let mut psk = [0u8; 32];
    pbkdf2_sha1(bytes, ssid, 4096, &mut psk).map_err(|_| Wpa2Error::InvalidPassphrase)?;
    Ok(psk)
}
#[allow(dead_code)]
struct KeyFrame<'a> {
    key_info: u16,
    replay: u64,
    nonce: [u8; 32],
    mic: [u8; 16],
    key_data: &'a [u8],
}
fn parse_key_frame(frame: &[u8]) -> Result<KeyFrame<'_>, Wpa2Error> {
    if frame.len() < KEY_DATA_OFFSET || frame[1] != EAPOL_TYPE_KEY {
        return Err(Wpa2Error::InvalidFrame);
    }
    let body_len = u16::from_be_bytes([frame[2], frame[3]]) as usize;
    if body_len != frame.len() - EAPOL_HEADER {
        return Err(Wpa2Error::InvalidFrame);
    }
    if frame[4] != DESC_RSN {
        return Err(Wpa2Error::DowngradeRejected);
    }
    let key_info = u16::from_be_bytes([frame[5], frame[6]]);
    if key_info & KEY_INFO_ERROR != 0 || key_info & KEY_INFO_REQUEST != 0 {
        return Err(Wpa2Error::InvalidFrame);
    }
    if key_info & KEY_INFO_VERSION_MASK != KEY_INFO_VERSION_2 {
        return Err(Wpa2Error::DowngradeRejected);
    }
    let key_data_len =
        u16::from_be_bytes([frame[KEY_DATA_LEN_OFFSET], frame[KEY_DATA_LEN_OFFSET + 1]]) as usize;
    if frame.len() != KEY_DATA_OFFSET + key_data_len {
        return Err(Wpa2Error::InvalidFrame);
    }
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&frame[17..49]);
    let mut mic = [0u8; 16];
    mic.copy_from_slice(&frame[MIC_OFFSET..MIC_OFFSET + 16]);
    Ok(KeyFrame {
        key_info,
        replay: u64::from_be_bytes(
            frame[9..17]
                .try_into()
                .map_err(|_| Wpa2Error::InvalidFrame)?,
        ),
        nonce,
        mic,
        key_data: &frame[KEY_DATA_OFFSET..],
    })
}
fn verify_mic(kck: &[u8; KCK_LEN], frame: &[u8]) -> Result<(), Wpa2Error> {
    if frame.len() < KEY_DATA_OFFSET {
        return Err(Wpa2Error::InvalidFrame);
    }
    let mut zeroed = Vec::from(frame);
    zeroed[MIC_OFFSET..MIC_OFFSET + 16].fill(0);
    let expected = hmac_sha1(kck, &zeroed);
    let mut diff = 0u8;
    for index in 0..16 {
        diff |= expected[index] ^ frame[MIC_OFFSET + index];
    }
    if diff != 0 {
        return Err(Wpa2Error::MicMismatch);
    }
    Ok(())
}
fn build_frame(
    key_info: u16,
    replay: u64,
    nonce: &[u8; 32],
    mic: &[u8; 16],
    key_data: &[u8],
) -> Vec<u8> {
    let body_len = KEY_DATA_OFFSET - EAPOL_HEADER + key_data.len();
    let mut frame = Vec::with_capacity(EAPOL_HEADER + body_len);
    frame.push(1);
    frame.push(EAPOL_TYPE_KEY);
    frame.extend_from_slice(&(body_len as u16).to_be_bytes());
    frame.push(DESC_RSN);
    frame.extend_from_slice(&key_info.to_be_bytes());
    frame.extend_from_slice(&KEY_LEN_CCMP.to_be_bytes());
    frame.extend_from_slice(&replay.to_be_bytes());
    frame.extend_from_slice(nonce);
    frame.extend_from_slice(&[0u8; 8]);
    frame.extend_from_slice(&[0u8; 6]);
    frame.extend_from_slice(mic);
    frame.extend_from_slice(&(key_data.len() as u16).to_be_bytes());
    frame.extend_from_slice(key_data);
    frame
}
fn derive_ptk(
    psk: &[u8; 32],
    aa: &[u8; 6],
    sa: &[u8; 6],
    anonce: &[u8; 32],
    snonce: &[u8; 32],
) -> [u8; PTK_LEN] {
    let mut data = Vec::with_capacity(76);
    if aa <= sa {
        data.extend_from_slice(aa);
        data.extend_from_slice(sa);
    } else {
        data.extend_from_slice(sa);
        data.extend_from_slice(aa);
    }
    if anonce <= snonce {
        data.extend_from_slice(anonce);
        data.extend_from_slice(snonce);
    } else {
        data.extend_from_slice(snonce);
        data.extend_from_slice(anonce);
    }
    let label = b"Pairwise key expansion";
    let mut out = [0u8; PTK_LEN];
    for round in 0..PTK_LEN.div_ceil(20) {
        let mut mac = HmacSha1::new(psk);
        mac.update(label);
        mac.update(&[0u8]);
        mac.update(&data);
        mac.update(&[round as u8]);
        let block = mac.finalize();
        let start = round * 20;
        let end = core::cmp::min(start + 20, PTK_LEN);
        out[start..end].copy_from_slice(&block[..end - start]);
    }
    out
}
enum State {
    NeedMessage1,
    NeedMessage3,
    Complete,
}
pub struct Handshake {
    psk: [u8; 32],
    aa: [u8; 6],
    sa: [u8; 6],
    anonce: [u8; 32],
    snonce: [u8; 32],
    kck: [u8; KCK_LEN],
    kek: [u8; KEK_LEN],
    tk: [u8; TK_LEN],
    replay: u64,
    state: State,
}
impl Drop for Handshake {
    fn drop(&mut self) {
        self.psk.fill(0);
        self.anonce.fill(0);
        self.snonce.fill(0);
        self.kck.fill(0);
        self.kek.fill(0);
        self.tk.fill(0);
    }
}
impl Handshake {
    pub fn new(psk: [u8; 32], aa: [u8; 6], sa: [u8; 6]) -> Self {
        Self {
            psk,
            aa,
            sa,
            anonce: [0u8; 32],
            snonce: [0u8; 32],
            kck: [0u8; KCK_LEN],
            kek: [0u8; KEK_LEN],
            tk: [0u8; TK_LEN],
            replay: 0,
            state: State::NeedMessage1,
        }
    }
    pub fn on_message_1(&mut self, frame: &[u8], snonce: &[u8; 32]) -> Result<Vec<u8>, Wpa2Error> {
        match self.state {
            State::NeedMessage1 => {}
            _ => return Err(Wpa2Error::UnexpectedMessage),
        }
        let parsed = parse_key_frame(frame)?;
        if parsed.key_info & KEY_INFO_PAIRWISE == 0 || parsed.key_info & KEY_INFO_ACK == 0 {
            return Err(Wpa2Error::InvalidFrame);
        }
        if parsed.key_info & KEY_INFO_MIC != 0 {
            return Err(Wpa2Error::UnexpectedMessage);
        }
        self.anonce.copy_from_slice(&parsed.nonce);
        self.snonce.copy_from_slice(snonce);
        self.replay = parsed.replay;
        let ptk = derive_ptk(&self.psk, &self.aa, &self.sa, &self.anonce, snonce);
        self.kck.copy_from_slice(&ptk[..KCK_LEN]);
        self.kek.copy_from_slice(&ptk[KCK_LEN..KCK_LEN + KEK_LEN]);
        self.tk
            .copy_from_slice(&ptk[KCK_LEN + KEK_LEN..KCK_LEN + KEK_LEN + TK_LEN]);
        let mut nonce = [0u8; 32];
        nonce.copy_from_slice(snonce);
        let mut mic = [0u8; 16];
        let mut message = build_frame(
            KEY_INFO_VERSION_2 | KEY_INFO_PAIRWISE | KEY_INFO_MIC,
            self.replay,
            &nonce,
            &mic,
            &[],
        );
        let computed = hmac_sha1(&self.kck, &message);
        mic.copy_from_slice(&computed[..16]);
        message[MIC_OFFSET..MIC_OFFSET + 16].copy_from_slice(&mic);
        self.state = State::NeedMessage3;
        Ok(message)
    }
    pub fn verify_message_3(&mut self, frame: &[u8]) -> Result<(), Wpa2Error> {
        match self.state {
            State::NeedMessage3 => {}
            _ => return Err(Wpa2Error::UnexpectedMessage),
        }
        let parsed = parse_key_frame(frame)?;
        if parsed.key_info & KEY_INFO_PAIRWISE == 0
            || parsed.key_info & KEY_INFO_ACK == 0
            || parsed.key_info & KEY_INFO_MIC == 0
            || parsed.key_info & KEY_INFO_SECURE == 0
        {
            return Err(Wpa2Error::InvalidFrame);
        }
        if parsed.replay <= self.replay {
            return Err(Wpa2Error::ReplayMismatch);
        }
        if parsed.nonce != self.anonce {
            return Err(Wpa2Error::NonceMismatch);
        }
        verify_mic(&self.kck, frame)?;
        self.replay = parsed.replay;
        Ok(())
    }
    pub fn build_message_4(&mut self) -> Result<Vec<u8>, Wpa2Error> {
        match self.state {
            State::NeedMessage3 => {}
            _ => return Err(Wpa2Error::UnexpectedMessage),
        }
        let mut nonce = [0u8; 32];
        nonce.copy_from_slice(&self.snonce);
        let mut mic = [0u8; 16];
        let mut message = build_frame(
            KEY_INFO_VERSION_2 | KEY_INFO_PAIRWISE | KEY_INFO_MIC,
            self.replay,
            &nonce,
            &mic,
            &[],
        );
        let computed = hmac_sha1(&self.kck, &message);
        mic.copy_from_slice(&computed[..16]);
        message[MIC_OFFSET..MIC_OFFSET + 16].copy_from_slice(&mic);
        self.state = State::Complete;
        Ok(message)
    }
    pub fn transmit_key(&self) -> Option<&[u8; TK_LEN]> {
        match self.state {
            State::Complete => Some(&self.tk),
            _ => None,
        }
    }
    pub fn message_2_transmitted(&self) -> bool {
        matches!(self.state, State::NeedMessage3)
    }
    pub fn completed(&self) -> bool {
        matches!(self.state, State::Complete)
    }
}
pub fn ccmp_nonce(packet: &[u8]) -> Result<[u8; 13], Wpa2Error> {
    if packet.len() < 24 {
        return Err(Wpa2Error::InvalidFrame);
    }
    let mut nonce = [0u8; 13];
    nonce[0] = 0;
    nonce[1..7].copy_from_slice(&packet[10..16]);
    nonce[7..13].copy_from_slice(&packet[24..30]);
    Ok(nonce)
}
pub fn pairwise_packet(packet: &[u8]) -> bool {
    if packet.len() < 24 {
        return false;
    }
    (packet[1] & 0x08) != 0
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;
    const PSK_HEX: &str = "f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e";
    const KCK_HEX: &str = "92c8463ee3192718b069615506cb243d";
    const KEK_HEX: &str = "1692e4f7fa7ffa532fdd1591e0fa7523";
    const TK_HEX: &str = "a9fb6ec4c6cd41f5f7f3593c218221d7";
    const M1_HEX: &str = "0103004d02008a0010000000000000000701010101010101010101010101010101010101010101010101010101010101010000000000000000000000000000000000000000000000000000000000000000";
    const M2_HEX: &str = "0103004d02010a0010000000000000000702020202020202020202020202020202020202020202020202020202020202020000000000000000000000000000a5e530e4b3bbebaa4788c5798a9968d00000";
    const M3_HEX: &str = "010300650203ca0010000000000000000801010101010101010101010101010101010101010101010101010101010101010000000000000000000000000000251adbf61d95078da1efc487a74165d00018000000000000000000000000000000000000000000000000";
    fn unhex(text: &str) -> Vec<u8> {
        let bytes = text.as_bytes();
        let mut out = Vec::with_capacity(bytes.len() / 2);
        for pair in bytes.chunks(2) {
            let hi = (pair[0] as char).to_digit(16).unwrap() as u8;
            let lo = (pair[1] as char).to_digit(16).unwrap() as u8;
            out.push(hi << 4 | lo);
        }
        out
    }
    fn to_hex(bytes: &[u8]) -> String {
        let mut out = String::new();
        for byte in bytes {
            out.push_str(&format!("{:02x}", byte));
        }
        out
    }
    #[test]
    fn psk_matches_reference_vector() {
        let psk = derive_psk("password", b"IEEE").unwrap();
        assert_eq!(to_hex(&psk), PSK_HEX);
    }
    #[test]
    fn psk_rejects_bad_passphrase() {
        assert_eq!(
            derive_psk("short", b"IEEE"),
            Err(Wpa2Error::InvalidPassphrase)
        );
        let long = "a".repeat(64);
        assert_eq!(
            derive_psk(&long, b"IEEE"),
            Err(Wpa2Error::InvalidPassphrase)
        );
    }
    #[test]
    fn message_2_matches_reference_vector() {
        let psk = derive_psk("password", b"IEEE").unwrap();
        let mut hs = Handshake::new(
            psk,
            [0, 1, 2, 3, 4, 5],
            [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        );
        let snonce = [0x02u8; 32];
        let m1 = unhex(M1_HEX);
        let m2 = hs.on_message_1(&m1, &snonce).unwrap();
        assert_eq!(to_hex(&m2), M2_HEX);
    }
    #[test]
    fn ptk_derivation_matches_reference() {
        let psk = derive_psk("password", b"IEEE").unwrap();
        let anonce = [0x01u8; 32];
        let snonce = [0x02u8; 32];
        let ptk = derive_ptk(
            &psk,
            &[0, 1, 2, 3, 4, 5],
            &[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
            &anonce,
            &snonce,
        );
        assert_eq!(to_hex(&ptk[..16]), KCK_HEX);
        assert_eq!(to_hex(&ptk[16..32]), KEK_HEX);
        assert_eq!(to_hex(&ptk[32..48]), TK_HEX);
    }
    #[test]
    fn message_3_verifies_and_message_4_builds() {
        let psk = derive_psk("password", b"IEEE").unwrap();
        let mut hs = Handshake::new(
            psk,
            [0, 1, 2, 3, 4, 5],
            [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        );
        let snonce = [0x02u8; 32];
        let m1 = unhex(M1_HEX);
        let _m2 = hs.on_message_1(&m1, &snonce).unwrap();
        let m3 = unhex(M3_HEX);
        hs.verify_message_3(&m3).unwrap();
        let m4 = hs.build_message_4().unwrap();
        assert_eq!(m4.len(), 81);
        assert_eq!(m4[5], 0x01);
        assert_eq!(m4[6], 0x0a);
        assert!(hs.completed());
    }
    #[test]
    fn tampered_message_3_is_rejected() {
        let psk = derive_psk("password", b"IEEE").unwrap();
        let mut hs = Handshake::new(
            psk,
            [0, 1, 2, 3, 4, 5],
            [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        );
        let snonce = [0x02u8; 32];
        let m1 = unhex(M1_HEX);
        let _m2 = hs.on_message_1(&m1, &snonce).unwrap();
        let mut m3 = unhex(M3_HEX);
        m3[KEY_DATA_OFFSET + 2] ^= 0x01;
        assert_eq!(hs.verify_message_3(&m3), Err(Wpa2Error::MicMismatch));
    }
    #[test]
    fn downgrade_to_version_1_is_rejected() {
        let psk = derive_psk("password", b"IEEE").unwrap();
        let mut hs = Handshake::new(
            psk,
            [0, 1, 2, 3, 4, 5],
            [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        );
        let snonce = [0x02u8; 32];
        let mut m1 = unhex(M1_HEX);
        m1[6] = (m1[6] & !0x07) | 0x01;
        assert_eq!(
            hs.on_message_1(&m1, &snonce),
            Err(Wpa2Error::DowngradeRejected)
        );
    }
    #[test]
    fn replay_reuse_is_rejected() {
        let psk = derive_psk("password", b"IEEE").unwrap();
        let mut hs = Handshake::new(
            psk,
            [0, 1, 2, 3, 4, 5],
            [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        );
        let snonce = [0x02u8; 32];
        let m1 = unhex(M1_HEX);
        let _m2 = hs.on_message_1(&m1, &snonce).unwrap();
        let m3 = unhex(M3_HEX);
        hs.verify_message_3(&m3).unwrap();
        assert_eq!(hs.verify_message_3(&m3), Err(Wpa2Error::ReplayMismatch));
    }
}
