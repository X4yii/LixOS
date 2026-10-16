use crate::crypto::{ccm_decrypt, ccm_encrypt, Aes128, CryptoError};
use crate::FrameError;
use alloc::vec;
use alloc::vec::Vec;

pub const LLC_SNAP_HEADER: [u8; 6] = [0xaa, 0xaa, 0x03, 0x00, 0x00, 0x00];
pub const CCMP_HEADER_LEN: usize = 8;
pub const CCMP_MIC_LEN: usize = 8;
pub const IEEE80211_HEADER_LEN: usize = 24;
pub const ETHERNET_HEADER_LEN: usize = 14;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataError {
    Truncated,
    InvalidFrame,
    BufferTooSmall,
    ReplayDetected,
    Crypto(CryptoError),
    InvalidLlcSnap,
    UnsupportedFrameType,
}

impl From<CryptoError> for DataError {
    fn from(error: CryptoError) -> Self {
        DataError::Crypto(error)
    }
}

impl From<FrameError> for DataError {
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::Truncated => DataError::Truncated,
            _ => DataError::InvalidFrame,
        }
    }
}

pub fn encapsulate_ethernet_to_80211(
    ethernet: &[u8],
    bssid: &[u8; 6],
    station_mac: &[u8; 6],
    pn: &mut u64,
    tk: &[u8; 16],
    sequence: u16,
    out: &mut [u8],
) -> Result<usize, DataError> {
    if ethernet.len() < ETHERNET_HEADER_LEN {
        return Err(DataError::Truncated);
    }
    let dest_mac = &ethernet[0..6];
    let ethertype = &ethernet[12..14];
    let payload = &ethernet[14..];

    let plaintext_len = 8usize
        .checked_add(payload.len())
        .ok_or(DataError::BufferTooSmall)?;
    let total_len = IEEE80211_HEADER_LEN
        .checked_add(CCMP_HEADER_LEN)
        .and_then(|n| n.checked_add(plaintext_len))
        .and_then(|n| n.checked_add(CCMP_MIC_LEN))
        .ok_or(DataError::BufferTooSmall)?;

    if out.len() < total_len {
        return Err(DataError::BufferTooSmall);
    }

    *pn = pn.checked_add(1).ok_or(DataError::ReplayDetected)?;
    let current_pn = *pn;

    out[0] = 0x08;
    out[1] = 0x41;
    out[2..4].fill(0);
    out[4..10].copy_from_slice(bssid);
    out[10..16].copy_from_slice(station_mac);
    out[16..22].copy_from_slice(dest_mac);
    let seq_control = (sequence & 0x0fff) << 4;
    out[22..24].copy_from_slice(&seq_control.to_le_bytes());

    let pn0 = (current_pn & 0xff) as u8;
    let pn1 = ((current_pn >> 8) & 0xff) as u8;
    let pn2 = ((current_pn >> 16) & 0xff) as u8;
    let pn3 = ((current_pn >> 24) & 0xff) as u8;
    let pn4 = ((current_pn >> 32) & 0xff) as u8;
    let pn5 = ((current_pn >> 40) & 0xff) as u8;

    out[24] = pn0;
    out[25] = pn1;
    out[26] = 0x00;
    out[27] = 0x20;
    out[28] = pn2;
    out[29] = pn3;
    out[30] = pn4;
    out[31] = pn5;

    let mut plaintext = Vec::with_capacity(plaintext_len);
    plaintext.extend_from_slice(&LLC_SNAP_HEADER);
    plaintext.extend_from_slice(ethertype);
    plaintext.extend_from_slice(payload);

    let mut aad = [0u8; 22];
    aad[0] = out[0] & 0x8f;
    aad[1] = out[1] & 0xc7;
    aad[2..8].copy_from_slice(bssid);
    aad[8..14].copy_from_slice(station_mac);
    aad[14..20].copy_from_slice(dest_mac);
    aad[20] = out[22] & 0x0f;
    aad[21] = 0;

    let mut nonce = [0u8; 13];
    nonce[0] = 0;
    nonce[1..7].copy_from_slice(station_mac);
    nonce[7] = pn5;
    nonce[8] = pn4;
    nonce[9] = pn3;
    nonce[10] = pn2;
    nonce[11] = pn1;
    nonce[12] = pn0;

    let cipher_out = &mut out[32..total_len];
    let cipher_len = ccm_encrypt(&Aes128::new(tk), &nonce, &aad, &plaintext, cipher_out)?;
    if cipher_len != plaintext_len + CCMP_MIC_LEN {
        return Err(DataError::InvalidFrame);
    }

    Ok(total_len)
}

