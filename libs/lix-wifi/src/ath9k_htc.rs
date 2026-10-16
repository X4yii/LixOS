use crate::FrameError;
pub const FIRMWARE_REQUEST: u8 = 0x30;
pub const FIRMWARE_COMPLETE_REQUEST: u8 = 0x31;
pub const AR9271_FIRMWARE_ADDRESS: u32 = 0x0050_1000;
pub const AR9271_FIRMWARE_TEXT_ADDRESS: u32 = 0x0090_3000;
pub const AR7010_FIRMWARE_TEXT_ADDRESS: u32 = 0x0090_6000;
pub const MAX_FIRMWARE_CHUNK: usize = 4096;
pub const RX_STREAM_TAG: u16 = 0x4e00;
pub const TX_STREAM_TAG: u16 = 0x697e;
pub const MAX_RX_FRAME: usize = 16 * 1024;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Chip {
    Ar9271,
    Ar7010Bridge,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbId {
    pub vendor: u16,
    pub product: u16,
    pub chip: Chip,
}
pub const USB_IDS: &[UsbId] = &[
    UsbId {
        vendor: 0x0cf3,
        product: 0x9271,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x0cf3,
        product: 0x1006,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x0846,
        product: 0x9030,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x07b8,
        product: 0x9271,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x07d1,
        product: 0x3a10,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x13d3,
        product: 0x3327,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x13d3,
        product: 0x3328,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x13d3,
        product: 0x3346,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x13d3,
        product: 0x3348,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x13d3,
        product: 0x3349,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x13d3,
        product: 0x3350,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x04ca,
        product: 0x4605,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x040d,
        product: 0x3801,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x0cf3,
        product: 0xb003,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x0cf3,
        product: 0xb002,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x057c,
        product: 0x8403,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x0471,
        product: 0x209e,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x1eda,
        product: 0x2315,
        chip: Chip::Ar9271,
    },
    UsbId {
        vendor: 0x0cf3,
        product: 0x7015,
        chip: Chip::Ar7010Bridge,
    },
    UsbId {
        vendor: 0x0cf3,
        product: 0x7010,
        chip: Chip::Ar7010Bridge,
    },
    UsbId {
        vendor: 0x0846,
        product: 0x9018,
        chip: Chip::Ar7010Bridge,
    },
    UsbId {
        vendor: 0x083a,
        product: 0xa704,
        chip: Chip::Ar7010Bridge,
    },
    UsbId {
        vendor: 0x0411,
        product: 0x017f,
        chip: Chip::Ar7010Bridge,
    },
    UsbId {
        vendor: 0x0411,
        product: 0x0197,
        chip: Chip::Ar7010Bridge,
    },
    UsbId {
        vendor: 0x04da,
        product: 0x3904,
        chip: Chip::Ar7010Bridge,
    },
    UsbId {
        vendor: 0x0930,
        product: 0x0a08,
        chip: Chip::Ar7010Bridge,
    },
];
pub fn match_usb(vendor: u16, product: u16) -> Option<Chip> {
    USB_IDS
        .iter()
        .find(|id| id.vendor == vendor && id.product == product)
        .map(|id| id.chip)
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirmwareChunk<'a> {
    pub request: u8,
    pub request_type: u8,
    pub address: u32,
    pub bytes: &'a [u8],
}
impl FirmwareChunk<'_> {
    pub fn request_value(self) -> Option<u16> {
        u16::try_from(self.address >> 8).ok()
    }
}
pub struct FirmwareUpload<'a> {
    image: &'a [u8],
    offset: usize,
    next_address: u32,
}
impl<'a> FirmwareUpload<'a> {
    pub fn new(chip: Chip, image: &'a [u8]) -> Option<Self> {
        if image.is_empty() || image.len() > 512 * 1024 {
            return None;
        }
        let address = match chip {
            Chip::Ar9271 => AR9271_FIRMWARE_ADDRESS,
            Chip::Ar7010Bridge => AR9271_FIRMWARE_ADDRESS,
        };
        address.checked_add(image.len() as u32)?;
        Some(Self {
            image,
            offset: 0,
            next_address: address,
        })
    }
    pub fn next_chunk(&mut self) -> Option<FirmwareChunk<'a>> {
        if self.offset == self.image.len() {
            return None;
        }
        let end = self
            .offset
            .saturating_add(MAX_FIRMWARE_CHUNK)
            .min(self.image.len());
        let bytes = &self.image[self.offset..end];
        let chunk = FirmwareChunk {
            request: FIRMWARE_REQUEST,
            request_type: 0x40,
            address: self.next_address,
            bytes,
        };
        self.offset = end;
        self.next_address += bytes.len() as u32;
        Some(chunk)
    }
    pub fn completion_request(chip: Chip) -> (u8, u8, u16) {
        let text = match chip {
            Chip::Ar9271 => AR9271_FIRMWARE_TEXT_ADDRESS,
            Chip::Ar7010Bridge => AR7010_FIRMWARE_TEXT_ADDRESS,
        };
        (FIRMWARE_COMPLETE_REQUEST, 0x40, (text >> 8) as u16)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RxFrame<'a> {
    pub bytes: &'a [u8],
}
pub struct RxStream<'a> {
    bytes: &'a [u8],
    offset: usize,
    failed: bool,
}
impl<'a> RxStream<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
            failed: false,
        }
    }
    pub fn unconsumed(&self) -> &'a [u8] {
        &self.bytes[self.offset..]
    }
}
impl<'a> Iterator for RxStream<'a> {
    type Item = Result<RxFrame<'a>, FrameError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.offset == self.bytes.len() {
            return None;
        }
        let Some(header) = self.bytes.get(self.offset..self.offset.saturating_add(4)) else {
            self.failed = true;
            return Some(Err(FrameError::Truncated));
        };
        let length = u16::from_le_bytes([header[0], header[1]]) as usize;
        let tag = u16::from_le_bytes([header[2], header[3]]);
        let padded = length.checked_add(3).map(|n| n & !3);
        let Some(padded) = padded else {
            self.failed = true;
            return Some(Err(FrameError::Truncated));
        };
        let payload_start = self.offset + 4;
        let Some(payload_end) = payload_start.checked_add(length) else {
            self.failed = true;
            return Some(Err(FrameError::Truncated));
        };
        let Some(next_offset) = payload_start.checked_add(padded) else {
            self.failed = true;
            return Some(Err(FrameError::Truncated));
        };
        if tag != RX_STREAM_TAG
            || length == 0
            || length > MAX_RX_FRAME
            || next_offset > self.bytes.len()
        {
            self.failed = true;
            return Some(Err(FrameError::InvalidTransportFrame));
        }
        self.offset = next_offset;
        Some(Ok(RxFrame {
            bytes: &self.bytes[payload_start..payload_end],
        }))
    }
}
pub fn encode_tx_stream(packet: &[u8], output: &mut [u8]) -> Option<usize> {
    if packet.is_empty() || packet.len() > u16::MAX as usize {
        return None;
    }
    let padded = packet.len().checked_add(3)? & !3;
    let total = 4usize.checked_add(padded)?;
    if output.len() < total {
        return None;
    }
    output[..total].fill(0);
    output[..2].copy_from_slice(&(packet.len() as u16).to_le_bytes());
    output[2..4].copy_from_slice(&TX_STREAM_TAG.to_le_bytes());
    output[4..4 + packet.len()].copy_from_slice(packet);
    Some(total)
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HtcEndpointSet {
    pub configuration_value: u8,
    pub interface_number: u8,
    pub alternate_setting: u8,
    pub wlan_in: lix_usb::EndpointDescriptor,
    pub wlan_out: lix_usb::EndpointDescriptor,
    pub reg_in: lix_usb::EndpointDescriptor,
    pub reg_out: lix_usb::EndpointDescriptor,
}
pub fn find_htc_endpoints(
    config: &[u8],
) -> Result<Option<HtcEndpointSet>, lix_usb::DescriptorError> {
    use lix_usb::{
        DESCRIPTOR_CONFIGURATION, DescriptorIter, EndpointDescriptor, InterfaceDescriptor,
    };
    let mut active_vendor_interface = false;
    let mut configuration_value = 0;
    let mut active_interface = (0, 0);
    let mut endpoints: [Option<EndpointDescriptor>; 4] = [None; 4];
    for item in DescriptorIter::new(config) {
        let d = item?;
        if d.kind == DESCRIPTOR_CONFIGURATION {
            endpoints = [None; 4];
            active_vendor_interface = false;
            configuration_value = d.bytes.get(5).copied().unwrap_or(0);
        }
        if let Some(interface) = InterfaceDescriptor::parse(d) {
            active_vendor_interface = interface.class == 0xff;
            active_interface = (interface.number, interface.alternate_setting);
            endpoints = [None; 4];
            continue;
        }
        if !active_vendor_interface {
            continue;
        }
        if let Some(ep) = EndpointDescriptor::parse(d) {
            if ep.max_packet_size == 0 {
                return Err(lix_usb::DescriptorError::InvalidLength);
            }
            let index = match (ep.transfer_type(), ep.is_in()) {
                (2, true) => 0,
                (2, false) => 1,
                (3, true) => 2,
                (3, false) => 3,
                _ => continue,
            };
            endpoints[index] = Some(ep);
            if let [Some(wlan_in), Some(wlan_out), Some(reg_in), Some(reg_out)] = endpoints {
                return Ok(Some(HtcEndpointSet {
                    configuration_value,
                    interface_number: active_interface.0,
                    alternate_setting: active_interface.1,
                    wlan_in,
                    wlan_out,
                    reg_in,
                    reg_out,
                }));
            }
        }
    }
    Ok(None)
}
