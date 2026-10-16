#![no_std]
extern crate alloc;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    Truncated,
    UnsupportedFrame,
    InvalidInformationElement,
    UnsupportedRsn,
    InvalidTransportFrame,
}
pub mod ath9k_htc;
pub mod crypto;
pub mod data;
pub mod htc;
pub mod station;
pub mod wmi;
pub mod wpa2;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bssid(pub [u8; 6]);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecuritySuites {
    pub ccmp: bool,
    pub tkip: bool,
    pub psk: bool,
    pub sae: bool,
    pub management_frame_protection_capable: bool,
    pub management_frame_protection_required: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Beacon<'a> {
    pub bssid: Bssid,
    pub ssid: &'a [u8],
    pub channel: Option<u8>,
    pub security: Option<SecuritySuites>,
    pub capability: u16,
    pub beacon_interval: u16,
}
impl<'a> Beacon<'a> {
    pub fn parse(frame: &'a [u8]) -> Result<Self, FrameError> {
        if frame.len() < 36 {
            return Err(FrameError::Truncated);
        }
        let control = u16::from_le_bytes([frame[0], frame[1]]);
        if control & 0x000c != 0 || !matches!((control >> 4) & 0x0f, 5 | 8) {
            return Err(FrameError::UnsupportedFrame);
        }
        let bssid = Bssid(
            frame[16..22]
                .try_into()
                .map_err(|_| FrameError::Truncated)?,
        );
        let beacon_interval = u16::from_le_bytes([frame[32], frame[33]]);
        let capability = u16::from_le_bytes([frame[34], frame[35]]);
        let mut ssid = None;
        let mut channel = None;
        let mut security = None;
        let mut pos = 36;
        while pos < frame.len() {
            if frame.len() - pos < 2 {
                return Err(FrameError::InvalidInformationElement);
            }
            let id = frame[pos];
            let len = frame[pos + 1] as usize;
            pos += 2;
            let end = pos
                .checked_add(len)
                .ok_or(FrameError::InvalidInformationElement)?;
            if end > frame.len() {
                return Err(FrameError::InvalidInformationElement);
            }
            let body = &frame[pos..end];
            match id {
                0 => {
                    if len > 32 {
                        return Err(FrameError::InvalidInformationElement);
                    }
                    ssid = Some(body);
                }
                3 if len == 1 => channel = Some(body[0]),
                48 => security = Some(parse_rsn(body)?),
                _ => {}
            }
            pos = end;
        }
        Ok(Self {
            bssid,
            ssid: ssid.ok_or(FrameError::InvalidInformationElement)?,
            channel,
            security,
            capability,
            beacon_interval,
        })
    }
}
fn suite_is(suite: &[u8], kind: u8) -> bool {
    suite.len() == 4 && suite[..3] == [0x00, 0x0f, 0xac] && suite[3] == kind
}
fn parse_rsn(bytes: &[u8]) -> Result<SecuritySuites, FrameError> {
    if bytes.len() < 2 + 4 + 2 + 4 + 2 + 4 {
        return Err(FrameError::UnsupportedRsn);
    }
    let mut pos = 0;
    let take = |pos: &mut usize, count: usize| -> Result<&[u8], FrameError> {
        let end = pos.checked_add(count).ok_or(FrameError::UnsupportedRsn)?;
        let value = bytes.get(*pos..end).ok_or(FrameError::UnsupportedRsn)?;
        *pos = end;
        Ok(value)
    };
    if u16::from_le_bytes(
        take(&mut pos, 2)?
            .try_into()
            .map_err(|_| FrameError::UnsupportedRsn)?,
    ) != 1
    {
        return Err(FrameError::UnsupportedRsn);
    }
    let _group_cipher = take(&mut pos, 4)?;
    let pairwise_count = u16::from_le_bytes(
        take(&mut pos, 2)?
            .try_into()
            .map_err(|_| FrameError::UnsupportedRsn)?,
    ) as usize;
    if pairwise_count == 0 || pairwise_count > 16 {
        return Err(FrameError::UnsupportedRsn);
    }
    let pairwise = take(
        &mut pos,
        pairwise_count
            .checked_mul(4)
            .ok_or(FrameError::UnsupportedRsn)?,
    )?;
    let akm_count = u16::from_le_bytes(
        take(&mut pos, 2)?
            .try_into()
            .map_err(|_| FrameError::UnsupportedRsn)?,
    ) as usize;
    if akm_count == 0 || akm_count > 16 {
        return Err(FrameError::UnsupportedRsn);
    }
    let akms = take(
        &mut pos,
        akm_count.checked_mul(4).ok_or(FrameError::UnsupportedRsn)?,
    )?;
    let mut suites = SecuritySuites {
        ccmp: pairwise.chunks_exact(4).any(|suite| suite_is(suite, 4)),
        tkip: pairwise.chunks_exact(4).any(|suite| suite_is(suite, 2)),
        psk: akms.chunks_exact(4).any(|suite| suite_is(suite, 2)),
        sae: akms.chunks_exact(4).any(|suite| suite_is(suite, 8)),
        management_frame_protection_capable: false,
        management_frame_protection_required: false,
    };
    if bytes.len().saturating_sub(pos) >= 2 {
        let caps = u16::from_le_bytes(
            take(&mut pos, 2)?
                .try_into()
                .map_err(|_| FrameError::UnsupportedRsn)?,
        );
        suites.management_frame_protection_required = caps & (1 << 6) != 0;
        suites.management_frame_protection_capable = caps & (1 << 7) != 0;
    }
    Ok(suites)
}
pub fn valid_wpa2_passphrase(passphrase: &[u8]) -> bool {
    (8..=63).contains(&passphrase.len()) && passphrase.is_ascii()
}
