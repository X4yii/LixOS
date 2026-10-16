#![cfg_attr(not(test), no_std)]
extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use authd::protocol::{self, HelloRequest, HelloResponse};
use authd::protocol::{kind as message_kind, ProtocolError};
pub const CONFIG_PATH: &str = "/etc/sessiond.conf";
pub const MAX_CONFIG_BYTES: usize = 4096;
pub const MAX_SERVICES: usize = 8;
pub const MAX_PATH_BYTES: usize = 128;
pub const MAX_SERVICE_NAME_BYTES: usize = 16;
pub const DEFAULT_MAX_RESTARTS: u32 = 3;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceSpec {
    pub name: String,
    pub path: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub services: Vec<ServiceSpec>,
    pub restart: bool,
    pub max_restarts: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    TooLarge,
    TooManyServices,
    UnknownKey,
    Malformed,
    DuplicateService,
    EmptyValue,
    NoServices,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportError {
    Unavailable,
    Closed,
    Rejected,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionEvent {
    ServiceMissing,
    ServiceDied,
    RestartIssued,
    RestartLimitReached,
    HandshakeRejected,
    HandlesUnavailable,
}
pub struct ProcessSnapshot {
    pub name: String,
    pub pid: u64,
}
pub trait Launcher {
    fn spawn(&mut self, path: &str) -> Result<u64, i32>;
    fn processes(&mut self) -> Result<Vec<ProcessSnapshot>, i32>;
}
pub trait Transport {
    fn exchange(&mut self, service: &str, request: &[u8]) -> Result<Vec<u8>, TransportError>;
}
pub struct UnavailableTransport;
impl Transport for UnavailableTransport {
    fn exchange(&mut self, _service: &str, _request: &[u8]) -> Result<Vec<u8>, TransportError> {
        Err(TransportError::Unavailable)
    }
}
fn validate_service(name: &str, path: &str) -> Result<(), ConfigError> {
    if name.is_empty() || name.len() > MAX_SERVICE_NAME_BYTES {
        return Err(ConfigError::Malformed);
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(ConfigError::Malformed);
    }
    if !path.starts_with('/') || path.len() > MAX_PATH_BYTES {
        return Err(ConfigError::Malformed);
    }
    if path.split('/').any(|component| component == "..") || path.contains(' ') {
        return Err(ConfigError::Malformed);
    }
    Ok(())
}
pub fn default_config() -> Config {
    Config {
        services: alloc::vec![
            ServiceSpec {
                name: String::from("authd"),
                path: String::from("/bin/system/authd.elf"),
            },
            ServiceSpec {
                name: String::from("netmgr"),
                path: String::from("/bin/system/netmgr.elf"),
            },
            ServiceSpec {
                name: String::from("bluetoothd"),
                path: String::from("/bin/system/bluetoothd.elf"),
            },
        ],
        restart: false,
        max_restarts: DEFAULT_MAX_RESTARTS,
    }
}
pub fn parse_config(text: &str) -> Result<Config, ConfigError> {
    if text.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge);
    }
    let mut config = Config {
        services: Vec::new(),
        restart: false,
        max_restarts: DEFAULT_MAX_RESTARTS,
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(ConfigError::Malformed)?;
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            return Err(ConfigError::EmptyValue);
        }
        match key {
            "restart" => match value {
                "on" => config.restart = true,
                "off" => config.restart = false,
                _ => return Err(ConfigError::Malformed),
            },
            "max_restarts" => {
                let parsed = value.parse::<u32>().map_err(|_| ConfigError::Malformed)?;
                if parsed > 100 {
                    return Err(ConfigError::Malformed);
                }
                config.max_restarts = parsed;
            }
            "service" => {
                if config.services.len() >= MAX_SERVICES {
                    return Err(ConfigError::TooManyServices);
                }
                let (name, path) = value.split_once(':').ok_or(ConfigError::Malformed)?;
                let name = name.trim();
                let path = path.trim();
                validate_service(name, path)?;
                if config.services.iter().any(|spec| spec.name == name) {
                    return Err(ConfigError::DuplicateService);
                }
                config.services.push(ServiceSpec {
                    name: String::from(name),
                    path: String::from(path),
                });
            }
            _ => return Err(ConfigError::UnknownKey),
        }
    }
    if config.services.is_empty() {
        return Err(ConfigError::NoServices);
    }
    Ok(config)
}
struct ServiceState {
    pid: Option<u64>,
    restarts: u32,
    missing_reported: bool,
}
pub struct Supervisor<L, T> {
    config: Config,
    launcher: L,
    transport: T,
    state: Vec<ServiceState>,
}
impl<L: Launcher, T: Transport> Supervisor<L, T> {
    pub fn new(config: Config, launcher: L, transport: T) -> Self {
        let state = config
            .services
            .iter()
            .map(|_| ServiceState {
                pid: None,
                restarts: 0,
                missing_reported: false,
            })
            .collect();
        Self {
            config,
            launcher,
            transport,
            state,
        }
    }
    pub fn services(&self) -> &[ServiceSpec] {
        &self.config.services
    }
    pub fn handshake(&mut self, index: usize) -> Result<HelloResponse, ProtocolError> {
        let service = self
            .config
            .services
            .get(index)
            .ok_or(ProtocolError::Truncated)?;
        let request = HelloRequest {
            client_version: protocol::PROTOCOL_VERSION,
        }
        .encode()?;
        let raw = self
            .transport
            .exchange(&service.name, &request)
            .map_err(|error| match error {
                TransportError::Unavailable => ProtocolError::Truncated,
                TransportError::Closed => ProtocolError::Truncated,
                TransportError::Rejected => ProtocolError::ChecksumMismatch,
            })?;
        let (kind, payload) = protocol::decode_message(&raw)?;
        if kind != message_kind::HELLO_RESPONSE {
            return Err(ProtocolError::UnsupportedKind { kind });
        }
        let response = HelloResponse::decode(&payload)?;
        if response.server_version != protocol::PROTOCOL_VERSION {
            return Err(ProtocolError::VersionMismatch {
                found: response.server_version,
            });
        }
        Ok(response)
    }
    pub fn verify_handshakes(&mut self) -> Vec<(usize, SessionEvent)> {
        let mut events = Vec::new();
        for index in 0..self.config.services.len() {
            match self.handshake(index) {
                Ok(_) => events.push((index, SessionEvent::HandlesUnavailable)),
                Err(_) => events.push((index, SessionEvent::HandshakeRejected)),
            }
        }
        events
    }
    pub fn reconcile(&mut self) -> Vec<(usize, SessionEvent)> {
        let alive = match self.launcher.processes() {
            Ok(alive) => alive,
            Err(_) => return Vec::new(),
        };
        let mut events = Vec::new();
        for (index, spec) in self.config.services.iter().enumerate() {
            let found = alive
                .iter()
                .find(|process| process.name == spec.name)
                .map(|process| process.pid);
            let state = &mut self.state[index];
            match (found, state.pid) {
                (Some(pid), previous) => {
                    if previous.is_some() && previous != Some(pid) {
                        events.push((index, SessionEvent::ServiceDied));
                        state.restarts = 0;
                    }
                    state.pid = Some(pid);
                    state.missing_reported = false;
                }
                (None, Some(pid)) => {
                    events.push((index, SessionEvent::ServiceDied));
                    state.pid = None;
                    let attempt = state.restarts + 1;
                    if self.config.restart && attempt <= self.config.max_restarts {
                        match self.launcher.spawn(&spec.path) {
                            Ok(new_pid) => {
                                state.pid = Some(new_pid);
                                state.restarts = attempt;
                                events.push((index, SessionEvent::RestartIssued));
                            }
                            Err(_) => {
                                state.restarts = self.config.max_restarts;
                                events.push((index, SessionEvent::RestartLimitReached));
                            }
                        }
                    } else {
                        state.restarts = self.config.max_restarts;
                        events.push((index, SessionEvent::RestartLimitReached));
                    }
                }
                (None, None) => {
                    if !state.missing_reported {
                        state.missing_reported = true;
                        events.push((index, SessionEvent::ServiceMissing));
                    }
                }
            }
        }
        events
    }
    pub fn launcher(&mut self) -> &mut L {
        &mut self.launcher
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;
    struct MockLauncher {
        alive: Vec<ProcessSnapshot>,
        next_pid: u64,
        fail_spawn: bool,
    }
    impl MockLauncher {
        fn new() -> Self {
            Self {
                alive: Vec::new(),
                next_pid: 10,
                fail_spawn: false,
            }
        }
    }
    impl Launcher for MockLauncher {
        fn spawn(&mut self, path: &str) -> Result<u64, i32> {
            if self.fail_spawn {
                return Err(-2);
            }
            self.next_pid += 1;
            let pid = self.next_pid;
            let name = path
                .rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".elf")
                .to_string();
            self.alive.push(ProcessSnapshot {
                name: String::from(name),
                pid,
            });
            Ok(pid)
        }
        fn processes(&mut self) -> Vec<ProcessSnapshot> {
            self.alive
                .iter()
                .map(|process| ProcessSnapshot {
                    name: process.name.clone(),
                    pid: process.pid,
                })
                .collect()
        }
    }
    struct MapTransport {
        replies: BTreeMap<String, Vec<u8>>,
    }
    impl Transport for MapTransport {
        fn exchange(
            &mut self,
            service: &str,
            _request: &[u8],
        ) -> Result<Vec<u8>, TransportError> {
            match self.replies.get(service) {
                Some(bytes) => Ok(bytes.clone()),
                None => Err(TransportError::Unavailable),
            }
        }
    }
    fn config_with_restart(restart: bool) -> Config {
        Config {
            services: alloc::vec![ServiceSpec {
                name: String::from("authd"),
                path: String::from("/bin/system/authd.elf"),
            }],
            restart,
            max_restarts: 2,
        }
    }
    #[test]
    fn parses_complete_configuration() {
        let text = "restart=on\nmax_restarts=5\nservice=authd:/bin/system/authd.elf\nservice=netmgr:/bin/system/netmgr.elf\n";
        let config = parse_config(text).unwrap();
        assert!(config.restart);
        assert_eq!(config.max_restarts, 5);
        assert_eq!(config.services.len(), 2);
        assert_eq!(config.services[0].name, "authd");
        assert_eq!(config.services[1].path, "/bin/system/netmgr.elf");
    }
    #[test]
    fn rejects_malformed_configuration() {
        assert_eq!(
            parse_config("restart=maybe\nservice=authd:/bin/authd.elf\n"),
            Err(ConfigError::Malformed)
        );
        assert_eq!(
            parse_config("secreto=1\nservice=authd:/bin/authd.elf\n"),
            Err(ConfigError::UnknownKey)
        );
        assert_eq!(
            parse_config("restart=on\n"),
            Err(ConfigError::NoServices)
        );
        assert_eq!(
            parse_config("service=authd:/bin/../../etc/passwd\n"),
            Err(ConfigError::Malformed)
        );
        assert_eq!(
            parse_config("service=authd:/bin/authd.elf\nservice=authd:/bin/other.elf\n"),
            Err(ConfigError::DuplicateService)
        );
        assert_eq!(parse_config("service=authd\n"), Err(ConfigError::Malformed));
        let huge = "x".repeat(MAX_CONFIG_BYTES + 1);
        assert_eq!(parse_config(&huge), Err(ConfigError::TooLarge));
    }
    #[test]
    fn default_configuration_lists_the_three_services() {
        let config = default_config();
        assert_eq!(config.services.len(), 3);
        assert!(!config.restart);
        let names: Vec<&str> = config.services.iter().map(|spec| spec.name.as_str()).collect();
        assert_eq!(names, alloc::vec!["authd", "netmgr", "bluetoothd"]);
    }
    #[test]
    fn reconcile_reports_missing_service_once() {
        let mut supervisor = Supervisor::new(default_config(), MockLauncher::new(), UnavailableTransport);
        let first = supervisor.reconcile();
        assert_eq!(first.len(), 3);
        assert!(first
            .iter()
            .all(|(_, event)| *event == SessionEvent::ServiceMissing));
        let second = supervisor.reconcile();
        assert!(second.is_empty());
    }
    #[test]
    fn reconcile_detects_death_and_honours_restart_limit() {
        let mut launcher = MockLauncher::new();
        launcher.alive.push(ProcessSnapshot {
            name: String::from("authd"),
            pid: 3,
        });
        let mut supervisor = Supervisor::new(config_with_restart(true), launcher, UnavailableTransport);
        let initial = supervisor.reconcile();
        assert!(initial.is_empty());
        supervisor.launcher().alive.clear();
        let events = supervisor.reconcile();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1, SessionEvent::RestartIssued);
        let alive = supervisor.launcher().processes();
        assert_eq!(alive.len(), 1);
        assert_ne!(alive[0].pid, 3);
        let mut saw_limit = false;
        for _ in 0..4 {
            let current = supervisor.launcher().processes();
            if let Some(process) = current.first() {
                let pid = process.pid;
                supervisor.launcher().alive.retain(|entry| entry.pid != pid);
            }
            for (_, event) in supervisor.reconcile() {
                if event == SessionEvent::RestartLimitReached {
                    saw_limit = true;
                }
            }
        }
        assert!(saw_limit);
        assert!(supervisor.launcher().processes().is_empty());
    }
    #[test]
    fn reconcile_without_restart_only_reports() {
        let mut launcher = MockLauncher::new();
        launcher.alive.push(ProcessSnapshot {
            name: String::from("authd"),
            pid: 3,
        });
        let mut supervisor = Supervisor::new(config_with_restart(false), launcher, UnavailableTransport);
        let events = supervisor.reconcile();
        assert!(events.iter().all(|(_, event)| *event == SessionEvent::ServiceDied));
        assert!(supervisor.launcher().processes().is_empty());
    }
    fn hello_response_bytes(version: u16) -> Vec<u8> {
        HelloResponse {
            server_version: version,
            service: protocol::service_id::AUTHD,
            max_message_bytes: protocol::MAX_MESSAGE_BYTES as u32,
        }
        .encode()
        .unwrap()
    }
    #[test]
    fn handshake_accepts_a_compatible_service() {
        let mut replies = BTreeMap::new();
        replies.insert(String::from("authd"), hello_response_bytes(protocol::PROTOCOL_VERSION));
        let mut supervisor = Supervisor::new(
            config_with_restart(false),
            MockLauncher::new(),
            MapTransport { replies },
        );
        let response = supervisor.handshake(0).unwrap();
        assert_eq!(response.server_version, protocol::PROTOCOL_VERSION);
        assert_eq!(response.service, protocol::service_id::AUTHD);
    }
    #[test]
    fn handshake_rejects_incompatible_or_unavailable_peers() {
        let mut replies = BTreeMap::new();
        replies.insert(
            String::from("authd"),
            hello_response_bytes(protocol::PROTOCOL_VERSION + 7),
        );
        let mut supervisor = Supervisor::new(
            config_with_restart(false),
            MockLauncher::new(),
            MapTransport { replies },
        );
        assert!(matches!(
            supervisor.handshake(0),
            Err(ProtocolError::VersionMismatch { .. })
        ));
        let mut supervisor = Supervisor::new(
            config_with_restart(false),
            MockLauncher::new(),
            MapTransport {
                replies: BTreeMap::new(),
            },
        );
        assert!(supervisor.handshake(0).is_err());
        let events = supervisor.verify_handshakes();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1, SessionEvent::HandshakeRejected);
    }
    #[test]
    fn handle_grant_is_reported_unavailable_instead_of_pretending() {
        let mut replies = BTreeMap::new();
        replies.insert(
            String::from("authd"),
            hello_response_bytes(protocol::PROTOCOL_VERSION),
        );
        replies.insert(
            String::from("netmgr"),
            hello_response_bytes(protocol::PROTOCOL_VERSION),
        );
        replies.insert(
            String::from("bluetoothd"),
            hello_response_bytes(protocol::PROTOCOL_VERSION),
        );
        let mut supervisor = Supervisor::new(
            default_config(),
            MockLauncher::new(),
            MapTransport { replies },
        );
        let events = supervisor.verify_handshakes();
        assert_eq!(events.len(), 3);
        assert!(events
            .iter()
            .all(|(_, event)| *event == SessionEvent::HandlesUnavailable));
    }
}