pub fn decapsulate_80211_to_ethernet(
    frame: &[u8],
    station_mac: &[u8; 6],
    bssid: &[u8; 6],
    tk: &[u8; 16],
    last_pn: &mut u64,
    out: &mut [u8],
) -> Result<usize, DataError> {
    if frame.len() < IEEE80211_HEADER_LEN + CCMP_HEADER_LEN + 8 + CCMP_MIC_LEN {
        return Err(DataError::Truncated);
    }

    let fc = u16::from_le_bytes([frame[0], frame[1]]);
    let frame_type = (fc >> 2) & 0x03;
    let frame_subtype = (fc >> 4) & 0x0f;
    if frame_type != 2 {
        return Err(DataError::UnsupportedFrameType);
    }

    let to_ds = (fc & (1 << 8)) != 0;
    let from_ds = (fc & (1 << 9)) != 0;
    let protected = (fc & (1 << 14)) != 0;

    if !protected {
        return Err(DataError::InvalidFrame);
    }

    let is_qos = frame_subtype == 8;
    let header_len = if is_qos { 26 } else { 24 };
    if frame.len() < header_len + CCMP_HEADER_LEN + 8 + CCMP_MIC_LEN {
        return Err(DataError::Truncated);
    }

    let (da, sa, ta) = if !to_ds && from_ds {
        (&frame[4..10], &frame[16..22], &frame[10..16])
    } else if to_ds && !from_ds {
        (&frame[16..22], &frame[10..16], &frame[10..16])
    } else {
        return Err(DataError::InvalidFrame);
    };

    if !to_ds && from_ds && ta != bssid {
        return Err(DataError::InvalidFrame);
    }

    let ccmp_header = &frame[header_len..header_len + CCMP_HEADER_LEN];
    if ccmp_header[3] & 0x20 == 0 {
        return Err(DataError::InvalidFrame);
    }

    let pn0 = ccmp_header[0] as u64;
    let pn1 = ccmp_header[1] as u64;
    let pn2 = ccmp_header[4] as u64;
    let pn3 = ccmp_header[5] as u64;
    let pn4 = ccmp_header[6] as u64;
    let pn5 = ccmp_header[7] as u64;

    let received_pn = pn0 | (pn1 << 8) | (pn2 << 16) | (pn3 << 24) | (pn4 << 32) | (pn5 << 40);
    if received_pn <= *last_pn {
        return Err(DataError::ReplayDetected);
    }

    let mut aad = [0u8; 26];
    let aad_len = if is_qos { 24 } else { 22 };
    aad[0] = frame[0] & 0x8f;
    aad[1] = frame[1] & 0xc7;
    aad[2..8].copy_from_slice(&frame[4..10]);
    aad[8..14].copy_from_slice(&frame[10..16]);
    aad[14..20].copy_from_slice(&frame[16..22]);
    aad[20] = frame[22] & 0x0f;
    aad[21] = 0;
    if is_qos {
        aad[22] = frame[24] & 0x0f;
        aad[23] = 0;
    }

    let mut nonce = [0u8; 13];
    nonce[0] = if is_qos { frame[24] & 0x0f } else { 0 };
    nonce[1..7].copy_from_slice(ta);
    nonce[7] = ccmp_header[7];
    nonce[8] = ccmp_header[6];
    nonce[9] = ccmp_header[5];
    nonce[10] = ccmp_header[4];
    nonce[11] = ccmp_header[1];
    nonce[12] = ccmp_header[0];

    let cipher_start = header_len + CCMP_HEADER_LEN;
    let ciphertext_and_mic = &frame[cipher_start..];
    let plaintext_max = ciphertext_and_mic.len().saturating_sub(CCMP_MIC_LEN);

    let mut decrypted = vec![0u8; plaintext_max];
    let decrypted_len = ccm_decrypt(
        &Aes128::new(tk),
        &nonce,
        &aad[..aad_len],
        ciphertext_and_mic,
        &mut decrypted,
    )?;

    if decrypted_len < 8 {
        return Err(DataError::Truncated);
    }

    if decrypted[0..6] != LLC_SNAP_HEADER {
        return Err(DataError::InvalidLlcSnap);
    }

    let ethertype = &decrypted[6..8];
    let payload = &decrypted[8..decrypted_len];

    let total_ethernet_len = ETHERNET_HEADER_LEN
        .checked_add(payload.len())
        .ok_or(DataError::BufferTooSmall)?;

    if out.len() < total_ethernet_len {
        return Err(DataError::BufferTooSmall);
    }

    out[0..6].copy_from_slice(da);
    out[6..12].copy_from_slice(sa);
    out[12..14].copy_from_slice(ethertype);
    out[14..total_ethernet_len].copy_from_slice(payload);

    *last_pn = received_pn;
    let _ = station_mac;
    Ok(total_ethernet_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TK: [u8; 16] = [
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
        0x1f,
    ];
    const BSSID: [u8; 6] = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
    const STATION: [u8; 6] = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
    const GATEWAY: [u8; 6] = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];

    #[test]
    fn roundtrip_ethernet_over_80211_ccmp() {
        let mut eth_payload = Vec::new();
        eth_payload.extend_from_slice(&GATEWAY);
        eth_payload.extend_from_slice(&STATION);
        eth_payload.extend_from_slice(&[0x08, 0x00]);
        eth_payload.extend_from_slice(b"Hello, LixOS Wi-Fi 802.11 CCMP data pipeline!");

        let mut tx_pn = 100u64;
        let mut frame_buf = [0u8; 512];
        let frame_len = encapsulate_ethernet_to_80211(
            &eth_payload,
            &BSSID,
            &STATION,
            &mut tx_pn,
            &TK,
            42,
            &mut frame_buf,
        )
        .unwrap();

        assert_eq!(tx_pn, 101);
        assert_eq!(&frame_buf[0..2], &[0x08, 0x41]);

        let mut rx_frame = frame_buf[..frame_len].to_vec();
        rx_frame[0] = 0x08;
        rx_frame[1] = 0x42;
        rx_frame[4..10].copy_from_slice(&STATION);
        rx_frame[10..16].copy_from_slice(&BSSID);
        rx_frame[16..22].copy_from_slice(&GATEWAY);

        let mut rx_pn = 100u64;
        let mut out_eth = [0u8; 512];

        let mut ap_frame_buf = [0u8; 512];
        let mut ap_eth = Vec::new();
        ap_eth.extend_from_slice(&STATION);
        ap_eth.extend_from_slice(&GATEWAY);
        ap_eth.extend_from_slice(&[0x08, 0x00]);
        ap_eth.extend_from_slice(b"Hello, LixOS Wi-Fi 802.11 CCMP data pipeline!");

        ap_frame_buf[0] = 0x08;
        ap_frame_buf[1] = 0x42;
        ap_frame_buf[2..4].fill(0);
        ap_frame_buf[4..10].copy_from_slice(&STATION);
        ap_frame_buf[10..16].copy_from_slice(&BSSID);
        ap_frame_buf[16..22].copy_from_slice(&GATEWAY);
        let seq = 42u16 << 4;
        ap_frame_buf[22..24].copy_from_slice(&seq.to_le_bytes());

        let cur_pn = 101u64;
        ap_frame_buf[24] = (cur_pn & 0xff) as u8;
        ap_frame_buf[25] = ((cur_pn >> 8) & 0xff) as u8;
        ap_frame_buf[26] = 0;
        ap_frame_buf[27] = 0x20;
        ap_frame_buf[28] = ((cur_pn >> 16) & 0xff) as u8;
        ap_frame_buf[29] = ((cur_pn >> 24) & 0xff) as u8;
        ap_frame_buf[30] = ((cur_pn >> 32) & 0xff) as u8;
        ap_frame_buf[31] = ((cur_pn >> 40) & 0xff) as u8;

        let mut plaintext = Vec::new();
        plaintext.extend_from_slice(&LLC_SNAP_HEADER);
        plaintext.extend_from_slice(&[0x08, 0x00]);
        plaintext.extend_from_slice(b"Hello, LixOS Wi-Fi 802.11 CCMP data pipeline!");

        let mut aad = [0u8; 22];
        aad[0] = ap_frame_buf[0] & 0x8f;
        aad[1] = ap_frame_buf[1] & 0xc7;
        aad[2..8].copy_from_slice(&STATION);
        aad[8..14].copy_from_slice(&BSSID);
        aad[14..20].copy_from_slice(&GATEWAY);
        aad[20] = ap_frame_buf[22] & 0x0f;
        aad[21] = 0;

        let mut nonce = [0u8; 13];
        nonce[0] = 0;
        nonce[1..7].copy_from_slice(&BSSID);
        nonce[7] = ap_frame_buf[31];
        nonce[8] = ap_frame_buf[30];
        nonce[9] = ap_frame_buf[29];
        nonce[10] = ap_frame_buf[28];
        nonce[11] = ap_frame_buf[25];
        nonce[12] = ap_frame_buf[24];

        let cipher_len = ccm_encrypt(
            &Aes128::new(&TK),
            &nonce,
            &aad,
            &plaintext,
            &mut ap_frame_buf[32..],
        )
        .unwrap();

        let total_ap_len = 32 + cipher_len;

        let decapsulated_len = decapsulate_80211_to_ethernet(
            &ap_frame_buf[..total_ap_len],
            &STATION,
            &BSSID,
            &TK,
            &mut rx_pn,
            &mut out_eth,
        )
        .unwrap();

        assert_eq!(decapsulated_len, ap_eth.len());
        assert_eq!(&out_eth[..decapsulated_len], &ap_eth[..]);
        assert_eq!(rx_pn, 101);

        let replay_err = decapsulate_80211_to_ethernet(
            &ap_frame_buf[..total_ap_len],
            &STATION,
            &BSSID,
            &TK,
            &mut rx_pn,
            &mut out_eth,
        );
        assert_eq!(replay_err, Err(DataError::ReplayDetected));
    }
}
