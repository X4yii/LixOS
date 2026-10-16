#![cfg_attr(not(test), no_std)]
#![allow(unsafe_code)]
pub mod audit;
pub mod entropy;
pub mod protocol;
pub mod store;
pub mod vault;
use alloc::string::String;
use alloc::vec::Vec;
extern crate alloc;
pub type UserId = u64;
pub type ProgramId = u64;
pub type RevokerId = u64;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    SystemConfiguration,
    NetworkAdministration,
    BluetoothAdministration,
    PowerManagement,
    AdministrativeSession,
}
#[derive(Clone, Debug)]
pub struct ElevationRequest {
    pub user: UserId,
    pub program: ProgramId,
    pub scope: Scope,
    pub lifetime_seconds: u32,
    pub credential: Vec<u8>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidCredential,
    NotAdministrator,
    InvalidLifetime,
    RateLimited,
    PolicyDenied,
    GrantFailed,
    EntropyUnavailable,
    InvalidInput,
    AlreadyEnrolled,
    ConsoleConfirmationRequired,
    AccessDenied,
    IpcUnavailable,
    MalformedMessage,
}
pub trait CredentialVerifier {
    fn verify(&mut self, user: UserId, credential: &[u8]) -> Result<bool, Error>;
    fn is_administrator(&self, user: UserId) -> bool;
}
pub trait GrantBroker {
    fn spawn_scoped(
        &mut self,
        user: UserId,
        program: ProgramId,
        scope: Scope,
        lifetime_seconds: u32,
    ) -> Result<RevokerId, Error>;
}
pub enum AuditEvent {
    Login {
        user: UserId,
        granted: bool,
    },
    Enrollment {
        user: UserId,
        administrator: bool,
    },
    Elevation {
        user: UserId,
        program: ProgramId,
        scope: Scope,
        granted: bool,
    },
}
pub trait AuditLog {
    fn write(&mut self, event: AuditEvent);
}
pub trait Clock {
    fn now_seconds(&self) -> u64;
}
pub const MAX_FAILED_ATTEMPTS: u32 = 5;
pub const MAX_LOCKOUT_SECONDS: u64 = 300;
pub struct AuthService<V, B, A, C> {
    verifier: V,
    broker: B,
    audit: A,
    clock: C,
    max_lifetime_seconds: u32,
    failures: Vec<(UserId, u32, u64)>,
}
impl<V: CredentialVerifier, B: GrantBroker, A: AuditLog, C: Clock> AuthService<V, B, A, C> {
    pub const fn new(verifier: V, broker: B, audit: A, clock: C) -> Self {
        Self {
            verifier,
            broker,
            audit,
            clock,
            max_lifetime_seconds: 300,
            failures: Vec::new(),
        }
    }
    fn lockout_until(&mut self, user: UserId) -> Option<u64> {
        let now = self.clock.now_seconds();
        let position = self.failures.iter().position(|(id, _, _)| *id == user);
        let index = position?;
        let (id, count, until) = self.failures[index];
        if count < MAX_FAILED_ATTEMPTS {
            return None;
        }
        if now >= until {
            self.failures[index] = (id, 0, 0);
            return None;
        }
        Some(until)
    }
    fn register_failure(&mut self, user: UserId) {
        let now = self.clock.now_seconds();
        if let Some(entry) = self.failures.iter_mut().find(|(id, _, _)| *id == user) {
            entry.1 = entry.1.saturating_add(1);
            let backoff = 1u64 << entry.1.min(9);
            entry.2 = now.saturating_add(backoff.min(MAX_LOCKOUT_SECONDS));
            return;
        }
        let backoff = 1u64 << 1;
        self.failures.push((user, 1, now.saturating_add(backoff)));
    }
    fn clear_failures(&mut self, user: UserId) {
        self.failures.retain(|(id, _, _)| *id != user);
    }
    pub fn request(&mut self, mut request: ElevationRequest) -> Result<RevokerId, Error> {
        if request.lifetime_seconds == 0 || request.lifetime_seconds > self.max_lifetime_seconds {
            erase_secret(&mut request.credential);
            return Err(Error::InvalidLifetime);
        }
        if self.lockout_until(request.user).is_some() {
            erase_secret(&mut request.credential);
            self.audit.write(AuditEvent::Elevation {
                user: request.user,
                program: request.program,
                scope: request.scope,
                granted: false,
            });
            return Err(Error::RateLimited);
        }
        let verification = self.verifier.verify(request.user, &request.credential);
        erase_secret(&mut request.credential);
        let authenticated = verification?;
        if !authenticated {
            self.register_failure(request.user);
            self.audit.write(AuditEvent::Elevation {
                user: request.user,
                program: request.program,
                scope: request.scope,
                granted: false,
            });
            return Err(Error::InvalidCredential);
        }
        self.clear_failures(request.user);
        if !self.verifier.is_administrator(request.user) {
            self.audit.write(AuditEvent::Elevation {
                user: request.user,
                program: request.program,
                scope: request.scope,
                granted: false,
            });
            return Err(Error::NotAdministrator);
        }
        match self.broker.spawn_scoped(
            request.user,
            request.program,
            request.scope,
            request.lifetime_seconds,
        ) {
            Ok(revoker) => {
                self.audit.write(AuditEvent::Elevation {
                    user: request.user,
                    program: request.program,
                    scope: request.scope,
                    granted: true,
                });
                Ok(revoker)
            }
            Err(error) => {
                self.audit.write(AuditEvent::Elevation {
                    user: request.user,
                    program: request.program,
                    scope: request.scope,
                    granted: false,
                });
                Err(error)
            }
        }
    }
    pub fn login(&mut self, user: UserId, mut credential: Vec<u8>) -> Result<(), Error> {
        if self.lockout_until(user).is_some() {
            erase_secret(&mut credential);
            self.audit.write(AuditEvent::Login {
                user,
                granted: false,
            });
            return Err(Error::RateLimited);
        }
        let verification = self.verifier.verify(user, &credential);
        erase_secret(&mut credential);
        let authenticated = verification?;
        self.audit.write(AuditEvent::Login {
            user,
            granted: authenticated,
        });
        if !authenticated {
            self.register_failure(user);
            return Err(Error::InvalidCredential);
        }
        self.clear_failures(user);
        Ok(())
    }
    pub fn failures(&self, user: UserId) -> u32 {
        self.failures
            .iter()
            .find(|(id, _, _)| *id == user)
            .map_or(0, |(_, count, _)| *count)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPolicy {
    pub name: String,
    pub administrator: bool,
    pub disabled: bool,
}
fn erase_secret(secret: &mut [u8]) {
    for byte in secret {
        unsafe {
            core::ptr::write_volatile(byte, 0);
        }
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}
pub struct UserDatabase {
    pub accounts: Vec<(UserId, AccountPolicy, vault::PasswordRecord)>,
    pub dummy: vault::PasswordRecord,
}
impl CredentialVerifier for UserDatabase {
    fn verify(&mut self, user: UserId, credential: &[u8]) -> Result<bool, Error> {
        let account = self.accounts.iter().find(|(id, _, _)| *id == user);
        let record = account.map_or(&self.dummy, |(_, _, record)| record);
        let matches = record
            .verify(credential)
            .map_err(|_| Error::InvalidCredential)?;
        Ok(account.is_some_and(|(_, policy, _)| !policy.disabled) && matches)
    }
    fn is_administrator(&self, user: UserId) -> bool {
        self.accounts
            .iter()
            .find(|(id, _, _)| *id == user)
            .is_some_and(|(_, policy, _)| !policy.disabled && policy.administrator)
    }
}
pub struct ConsolePermit {
    _private: (),
}
pub trait ConsoleInteraction {
    fn confirm(&mut self, subject: &str) -> bool;
}
pub fn acquire_console_permit<I: ConsoleInteraction>(
    interaction: &mut I,
    subject: &str,
) -> Option<ConsolePermit> {
    if subject.is_empty() {
        return None;
    }
    if !interaction.confirm(subject) {
        return None;
    }
    Some(ConsolePermit { _private: () })
}
pub const fn dummy_record() -> vault::PasswordRecord {
    vault::PasswordRecord {
        salt: *b"lixos-dummy-salt",
        verifier: [0; 32],
    }
}
fn valid_account_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 32 {
        return false;
    }
    bytes.iter().enumerate().all(|(index, byte)| {
        byte.is_ascii_alphanumeric() || *byte == b'_' || (index > 0 && *byte == b'-')
    })
}
impl UserDatabase {
    pub fn enroll_first_admin<E: entropy::Entropy>(
        &mut self,
        _permit: ConsolePermit,
        user: UserId,
        name: String,
        mut credential: Vec<u8>,
        source: &mut E,
    ) -> Result<(), Error> {
        if !self.accounts.is_empty() {
            return Err(Error::AlreadyEnrolled);
        }
        if !valid_account_name(&name) {
            return Err(Error::InvalidInput);
        }
        let mut salt = [0u8; 16];
        let generated = match source.fill(&mut salt) {
            Ok(()) => salt,
            Err(error) => {
                erase_secret(&mut credential);
                return Err(error);
            }
        };
        let record = match vault::PasswordRecord::create(&credential, generated) {
            Ok(record) => record,
            Err(vault::CryptoError::InvalidPassword) => {
                erase_secret(&mut credential);
                return Err(Error::InvalidInput);
            }
            Err(_) => {
                erase_secret(&mut credential);
                return Err(Error::PolicyDenied);
            }
        };
        erase_secret(&mut credential);
        self.accounts.push((
            user,
            AccountPolicy {
                name,
                administrator: true,
                disabled: false,
            },
            record,
        ));
        Ok(())
    }
}
pub const fn required_right(scope: Scope) -> lix_sys::Rights {
    match scope {
        Scope::SystemConfiguration => lix_sys::Rights::AUTH,
        Scope::NetworkAdministration => lix_sys::Rights::NET_ADMIN,
        Scope::BluetoothAdministration => lix_sys::Rights::BLUETOOTH_ADMIN,
        Scope::PowerManagement => lix_sys::Rights::POWER,
        Scope::AdministrativeSession => lix_sys::Rights::ELEVATE,
    }
}
pub fn authorize_scope(granted: lix_sys::Rights, scope: Scope) -> Result<(), Error> {
    if granted.contains(required_right(scope)) {
        Ok(())
    } else {
        Err(Error::AccessDenied)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::MemoryAudit;
    use core::cell::Cell;
    struct FixedClock {
        now: Cell<u64>,
    }
    impl Clock for FixedClock {
        fn now_seconds(&self) -> u64 {
            self.now.get()
        }
    }
    struct StaticVerifier {
        password: Vec<u8>,
        administrator: bool,
    }
    impl CredentialVerifier for StaticVerifier {
        fn verify(&mut self, _user: UserId, credential: &[u8]) -> Result<bool, Error> {
            Ok(credential == self.password.as_slice())
        }
        fn is_administrator(&self, user: UserId) -> bool {
            self.administrator && user == 1
        }
    }
    struct FixedBroker;
    impl GrantBroker for FixedBroker {
        fn spawn_scoped(
            &mut self,
            _user: UserId,
            _program: ProgramId,
            _scope: Scope,
            _lifetime_seconds: u32,
        ) -> Result<RevokerId, Error> {
            Ok(1)
        }
    }
    struct FixedEntropy {
        seed: u8,
    }
    impl entropy::Entropy for FixedEntropy {
        fn fill(&mut self, buf: &mut [u8]) -> Result<(), Error> {
            for byte in buf.iter_mut() {
                *byte = self.seed;
                self.seed = self.seed.wrapping_add(1);
            }
            Ok(())
        }
    }
    struct ApproveConsole;
    impl ConsoleInteraction for ApproveConsole {
        fn confirm(&mut self, _subject: &str) -> bool {
            true
        }
    }
    struct DenyConsole;
    impl ConsoleInteraction for DenyConsole {
        fn confirm(&mut self, _subject: &str) -> bool {
            false
        }
    }
    fn service(
        password: &[u8],
        administrator: bool,
    ) -> AuthService<StaticVerifier, FixedBroker, MemoryAudit, FixedClock> {
        AuthService::new(
            StaticVerifier {
                password: password.to_vec(),
                administrator,
            },
            FixedBroker,
            MemoryAudit { lines: Vec::new() },
            FixedClock {
                now: Cell::new(1_000),
            },
        )
    }
    fn elevation(user: UserId, password: &[u8], lifetime: u32) -> ElevationRequest {
        ElevationRequest {
            user,
            program: 77,
            scope: Scope::NetworkAdministration,
            lifetime_seconds: lifetime,
            credential: password.to_vec(),
        }
    }
    #[test]
    fn erase_secret_wipes_buffer() {
        let mut buffer = b"contrasena-en-buffers".to_vec();
        erase_secret(&mut buffer);
        assert!(buffer.iter().all(|byte| *byte == 0));
    }
    #[test]
    fn rate_limit_triggers_after_max_failed_attempts() {
        let mut service = service(b"clave-correcta-1", true);
        for _ in 0..MAX_FAILED_ATTEMPTS {
            assert_eq!(
                service.request(elevation(1, b"incorrecta-1", 60)),
                Err(Error::InvalidCredential)
            );
        }
        assert_eq!(service.failures(1), MAX_FAILED_ATTEMPTS);
        assert_eq!(
            service.request(elevation(1, b"clave-correcta-1", 60)),
            Err(Error::RateLimited)
        );
    }
    #[test]
    fn successful_authentication_clears_failures() {
        let mut service = service(b"clave-correcta-1", true);
        assert_eq!(
            service.request(elevation(1, b"incorrecta-1", 60)),
            Err(Error::InvalidCredential)
        );
        assert!(service
            .request(elevation(1, b"clave-correcta-1", 60))
            .is_ok());
        assert_eq!(service.failures(1), 0);
        assert!(service
            .request(elevation(1, b"clave-correcta-1", 60))
            .is_ok());
    }
    #[test]
    fn lockout_expires_after_backoff_window() {
        let mut service = service(b"clave-correcta-1", true);
        for _ in 0..MAX_FAILED_ATTEMPTS {
            assert_eq!(
                service.request(elevation(1, b"incorrecta-1", 60)),
                Err(Error::InvalidCredential)
            );
        }
        assert_eq!(
            service.request(elevation(1, b"clave-correcta-1", 60)),
            Err(Error::RateLimited)
        );
        service.clock.now = Cell::new(1_000 + MAX_LOCKOUT_SECONDS + 1);
        assert!(service
            .request(elevation(1, b"clave-correcta-1", 60))
            .is_ok());
        assert_eq!(service.failures(1), 0);
    }
    #[test]
    fn elevation_rejects_invalid_lifetime_and_non_administrators() {
        let mut service = service(b"clave-correcta-1", false);
        assert_eq!(
            service.request(elevation(1, b"clave-correcta-1", 0)),
            Err(Error::InvalidLifetime)
        );
        assert_eq!(
            service.request(elevation(1, b"clave-correcta-1", 10_000)),
            Err(Error::InvalidLifetime)
        );
        assert_eq!(
            service.request(elevation(1, b"clave-correcta-1", 60)),
            Err(Error::NotAdministrator)
        );
    }
    #[test]
    fn login_follows_the_same_rate_limit() {
        let mut service = service(b"clave-correcta-1", false);
        for _ in 0..MAX_FAILED_ATTEMPTS {
            assert_eq!(
                service.login(1, b"incorrecta-1".to_vec()),
                Err(Error::InvalidCredential)
            );
        }
        assert_eq!(
            service.login(1, b"clave-correcta-1".to_vec()),
            Err(Error::RateLimited)
        );
    }
    #[test]
    fn audit_records_never_contain_credentials() {
        let mut service = service(b"super-secreta-1", true);
        assert_eq!(service.login(1, b"super-secreta-1".to_vec()), Ok(()));
        assert_eq!(
            service.login(1, b"mal-intentada-1".to_vec()),
            Err(Error::InvalidCredential)
        );
        assert_eq!(service.request(elevation(1, b"super-secreta-1", 60)), Ok(1));
        assert!(!service.audit.lines.is_empty());
        for line in &service.audit.lines {
            assert!(!line.contains("super-secreta-1"));
            assert!(!line.contains("mal-intentada-1"));
        }
    }
    fn empty_database() -> UserDatabase {
        UserDatabase {
            accounts: Vec::new(),
            dummy: dummy_record(),
        }
    }
    #[test]
    fn first_admin_requires_explicit_console_confirmation() {
        let mut denied = DenyConsole;
        assert!(acquire_console_permit(&mut denied, "ana").is_none());
        let mut approved = ApproveConsole;
        assert!(acquire_console_permit(&mut approved, "").is_none());
        assert!(acquire_console_permit(&mut approved, "ana").is_some());
    }
    #[test]
    fn first_admin_enrollment_succeeds_with_working_entropy() {
        let mut database = empty_database();
        let mut console = ApproveConsole;
        let permit = acquire_console_permit(&mut console, "ana").unwrap();
        let mut source = FixedEntropy { seed: 9 };
        let result = database.enroll_first_admin(
            permit,
            1,
            String::from("ana"),
            b"contrasena-larga".to_vec(),
            &mut source,
        );
        assert_eq!(result, Ok(()));
        assert_eq!(database.accounts.len(), 1);
        assert!(database.verify(1, b"contrasena-larga").unwrap());
        assert!(database.is_administrator(1));
    }
    #[test]
    fn enrollment_fails_closed_without_entropy() {
        let mut database = empty_database();
        let mut console = ApproveConsole;
        let permit = acquire_console_permit(&mut console, "ana").unwrap();
        let mut source = entropy::NoEntropy;
        let result = database.enroll_first_admin(
            permit,
            1,
            String::from("ana"),
            b"contrasena-larga".to_vec(),
            &mut source,
        );
        assert_eq!(result, Err(Error::EntropyUnavailable));
        assert!(database.accounts.is_empty());
    }
    #[test]
    fn enrollment_is_rejected_once_an_account_exists() {
        let mut database = empty_database();
        let mut console = ApproveConsole;
        let permit = acquire_console_permit(&mut console, "ana").unwrap();
        let mut source = FixedEntropy { seed: 3 };
        database
            .enroll_first_admin(
                permit,
                1,
                String::from("ana"),
                b"contrasena-larga".to_vec(),
                &mut source,
            )
            .unwrap();
        let second = acquire_console_permit(&mut console, "bob").unwrap();
        assert_eq!(
            database.enroll_first_admin(
                second,
                2,
                String::from("bob"),
                b"otra-contrasena".to_vec(),
                &mut source
            ),
            Err(Error::AlreadyEnrolled)
        );
        assert_eq!(database.accounts.len(), 1);
    }
    #[test]
    fn enrollment_validates_name_and_credential() {
        let mut database = empty_database();
        let mut console = ApproveConsole;
        let permit = acquire_console_permit(&mut console, "ana").unwrap();
        let mut source = FixedEntropy { seed: 4 };
        assert_eq!(
            database.enroll_first_admin(
                permit,
                1,
                String::from(""),
                b"contrasena-larga".to_vec(),
                &mut source
            ),
            Err(Error::InvalidInput)
        );
        let permit = acquire_console_permit(&mut console, "ana con espacios").unwrap();
        assert_eq!(
            database.enroll_first_admin(
                permit,
                1,
                String::from("ana con espacios"),
                b"contrasena-larga".to_vec(),
                &mut source
            ),
            Err(Error::InvalidInput)
        );
        let permit = acquire_console_permit(&mut console, "ana").unwrap();
        assert_eq!(
            database.enroll_first_admin(
                permit,
                1,
                String::from("ana"),
                b"corta".to_vec(),
                &mut source
            ),
            Err(Error::InvalidInput)
        );
        assert!(database.accounts.is_empty());
    }
    fn ordinary_process_rights() -> lix_sys::Rights {
        lix_sys::Rights::READ
            .union(lix_sys::Rights::WRITE)
            .union(lix_sys::Rights::EXEC)
            .union(lix_sys::Rights::SPAWN)
            .union(lix_sys::Rights::DEVICES)
            .union(lix_sys::Rights::LOG)
            .union(lix_sys::Rights::NET_CONNECT)
    }
    #[test]
    fn process_without_matching_capability_cannot_authorize_scope() {
        let ordinary = ordinary_process_rights();
        assert_eq!(
            authorize_scope(ordinary, Scope::NetworkAdministration),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            authorize_scope(ordinary, Scope::BluetoothAdministration),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            authorize_scope(ordinary, Scope::PowerManagement),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            authorize_scope(ordinary, Scope::SystemConfiguration),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            authorize_scope(ordinary, Scope::AdministrativeSession),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            authorize_scope(lix_sys::Rights::NONE, Scope::NetworkAdministration),
            Err(Error::AccessDenied)
        );
        assert!(authorize_scope(
            ordinary.union(lix_sys::Rights::NET_ADMIN),
            Scope::NetworkAdministration
        )
        .is_ok());
        assert!(authorize_scope(
            ordinary.union(lix_sys::Rights::POWER),
            Scope::PowerManagement
        )
        .is_ok());
    }
}
