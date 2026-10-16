#![no_std]
pub const DESCRIPTOR_DEVICE: u8 = 1;
pub const DESCRIPTOR_CONFIGURATION: u8 = 2;
pub const DESCRIPTOR_INTERFACE: u8 = 4;
pub const DESCRIPTOR_ENDPOINT: u8 = 5;
pub const DESCRIPTOR_HID: u8 = 0x21;
pub const CLASS_HID: u8 = 3;
pub const CLASS_MASS_STORAGE: u8 = 8;
pub const CLASS_WIRELESS_CONTROLLER: u8 = 0xe0;
pub const SUBCLASS_RF_CONTROLLER: u8 = 0x01;
pub const PROTOCOL_BLUETOOTH: u8 = 0x01;
pub const HID_SUBCLASS_BOOT: u8 = 1;
pub const HID_PROTOCOL_KEYBOARD: u8 = 1;
pub const HID_PROTOCOL_MOUSE: u8 = 2;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorError {
    TruncatedHeader,
    InvalidLength,
    TruncatedDescriptor,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Descriptor<'a> {
    pub kind: u8,
    pub bytes: &'a [u8],
}
pub struct DescriptorIter<'a> {
    remaining: &'a [u8],
    failed: bool,
}
impl<'a> DescriptorIter<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self {
            remaining: bytes,
            failed: false,
        }
    }
}
impl<'a> Iterator for DescriptorIter<'a> {
    type Item = Result<Descriptor<'a>, DescriptorError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.remaining.is_empty() {
            return None;
        }
        if self.remaining.len() < 2 {
            self.failed = true;
            return Some(Err(DescriptorError::TruncatedHeader));
        }
        let length = self.remaining[0] as usize;
        if length < 2 {
            self.failed = true;
            return Some(Err(DescriptorError::InvalidLength));
        }
        if length > self.remaining.len() {
            self.failed = true;
            return Some(Err(DescriptorError::TruncatedDescriptor));
        }
        let (current, rest) = self.remaining.split_at(length);
        self.remaining = rest;
        Some(Ok(Descriptor {
            kind: current[1],
            bytes: current,
        }))
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterfaceDescriptor {
    pub number: u8,
    pub alternate_setting: u8,
    pub endpoint_count: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceDescriptor {
    pub usb_version: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub max_packet_size_zero: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_version: u16,
    pub manufacturer_index: u8,
    pub product_index: u8,
    pub serial_index: u8,
    pub configuration_count: u8,
}
impl DeviceDescriptor {
    pub const fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 18 || bytes[0] < 18 || bytes[1] != DESCRIPTOR_DEVICE {
            return None;
        }
        Some(Self {
            usb_version: u16::from_le_bytes([bytes[2], bytes[3]]),
            device_class: bytes[4],
            device_subclass: bytes[5],
            device_protocol: bytes[6],
            max_packet_size_zero: bytes[7],
            vendor_id: u16::from_le_bytes([bytes[8], bytes[9]]),
            product_id: u16::from_le_bytes([bytes[10], bytes[11]]),
            device_version: u16::from_le_bytes([bytes[12], bytes[13]]),
            manufacturer_index: bytes[14],
            product_index: bytes[15],
            serial_index: bytes[16],
            configuration_count: bytes[17],
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndpointDescriptor {
    pub address: u8,
    pub attributes: u8,
    pub max_packet_size: u16,
    pub interval: u8,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BluetoothHciEndpoints {
    pub interface: InterfaceDescriptor,
    pub configuration_value: u8,
    pub event_in: EndpointDescriptor,
    pub acl_in: EndpointDescriptor,
    pub acl_out: EndpointDescriptor,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootHidKind {
    Keyboard,
    Mouse,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootHidEndpoint {
    pub kind: BootHidKind,
    pub configuration_value: u8,
    pub interface: InterfaceDescriptor,
    pub endpoint: EndpointDescriptor,
}
pub fn find_boot_hid_endpoint(bytes: &[u8]) -> Result<Option<BootHidEndpoint>, DescriptorError> {
    let mut interface = None;
    let mut configuration_value = 1;
    for item in DescriptorIter::new(bytes) {
        let descriptor = item?;
        if descriptor.kind == DESCRIPTOR_CONFIGURATION && descriptor.bytes.len() >= 6 {
            configuration_value = descriptor.bytes[5];
        }
        if let Some(parsed) = InterfaceDescriptor::parse(descriptor) {
            interface = if parsed.class == CLASS_HID && parsed.subclass == HID_SUBCLASS_BOOT {
                let kind = match parsed.protocol {
                    HID_PROTOCOL_KEYBOARD => Some(BootHidKind::Keyboard),
                    HID_PROTOCOL_MOUSE => Some(BootHidKind::Mouse),
                    _ => None,
                };
                kind.map(|kind| (parsed, kind))
            } else {
                None
            };
            continue;
        }
        let Some((active_interface, kind)) = interface else {
            continue;
        };
        if let Some(endpoint) = EndpointDescriptor::parse(descriptor)
            && endpoint.is_in()
            && endpoint.transfer_type() == 3
        {
            return Ok(Some(BootHidEndpoint {
                kind,
                configuration_value,
                interface: active_interface,
                endpoint,
            }));
        }
    }
    Ok(None)
}
pub fn find_bluetooth_hci_endpoints(
    bytes: &[u8],
) -> Result<Option<BluetoothHciEndpoints>, DescriptorError> {
    let mut interface = None;
    let mut configuration_value = 1;
    let mut event_in = None;
    let mut acl_in = None;
    let mut acl_out = None;
    for item in DescriptorIter::new(bytes) {
        let descriptor = item?;
        if descriptor.kind == DESCRIPTOR_CONFIGURATION && descriptor.bytes.len() >= 6 {
            configuration_value = descriptor.bytes[5];
        }
        if let Some(parsed) = InterfaceDescriptor::parse(descriptor) {
            interface = (parsed.class == CLASS_WIRELESS_CONTROLLER
                && parsed.subclass == SUBCLASS_RF_CONTROLLER
                && parsed.protocol == PROTOCOL_BLUETOOTH)
                .then_some(parsed);
            event_in = None;
            acl_in = None;
            acl_out = None;
            continue;
        }
        if interface.is_none() {
            continue;
        }
        let Some(endpoint) = EndpointDescriptor::parse(descriptor) else {
            continue;
        };
        if endpoint.max_packet_size == 0 {
            return Err(DescriptorError::InvalidLength);
        }
        match (endpoint.transfer_type(), endpoint.is_in()) {
            (3, true) => event_in = Some(endpoint),
            (2, true) => acl_in = Some(endpoint),
            (2, false) => acl_out = Some(endpoint),
            _ => {}
        }
        if let (Some(interface), Some(event_in), Some(acl_in), Some(acl_out)) =
            (interface, event_in, acl_in, acl_out)
        {
            return Ok(Some(BluetoothHciEndpoints {
                interface,
                configuration_value,
                event_in,
                acl_in,
                acl_out,
            }));
        }
    }
    Ok(None)
}
impl EndpointDescriptor {
    pub const fn parse(descriptor: Descriptor<'_>) -> Option<Self> {
        let b = descriptor.bytes;
        if descriptor.kind != DESCRIPTOR_ENDPOINT || b.len() < 7 {
            return None;
        }
        Some(Self {
            address: b[2],
            attributes: b[3],
            max_packet_size: u16::from_le_bytes([b[4], b[5]]) & 0x07ff,
            interval: b[6],
        })
    }
    pub const fn is_in(self) -> bool {
        self.address & 0x80 != 0
    }
    pub const fn transfer_type(self) -> u8 {
        self.attributes & 3
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputEvent {
    pub kind: u16,
    pub code: u32,
    pub value: i32,
}
pub const INPUT_KEY: u16 = 1;
pub const INPUT_REL: u16 = 2;
pub const INPUT_BUTTON: u16 = 3;
pub const REL_X: u32 = 0;
pub const REL_Y: u32 = 1;
pub const BUTTON_LEFT: u32 = 0;
pub const BUTTON_RIGHT: u32 = 1;
pub const BUTTON_MIDDLE: u32 = 2;
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BootKeyboard {
    modifiers: u8,
    keys: [u8; 6],
}
impl BootKeyboard {
    pub fn push_report(&mut self, report: &[u8]) -> ([InputEvent; 20], usize) {
        let mut events = [InputEvent {
            kind: 0,
            code: 0,
            value: 0,
        }; 20];
        if report.len() < 8 {
            return (events, 0);
        }
        let modifiers = report[0];
        let keys: [u8; 6] = report[2..8].try_into().unwrap_or([0; 6]);
        if keys.iter().any(|key| (1..=3).contains(key)) {
            return (events, 0);
        }
        let mut count = 0;
        for bit in 0..8 {
            let old = self.modifiers & (1 << bit) != 0;
            let new = modifiers & (1 << bit) != 0;
            if old != new {
                events[count] = InputEvent {
                    kind: INPUT_KEY,
                    code: (0xe0 + bit) as u32,
                    value: i32::from(new),
                };
                count += 1;
            }
        }
        for key in self.keys {
            if key != 0 && !keys.contains(&key) {
                events[count] = InputEvent {
                    kind: INPUT_KEY,
                    code: u32::from(key),
                    value: 0,
                };
                count += 1;
            }
        }
        for key in keys {
            if key != 0 && !self.keys.contains(&key) {
                events[count] = InputEvent {
                    kind: INPUT_KEY,
                    code: u32::from(key),
                    value: 1,
                };
                count += 1;
            }
        }
        self.modifiers = modifiers;
        self.keys = keys;
        (events, count)
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BootMouse {
    buttons: u8,
}
impl BootMouse {
    pub fn push_report(&mut self, report: &[u8]) -> ([InputEvent; 5], usize) {
        let mut events = [InputEvent {
            kind: 0,
            code: 0,
            value: 0,
        }; 5];
        if report.len() < 3 {
            return (events, 0);
        }
        let buttons = report[0];
        let mut count = 0;
        for bit in 0..3 {
            let old = self.buttons & (1 << bit) != 0;
            let new = buttons & (1 << bit) != 0;
            if old != new {
                events[count] = InputEvent {
                    kind: INPUT_BUTTON,
                    code: bit as u32,
                    value: i32::from(new),
                };
                count += 1;
            }
        }
        let x = report[1] as i8;
        let y = report[2] as i8;
        if x != 0 {
            events[count] = InputEvent {
                kind: INPUT_REL,
                code: REL_X,
                value: i32::from(x),
            };
            count += 1;
        }
        if y != 0 {
            events[count] = InputEvent {
                kind: INPUT_REL,
                code: REL_Y,
                value: i32::from(y),
            };
            count += 1;
        }
        self.buttons = buttons;
        (events, count)
    }
}
impl InterfaceDescriptor {
    pub const fn parse(descriptor: Descriptor<'_>) -> Option<Self> {
        let b = descriptor.bytes;
        if descriptor.kind != DESCRIPTOR_INTERFACE || b.len() < 9 {
            return None;
        }
        Some(Self {
            number: b[2],
            alternate_setting: b[3],
            endpoint_count: b[4],
            class: b[5],
            subclass: b[6],
            protocol: b[7],
        })
    }
    pub const fn is_hid(self) -> bool {
        self.class == CLASS_HID
    }
    pub const fn is_mass_storage(self) -> bool {
        self.class == CLASS_MASS_STORAGE
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn iterates_and_classifies_hid_interface() {
        let descriptors = [9, 4, 0, 0, 1, 3, 1, 1, 0, 7, 5, 0x81, 3, 8, 0, 10];
        let mut iter = DescriptorIter::new(&descriptors);
        let interface = InterfaceDescriptor::parse(iter.next().unwrap().unwrap()).unwrap();
        assert!(interface.is_hid());
        assert_eq!(iter.next().unwrap().unwrap().kind, DESCRIPTOR_ENDPOINT);
        assert!(iter.next().is_none());
    }
    #[test]
    fn rejects_malformed_lengths_without_panicking() {
        assert_eq!(
            DescriptorIter::new(&[1, 4]).next().unwrap(),
            Err(DescriptorError::InvalidLength)
        );
        assert_eq!(
            DescriptorIter::new(&[4, 5, 0]).next().unwrap(),
            Err(DescriptorError::TruncatedDescriptor)
        );
        assert_eq!(
            DescriptorIter::new(&[4]).next().unwrap(),
            Err(DescriptorError::TruncatedHeader)
        );
    }
}
