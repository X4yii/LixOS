use alloc::vec::Vec;
use lix_bluetooth::{ControllerState, Profile};
extern crate alloc;
pub type ControllerId = u32;
pub type DeviceId = u64;
pub type SecretId = u64;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceState {
    Discovered,
    Pairing,
    Paired,
    Connected,
    Blocked,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bond {
    pub device: DeviceId,
    pub secret: SecretId,
    pub allow_hid: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    UnknownController,
    UnknownDevice,
    NotReady,
    PairingUnavailable,
    ProfileDenied,
}
pub trait HciBackend {
    fn reset_and_initialize(&mut self, controller: ControllerId) -> Result<(), Error>;
    fn disconnect(&mut self, controller: ControllerId, device: DeviceId) -> Result<(), Error>;
}
pub struct BluetoothManager<B> {
    backend: B,
    controllers: Vec<(ControllerId, ControllerState)>,
    bonds: Vec<Bond>,
}
impl<B: HciBackend> BluetoothManager<B> {
    pub const fn new(backend: B) -> Self {
        Self {
            backend,
            controllers: Vec::new(),
            bonds: Vec::new(),
        }
    }
    pub fn add_controller(&mut self, id: ControllerId) {
        if !self.controllers.iter().any(|(existing, _)| *existing == id) {
            self.controllers.push((id, ControllerState::Discovered));
        }
    }
    pub fn initialize(&mut self, id: ControllerId) -> Result<(), Error> {
        let (_, state) = self
            .controllers
            .iter_mut()
            .find(|(existing, _)| *existing == id)
            .ok_or(Error::UnknownController)?;
        *state = ControllerState::Initializing;
        match self.backend.reset_and_initialize(id) {
            Ok(()) => {
                *state = ControllerState::Ready;
                Ok(())
            }
            Err(error) => {
                *state = ControllerState::Failed;
                Err(error)
            }
        }
    }
    pub fn record_bond(&mut self, bond: Bond) {
        self.bonds.retain(|existing| existing.device != bond.device);
        self.bonds.push(bond);
    }
    pub fn forget(&mut self, controller: ControllerId, device: DeviceId) -> Result<bool, Error> {
        if !self
            .controllers
            .iter()
            .any(|(id, state)| *id == controller && *state == ControllerState::Ready)
        {
            return Err(Error::NotReady);
        }
        self.backend.disconnect(controller, device)?;
        let before = self.bonds.len();
        self.bonds.retain(|bond| bond.device != device);
        Ok(self.bonds.len() != before)
    }
    pub fn authorize_profile(&self, device: DeviceId, profile: Profile) -> Result<(), Error> {
        let bond = self
            .bonds
            .iter()
            .find(|bond| bond.device == device)
            .ok_or(Error::UnknownDevice)?;
        match profile {
            Profile::Hid if bond.allow_hid => Ok(()),
            _ => Err(Error::ProfileDenied),
        }
    }
}
