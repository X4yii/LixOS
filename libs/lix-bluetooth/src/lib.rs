#![no_std]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PacketError {
    Empty,
    InvalidType,
    Truncated,
    LengthMismatch,
    InvalidHandle,
    OutputTooSmall,
    ParametersTooLong,
}
pub const HCI_COMMAND_RESET: u16 = 0x0c03;
pub const HCI_COMMAND_READ_LOCAL_VERSION: u16 = 0x1001;
pub fn encode_command(
    opcode: u16,
    parameters: &[u8],
    output: &mut [u8],
) -> Result<usize, PacketError> {
    if parameters.len() > u8::MAX as usize {
        return Err(PacketError::ParametersTooLong);
    }
    let length = parameters.len() + 4;
    if output.len() < length {
        return Err(PacketError::OutputTooSmall);
    }
    output[0] = 0x01;
    output[1..3].copy_from_slice(&opcode.to_le_bytes());
    output[3] = parameters.len() as u8;
    output[4..length].copy_from_slice(parameters);
    Ok(length)
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Command<'a> {
    pub opcode: u16,
    pub parameters: &'a [u8],
}
impl<'a> Command<'a> {
    pub fn parse(packet: &'a [u8]) -> Result<Self, PacketError> {
        if packet.is_empty() {
            return Err(PacketError::Empty);
        }
        if packet[0] != 0x01 {
            return Err(PacketError::InvalidType);
        }
        if packet.len() < 4 {
            return Err(PacketError::Truncated);
        }
        let length = packet[3] as usize;
        if packet.len() != length + 4 {
            return Err(PacketError::LengthMismatch);
        }
        Ok(Self {
            opcode: u16::from_le_bytes([packet[1], packet[2]]),
            parameters: &packet[4..],
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Event<'a> {
    pub code: u8,
    pub parameters: &'a [u8],
}
impl<'a> Event<'a> {
    pub fn parse(packet: &'a [u8]) -> Result<Self, PacketError> {
        if packet.is_empty() {
            return Err(PacketError::Empty);
        }
        if packet.len() < 2 {
            return Err(PacketError::Truncated);
        }
        let length = packet[1] as usize;
        if packet.len() != length + 2 {
            return Err(PacketError::LengthMismatch);
        }
        Ok(Self {
            code: packet[0],
            parameters: &packet[2..],
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandComplete<'a> {
    pub command_packets: u8,
    pub opcode: u16,
    pub return_parameters: &'a [u8],
}
impl<'a> CommandComplete<'a> {
    pub fn parse(event: Event<'a>) -> Result<Self, PacketError> {
        if event.code != 0x0e {
            return Err(PacketError::InvalidType);
        }
        if event.parameters.len() < 3 {
            return Err(PacketError::Truncated);
        }
        Ok(Self {
            command_packets: event.parameters[0],
            opcode: u16::from_le_bytes([event.parameters[1], event.parameters[2]]),
            return_parameters: &event.parameters[3..],
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandStatus {
    pub status: u8,
    pub command_packets: u8,
    pub opcode: u16,
}
impl CommandStatus {
    pub const fn parse(event: Event<'_>) -> Result<Self, PacketError> {
        if event.code != 0x0f {
            return Err(PacketError::InvalidType);
        }
        if event.parameters.len() != 4 {
            return Err(PacketError::LengthMismatch);
        }
        Ok(Self {
            status: event.parameters[0],
            command_packets: event.parameters[1],
            opcode: u16::from_le_bytes([event.parameters[2], event.parameters[3]]),
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AclData<'a> {
    pub handle: u16,
    pub packet_boundary: u8,
    pub broadcast: u8,
    pub payload: &'a [u8],
}
impl<'a> AclData<'a> {
    pub fn parse(packet: &'a [u8]) -> Result<Self, PacketError> {
        if packet.is_empty() {
            return Err(PacketError::Empty);
        }
        if packet[0] != 0x02 {
            return Err(PacketError::InvalidType);
        }
        if packet.len() < 5 {
            return Err(PacketError::Truncated);
        }
        let header = u16::from_le_bytes([packet[1], packet[2]]);
        let length = u16::from_le_bytes([packet[3], packet[4]]) as usize;
        if packet.len() != length + 5 {
            return Err(PacketError::LengthMismatch);
        }
        let handle = header & 0x0fff;
        if handle > 0x0eff {
            return Err(PacketError::InvalidHandle);
        }
        Ok(Self {
            handle,
            packet_boundary: ((header >> 12) & 3) as u8,
            broadcast: ((header >> 14) & 3) as u8,
            payload: &packet[5..],
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControllerState {
    Absent,
    Discovered,
    Initializing,
    Ready,
    Failed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Profile {
    Hid,
    Gatt,
    Audio,
}
