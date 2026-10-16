use crate::{Beacon, Bssid, FrameError, SecuritySuites};
pub const FRAME_TYPE_MANAGEMENT: u8 = 0x00;
pub const SUBTYPE_PROBE_REQUEST: u8 = 4;
pub const SUBTYPE_PROBE_RESPONSE: u8 = 5;
pub const SUBTYPE_BEACON: u8 = 8;
pub const SUBTYPE_AUTHENTICATION: u8 = 11;
pub const SUBTYPE_DEAUTHENTICATION: u8 = 12;
pub const SUBTYPE_ASSOC_REQUEST: u8 = 0;
pub const SUBTYPE_ASSOC_RESPONSE: u8 = 1;
pub const AUTH_OPEN_SYSTEM: u16 = 0;
pub const AUTH_SEQUENCE_REQUEST: u16 = 1;
pub const AUTH_SEQUENCE_RESPONSE: u16 = 2;
pub const AUTH_SUCCESS: u16 = 0;
pub const STATUS_SUCCESS: u16 = 0;
pub const CAPABILITY_ESS: u16 = 1 << 0;
pub const CAPABILITY_PRIVACY: u16 = 1 << 4;
pub const IE_SSID: u8 = 0;
pub const IE_RATES: u8 = 1;
pub const IE_DS_PARAMS: u8 = 3;
pub const IE_RSN: u8 = 48;
pub const SUPPORTED_RATES: [u8; 8] = [0x82, 0x84, 0x8b, 0x96, 0x0c, 0x12, 0x18, 0x24];
pub const MAX_FRAME: usize = 256;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationError {
    Truncated,
    BufferTooSmall,
    UnsupportedFrame,
    AuthenticationRejected(u16),
    AssociationRejected(u16),
    WrongPeer,
    InvalidFrame,
}
fn put_address(frame: &mut [u8], offset: usize, address: &[u8; 6]) {
    frame[offset..offset + 6].copy_from_slice(address);
}
fn put_header(
    frame: &mut [u8],
    control: u16,
    duration: u16,
    destination: &[u8; 6],
    source: &[u8; 6],
    bssid: &[u8; 6],
    sequence: u16,
) -> Result<usize, StationError> {
    if frame.len() < 24 {
        return Err(StationError::BufferTooSmall);
    }
    frame[0..2].copy_from_slice(&control.to_le_bytes());
    frame[2..4].copy_from_slice(&duration.to_le_bytes());
    put_address(frame, 4, destination);
    put_address(frame, 10, source);
    put_address(frame, 16, bssid);
    frame[22..24].copy_from_slice(&sequence.to_le_bytes());
    Ok(24)
}
fn management_control(subtype: u8) -> u16 {
    ((subtype as u16) & 0x0f) << 4
}
pub fn build_probe_request(
    out: &mut [u8],
    transmitter: &[u8; 6],
    ssid: &[u8],
    sequence: u16,
) -> Result<usize, StationError> {
    if ssid.len() > 32 {
        return Err(StationError::InvalidFrame);
    }
    let broadcast = [0xffu8; 6];
    let offset = put_header(
        out,
        management_control(SUBTYPE_PROBE_REQUEST),
        0,
        &broadcast,
        transmitter,
        &broadcast,
        sequence,
    )?;
    let mut position = offset;
    out[position] = IE_SSID;
    out[position + 1] = ssid.len() as u8;
    out[position + 2..position + 2 + ssid.len()].copy_from_slice(ssid);
    position += 2 + ssid.len();
    out[position] = IE_RATES;
    out[position + 1] = SUPPORTED_RATES.len() as u8;
    out[position + 2..position + 10].copy_from_slice(&SUPPORTED_RATES);
    Ok(position + 10)
}
pub fn build_authentication(
    out: &mut [u8],
    transmitter: &[u8; 6],
    bssid: &[u8; 6],
    sequence: u16,
) -> Result<usize, StationError> {
    let offset = put_header(
        out,
        management_control(SUBTYPE_AUTHENTICATION),
        0,
        bssid,
        transmitter,
        bssid,
        sequence,
    )?;
    if out.len() < offset + 6 {
        return Err(StationError::BufferTooSmall);
    }
    out[offset..offset + 2].copy_from_slice(&AUTH_OPEN_SYSTEM.to_le_bytes());
    out[offset + 2..offset + 4].copy_from_slice(&AUTH_SEQUENCE_REQUEST.to_le_bytes());
    out[offset + 4..offset + 6].copy_from_slice(&0u16.to_le_bytes());
    Ok(offset + 6)
}
fn put_rsn(out: &mut [u8], position: usize) -> Result<usize, StationError> {
    let body_len = 2 + 4 + 2 + 4 + 2 + 4;
    if out.len() < position + 2 + body_len {
        return Err(StationError::BufferTooSmall);
    }
    out[position] = IE_RSN;
    out[position + 1] = body_len as u8;
    let mut cursor = position + 2;
    out[cursor..cursor + 2].copy_from_slice(&1u16.to_le_bytes());
    cursor += 2;
    out[cursor..cursor + 4].copy_from_slice(&[0x00, 0x0f, 0xac, 4]);
    cursor += 4;
    out[cursor..cursor + 2].copy_from_slice(&1u16.to_le_bytes());
    cursor += 2;
    out[cursor..cursor + 4].copy_from_slice(&[0x00, 0x0f, 0xac, 4]);
    cursor += 4;
    out[cursor..cursor + 2].copy_from_slice(&1u16.to_le_bytes());
    cursor += 2;
    out[cursor..cursor + 4].copy_from_slice(&[0x00, 0x0f, 0xac, 2]);
    cursor += 4;
    Ok(cursor)
}
pub fn build_association_request(
    out: &mut [u8],
    transmitter: &[u8; 6],
    bssid: &[u8; 6],
    ssid: &[u8],
    channel: Option<u8>,
    privacy: bool,
    sequence: u16,
) -> Result<usize, StationError> {
    if ssid.len() > 32 {
        return Err(StationError::InvalidFrame);
    }
    let offset = put_header(
        out,
        management_control(SUBTYPE_ASSOC_REQUEST),
        0,
        bssid,
        transmitter,
        bssid,
        sequence,
    )?;
    if out.len() < offset + 4 {
        return Err(StationError::BufferTooSmall);
    }
    let mut capability = CAPABILITY_ESS;
    if privacy {
        capability |= CAPABILITY_PRIVACY;
    }
    out[offset..offset + 2].copy_from_slice(&capability.to_le_bytes());
    out[offset + 2..offset + 4].copy_from_slice(&10u16.to_le_bytes());
    let mut position = offset + 4;
    out[position] = IE_SSID;
    out[position + 1] = ssid.len() as u8;
    out[position + 2..position + 2 + ssid.len()].copy_from_slice(ssid);
    position += 2 + ssid.len();
    out[position] = IE_RATES;
    out[position + 1] = SUPPORTED_RATES.len() as u8;
    out[position + 2..position + 10].copy_from_slice(&SUPPORTED_RATES);
    position += 10;
    if let Some(channel) = channel {
        if out.len() < position + 3 {
            return Err(StationError::BufferTooSmall);
        }
        out[position] = IE_DS_PARAMS;
        out[position + 1] = 1;
        out[position + 2] = channel;
        position += 3;
    }
    put_rsn(out, position)
}
fn address_of(frame: &[u8], offset: usize) -> Result<[u8; 6], StationError> {
    frame
        .get(offset..offset + 6)
        .ok_or(StationError::Truncated)?
        .try_into()
        .map_err(|_| StationError::Truncated)
}
fn subtype_of(frame: &[u8]) -> Result<u8, StationError> {
    let control = u16::from_le_bytes(
        frame
            .get(..2)
            .ok_or(StationError::Truncated)?
            .try_into()
            .map_err(|_| StationError::Truncated)?,
    );
    if control & 0x000c != 0 {
        return Err(StationError::UnsupportedFrame);
    }
    Ok(((control >> 4) & 0x0f) as u8)
}
pub fn parse_authentication(
    frame: &[u8],
    bssid: &Bssid,
    station: &[u8; 6],
) -> Result<(), StationError> {
    if subtype_of(frame)? != SUBTYPE_AUTHENTICATION {
        return Err(StationError::UnsupportedFrame);
    }
    let receiver = address_of(frame, 4)?;
    let source = address_of(frame, 10)?;
    let frame_bssid = address_of(frame, 16)?;
    if frame_bssid != bssid.0 || source != bssid.0 || receiver != *station {
        return Err(StationError::WrongPeer);
    }
    if frame.len() < 30 {
        return Err(StationError::Truncated);
    }
    let algorithm = u16::from_le_bytes([frame[24], frame[25]]);
    let sequence = u16::from_le_bytes([frame[26], frame[27]]);
    let status = u16::from_le_bytes([frame[28], frame[29]]);
    if algorithm != AUTH_OPEN_SYSTEM || sequence != AUTH_SEQUENCE_RESPONSE {
        return Err(StationError::InvalidFrame);
    }
    if status != AUTH_SUCCESS {
        return Err(StationError::AuthenticationRejected(status));
    }
    Ok(())
}
pub fn parse_association_response(
    frame: &[u8],
    bssid: &Bssid,
    station: &[u8; 6],
) -> Result<(u16, u16), StationError> {
    if subtype_of(frame)? != SUBTYPE_ASSOC_RESPONSE {
        return Err(StationError::UnsupportedFrame);
    }
    let receiver = address_of(frame, 4)?;
    let source = address_of(frame, 10)?;
    let frame_bssid = address_of(frame, 16)?;
    if frame_bssid != bssid.0 || source != bssid.0 || receiver != *station {
        return Err(StationError::WrongPeer);
    }
    if frame.len() < 30 {
        return Err(StationError::Truncated);
    }
    let status = u16::from_le_bytes([frame[26], frame[27]]);
    let aid = u16::from_le_bytes([frame[28], frame[29]]) & 0x3fff;
    if status != STATUS_SUCCESS {
        return Err(StationError::AssociationRejected(status));
    }
    Ok((aid, status))
}
pub fn is_deauthentication(frame: &[u8]) -> bool {
    matches!(subtype_of(frame), Ok(SUBTYPE_DEAUTHENTICATION))
}
pub fn candidate_rank(beacon: &Beacon<'_>, ssid: &[u8]) -> Option<i32> {
    if beacon.ssid != ssid {
        return None;
    }
    let security = beacon.security?;
    if !security.ccmp || !security.psk {
        return None;
    }
    if security.tkip || security.sae {
        return None;
    }
    if security.management_frame_protection_required {
        return None;
    }
    if beacon.capability & CAPABILITY_PRIVACY == 0 {
        return None;
    }
    if beacon.channel.is_none() {
        return None;
    }
    Some(rssi_rank(0))
}
pub const fn rssi_rank(rssi_dbm: i32) -> i32 {
    rssi_dbm
}
pub struct AccessPoint {
    pub bssid: Bssid,
    pub ssid: [u8; 32],
    pub ssid_len: usize,
    pub channel: u8,
    pub security: SecuritySuites,
}
pub fn select_candidate(beacons: &[Beacon<'_>], ssid: &[u8]) -> Option<AccessPoint> {
    let mut best: Option<(i32, usize)> = None;
    for (index, beacon) in beacons.iter().enumerate() {
        if let Some(rank) = candidate_rank(beacon, ssid) {
            if best.is_none_or(|(best_rank, _)| rank > best_rank) {
                best = Some((rank, index));
            }
        }
    }
    let (_, index) = best?;
    let beacon = &beacons[index];
    let mut ap = AccessPoint {
        bssid: beacon.bssid,
        ssid: [0u8; 32],
        ssid_len: 0,
        channel: beacon.channel.unwrap_or(0),
        security: beacon.security?,
    };
    ap.ssid_len = core::cmp::min(beacon.ssid.len(), 32);
    ap.ssid[..ap.ssid_len].copy_from_slice(&beacon.ssid[..ap.ssid_len]);
    Some(ap)
}
pub fn frame_from_beacon_bytes(frame: &[u8]) -> Result<Beacon<'_>, FrameError> {
    Beacon::parse(frame)
}
#[cfg(test)]
mod tests {
    use super::*;
    const BSSID: [u8; 6] = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
    const STATION: [u8; 6] = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
    #[test]
    fn probe_request_contains_ssid_and_rates() {
        let mut out = [0u8; MAX_FRAME];
        let length = build_probe_request(&mut out, &STATION, b"IEEE", 4).unwrap();
        assert_eq!(&out[0..2], &[0x40, 0x00]);
        assert_eq!(&out[4..10], &[0xff; 6]);
        assert_eq!(&out[10..16], &STATION);
        assert_eq!(&out[16..22], &[0xff; 6]);
        assert_eq!(out[24], IE_SSID);
        assert_eq!(out[25], 4);
        assert_eq!(&out[26..30], b"IEEE");
        assert_eq!(out[30], IE_RATES);
        assert_eq!(length, 40);
    }
    #[test]
    fn authentication_request_is_open_system() {
        let mut out = [0u8; MAX_FRAME];
        let length = build_authentication(&mut out, &STATION, &BSSID, 8).unwrap();
        assert_eq!(length, 30);
        assert_eq!(&out[0..2], &[0xb0, 0x00]);
        assert_eq!(&out[4..10], &BSSID);
        assert_eq!(&out[10..16], &STATION);
        assert_eq!(&out[24..30], &[0x00, 0x00, 0x01, 0x00, 0x00, 0x00]);
    }
    #[test]
    fn association_request_carries_rsn_psk() {
        let mut out = [0u8; MAX_FRAME];
        let length =
            build_association_request(&mut out, &STATION, &BSSID, b"IEEE", Some(6), true, 12)
                .unwrap();
        assert_eq!(&out[0..2], &[0x00, 0x00]);
        assert_eq!(
            u16::from_le_bytes([out[24], out[25]]),
            CAPABILITY_ESS | CAPABILITY_PRIVACY
        );
        assert_eq!(out[28], IE_SSID);
        assert_eq!(out[29], 4);
        let rsn_offset = 28 + 2 + 4 + 2 + 8 + 3;
        assert_eq!(out[rsn_offset], IE_RSN);
        assert_eq!(out[rsn_offset + 1], 18);
        assert_eq!(&out[rsn_offset + 4..rsn_offset + 8], &[0x00, 0x0f, 0xac, 4]);
        assert_eq!(
            &out[rsn_offset + 10..rsn_offset + 14],
            &[0x00, 0x0f, 0xac, 4]
        );
        assert_eq!(
            &out[rsn_offset + 16..rsn_offset + 20],
            &[0x00, 0x0f, 0xac, 2]
        );
        assert_eq!(length, rsn_offset + 20);
    }
    #[test]
    fn authentication_response_is_validated() {
        let mut frame = [0u8; MAX_FRAME];
        frame[0..2]
            .copy_from_slice(&(management_control(SUBTYPE_AUTHENTICATION) << 0).to_le_bytes());
        let offset = put_header(
            &mut frame,
            management_control(SUBTYPE_AUTHENTICATION),
            0,
            &STATION,
            &BSSID,
            &BSSID,
            8,
        )
        .unwrap();
        frame[offset..offset + 2].copy_from_slice(&AUTH_OPEN_SYSTEM.to_le_bytes());
        frame[offset + 2..offset + 4].copy_from_slice(&AUTH_SEQUENCE_RESPONSE.to_le_bytes());
        frame[offset + 4..offset + 6].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(
            parse_authentication(&frame, &Bssid(BSSID), &STATION),
            Ok(())
        );
        frame[offset + 5] = 1;
        assert_eq!(
            parse_authentication(&frame, &Bssid(BSSID), &STATION),
            Err(StationError::AuthenticationRejected(256))
        );
    }
    #[test]
    fn association_response_status_is_checked() {
        let mut frame = [0u8; MAX_FRAME];
        let offset = put_header(
            &mut frame,
            management_control(SUBTYPE_ASSOC_RESPONSE),
            0,
            &STATION,
            &BSSID,
            &BSSID,
            8,
        )
        .unwrap();
        frame[offset..offset + 2].copy_from_slice(&CAPABILITY_ESS.to_le_bytes());
        frame[offset + 2..offset + 4].copy_from_slice(&0u16.to_le_bytes());
        frame[offset + 4..offset + 6].copy_from_slice(&0u16.to_le_bytes());
        frame[offset + 6..offset + 8].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(
            parse_association_response(&frame, &Bssid(BSSID), &STATION),
            Ok((0, 0))
        );
        frame[offset + 2] = 12;
        assert_eq!(
            parse_association_response(&frame, &Bssid(BSSID), &STATION),
            Err(StationError::AssociationRejected(12))
        );
    }
    #[test]
    fn candidate_selection_requires_ccmp_psk_and_rejects_mfp() {
        let security = SecuritySuites {
            ccmp: true,
            tkip: false,
            psk: true,
            sae: false,
            management_frame_protection_capable: false,
            management_frame_protection_required: false,
        };
        let beacon = Beacon {
            bssid: Bssid(BSSID),
            ssid: b"IEEE",
            channel: Some(6),
            security: Some(security),
            capability: CAPABILITY_ESS | CAPABILITY_PRIVACY,
            beacon_interval: 100,
        };
        assert!(candidate_rank(&beacon, b"IEEE").is_some());
        assert_eq!(candidate_rank(&beacon, b"IEEE "), None);
        let mut mfp = security;
        mfp.management_frame_protection_required = true;
        let beacon_mfp = Beacon {
            security: Some(mfp),
            ..beacon
        };
        assert_eq!(candidate_rank(&beacon_mfp, b"IEEE"), None);
        let mut open = security;
        open.psk = false;
        let beacon_open = Beacon {
            security: Some(open),
            ..beacon
        };
        assert_eq!(candidate_rank(&beacon_open, b"IEEE"), None);
        let beacon_no_ie = Beacon {
            security: None,
            ..beacon
        };
        assert_eq!(candidate_rank(&beacon_no_ie, b"IEEE"), None);
        let ap = select_candidate(&[beacon], b"IEEE").unwrap();
        assert_eq!(ap.bssid, Bssid(BSSID));
        assert_eq!(ap.channel, 6);
        assert_eq!(&ap.ssid[..ap.ssid_len], b"IEEE");
    }
    #[test]
    fn deauthentication_detection() {
        let mut frame = [0u8; MAX_FRAME];
        let _ = put_header(
            &mut frame,
            management_control(SUBTYPE_DEAUTHENTICATION),
            0,
            &STATION,
            &BSSID,
            &BSSID,
            1,
        );
        assert!(is_deauthentication(&frame));
        let mut beacon = [0u8; MAX_FRAME];
        let _ = put_header(
            &mut beacon,
            management_control(SUBTYPE_BEACON),
            0,
            &STATION,
            &BSSID,
            &BSSID,
            1,
        );
        assert!(!is_deauthentication(&beacon));
    }
}
