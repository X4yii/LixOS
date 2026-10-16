use alloc::vec::Vec;
use lix_wifi::{Beacon, Bssid, SecuritySuites};
extern crate alloc;
pub type InterfaceId = u32;
pub type ProfileId = u64;
pub type SecretId = u64;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Security {
    Open,
    Wpa2Personal,
    Wpa3Personal,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WifiProfile {
    pub id: ProfileId,
    pub ssid: Vec<u8>,
    pub security: Security,
    pub secret: Option<SecretId>,
    pub priority: i32,
    pub auto_connect: bool,
}
impl WifiProfile {
    pub const fn validate(&self) -> Result<(), ManagerError> {
        if self.ssid.is_empty() || self.ssid.len() > 32 {
            return Err(ManagerError::InvalidProfile);
        }
        match (self.security, self.secret) {
            (Security::Open, None) | (Security::Wpa2Personal | Security::Wpa3Personal, Some(_)) => {
                Ok(())
            }
            _ => Err(ManagerError::InvalidProfile),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkState {
    Down,
    Scanning,
    Associating(ProfileId),
    Authenticating(ProfileId),
    ConfiguringIpv4(ProfileId),
    Online(ProfileId),
    Disconnecting,
    Failed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagerError {
    InvalidProfile,
    UnknownInterface,
    BackendFailure,
    UnsupportedSecurity,
}
pub trait WifiBackend {
    fn scan(&mut self, interface: InterfaceId) -> Result<(), ManagerError>;
    fn associate(
        &mut self,
        interface: InterfaceId,
        profile: &WifiProfile,
    ) -> Result<(), ManagerError>;
    fn disconnect(&mut self, interface: InterfaceId) -> Result<(), ManagerError>;
}
pub struct ManagedInterface {
    pub id: InterfaceId,
    pub state: LinkState,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoundNetwork {
    pub interface: InterfaceId,
    pub bssid: Bssid,
    pub ssid: Vec<u8>,
    pub channel: Option<u8>,
    pub security: Option<SecuritySuites>,
    pub rssi_dbm: i8,
}
pub struct NetworkManager<B> {
    backend: B,
    profiles: Vec<WifiProfile>,
    interfaces: Vec<ManagedInterface>,
    scan_results: Vec<FoundNetwork>,
}
impl<B: WifiBackend> NetworkManager<B> {
    pub const fn new(backend: B) -> Self {
        Self {
            backend,
            profiles: Vec::new(),
            interfaces: Vec::new(),
            scan_results: Vec::new(),
        }
    }
    pub fn add_interface(&mut self, id: InterfaceId) {
        if !self.interfaces.iter().any(|interface| interface.id == id) {
            self.interfaces.push(ManagedInterface {
                id,
                state: LinkState::Down,
            });
        }
    }
    pub fn add_profile(&mut self, profile: WifiProfile) -> Result<(), ManagerError> {
        profile.validate()?;
        self.profiles.retain(|old| old.id != profile.id);
        self.profiles.push(profile);
        self.profiles.sort_by(|a, b| b.priority.cmp(&a.priority));
        Ok(())
    }
    pub fn remove_profile(&mut self, id: ProfileId) -> Result<bool, ManagerError> {
        for interface in &mut self.interfaces {
            if matches!(interface.state, LinkState::Online(profile) | LinkState::Associating(profile) | LinkState::Authenticating(profile) | LinkState::ConfiguringIpv4(profile) if profile == id)
            {
                self.backend.disconnect(interface.id)?;
                interface.state = LinkState::Down;
            }
        }
        let before = self.profiles.len();
        self.profiles.retain(|profile| profile.id != id);
        Ok(self.profiles.len() != before)
    }
    pub fn scan(&mut self, id: InterfaceId) -> Result<(), ManagerError> {
        let interface = self
            .interfaces
            .iter_mut()
            .find(|interface| interface.id == id)
            .ok_or(ManagerError::UnknownInterface)?;
        self.backend.scan(id)?;
        self.scan_results.retain(|network| network.interface != id);
        interface.state = LinkState::Scanning;
        Ok(())
    }
    pub fn connect(&mut self, id: InterfaceId, profile_id: ProfileId) -> Result<(), ManagerError> {
        let profile = self
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .ok_or(ManagerError::InvalidProfile)?
            .clone();
        let interface = self
            .interfaces
            .iter_mut()
            .find(|interface| interface.id == id)
            .ok_or(ManagerError::UnknownInterface)?;
        if profile.security == Security::Wpa3Personal {
            return Err(ManagerError::UnsupportedSecurity);
        }
        interface.state = LinkState::Associating(profile_id);
        if self.backend.associate(id, &profile).is_err() {
            interface.state = LinkState::Failed;
            return Err(ManagerError::BackendFailure);
        }
        interface.state = LinkState::Authenticating(profile_id);
        Ok(())
    }
    pub fn ingest_beacon(
        &mut self,
        id: InterfaceId,
        frame: &[u8],
        rssi_dbm: i8,
    ) -> Result<(), ManagerError> {
        if !self.interfaces.iter().any(|interface| interface.id == id) {
            return Err(ManagerError::UnknownInterface);
        }
        let beacon = Beacon::parse(frame).map_err(|_| ManagerError::BackendFailure)?;
        let found = FoundNetwork {
            interface: id,
            bssid: beacon.bssid,
            ssid: beacon.ssid.to_vec(),
            channel: beacon.channel,
            security: beacon.security,
            rssi_dbm,
        };
        if let Some(old) = self
            .scan_results
            .iter_mut()
            .find(|old| old.interface == id && old.bssid == found.bssid)
        {
            *old = found;
        } else {
            self.scan_results.push(found);
        }
        Ok(())
    }
    pub fn scan_results(&self, id: InterfaceId) -> impl Iterator<Item = &FoundNetwork> {
        self.scan_results
            .iter()
            .filter(move |network| network.interface == id)
    }
    pub fn autoconnect_candidate(&self, id: InterfaceId) -> Option<ProfileId> {
        self.profiles.iter().find_map(|profile| {
            if !profile.auto_connect || profile.security != Security::Wpa2Personal {
                return None;
            }
            let visible = self.scan_results.iter().any(|network| {
                network.interface == id
                    && network.ssid == profile.ssid
                    && network
                        .security
                        .is_some_and(|security| security.ccmp && security.psk)
            });
            visible.then_some(profile.id)
        })
    }
    pub fn authentication_complete(
        &mut self,
        id: InterfaceId,
        success: bool,
    ) -> Result<(), ManagerError> {
        let interface = self
            .interfaces
            .iter_mut()
            .find(|interface| interface.id == id)
            .ok_or(ManagerError::UnknownInterface)?;
        let LinkState::Authenticating(profile) = interface.state else {
            return Err(ManagerError::BackendFailure);
        };
        interface.state = if success {
            LinkState::ConfiguringIpv4(profile)
        } else {
            LinkState::Failed
        };
        Ok(())
    }
    pub fn ipv4_configured(&mut self, id: InterfaceId, success: bool) -> Result<(), ManagerError> {
        let interface = self
            .interfaces
            .iter_mut()
            .find(|interface| interface.id == id)
            .ok_or(ManagerError::UnknownInterface)?;
        let LinkState::ConfiguringIpv4(profile) = interface.state else {
            return Err(ManagerError::BackendFailure);
        };
        interface.state = if success {
            LinkState::Online(profile)
        } else {
            LinkState::Failed
        };
        Ok(())
    }
    pub fn state(&self, id: InterfaceId) -> Option<LinkState> {
        self.interfaces
            .iter()
            .find(|interface| interface.id == id)
            .map(|interface| interface.state)
    }
    pub const fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
}
