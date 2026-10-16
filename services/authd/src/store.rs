use crate::vault;
use crate::{AccountPolicy, UserId};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
pub const STORE_MAGIC: [u8; 8] = *b"LIXAUTH\0";
pub const STORE_FORMAT_VERSION: u16 = 2;
pub const OLDEST_STORE_FORMAT_VERSION: u16 = 1;
pub const STORE_HEADER_BYTES: usize = 16;
pub const RECORD_HEADER_BYTES: usize = 12;
pub const MAX_RECORD_BYTES: usize = 64 * 1024;
pub const MAX_RECORDS: usize = 256;
pub const RECORD_POLICY: u16 = 1;
pub const RECORD_ACCOUNTS: u16 = 2;
pub const RECORD_VAULT: u16 = 3;
pub const ACCOUNTS_VERSION: u16 = 2;
pub const ACCOUNTS_LEGACY_VERSION: u16 = 1;
pub const POLICY_VERSION: u16 = 2;
pub const POLICY_LEGACY_VERSION: u16 = 1;
pub const VAULT_VERSION: u16 = 1;
pub const MAX_ACCOUNTS: usize = 1024;
pub const MAX_ACCOUNT_NAME_BYTES: usize = 32;
pub const MAX_VAULT_ENTRIES: usize = 1024;
pub const MAX_VAULT_LABEL_BYTES: usize = 64;
pub const MAX_SECRET_BYTES: usize = 16 * 1024;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError {
    NotFound,
    HeaderCorrupt,
    UnsupportedVersion { found: u16 },
    Truncated,
    RecordTooLarge,
    TooManyRecords,
    ChecksumMismatch { index: usize },
    MissingRecord(u16),
    DuplicateRecord(u16),
    MigrationRequired { kind: u16, found: u16 },
    InvalidPayload { kind: u16 },
    AtomicWriteUnavailable,
    Io(&'static str),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredRecord {
    pub kind: u16,
    pub version: u16,
    pub payload: Vec<u8>,
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
pub fn encode_store(records: &[StoredRecord]) -> Result<Vec<u8>, StoreError> {
    if records.is_empty() {
        return Err(StoreError::MissingRecord(RECORD_POLICY));
    }
    if records.len() > MAX_RECORDS {
        return Err(StoreError::TooManyRecords);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&STORE_MAGIC);
    out.extend_from_slice(&STORE_FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    let header_crc = crc32(&out);
    out.extend_from_slice(&header_crc.to_le_bytes());
    for record in records {
        if record.payload.len() > MAX_RECORD_BYTES {
            return Err(StoreError::RecordTooLarge);
        }
        out.extend_from_slice(&record.kind.to_le_bytes());
        out.extend_from_slice(&record.version.to_le_bytes());
        out.extend_from_slice(&(record.payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&crc32(&record.payload).to_le_bytes());
        out.extend_from_slice(&record.payload);
    }
    Ok(out)
}
pub fn decode_store(bytes: &[u8]) -> Result<Vec<StoredRecord>, StoreError> {
    if bytes.len() < STORE_HEADER_BYTES {
        return Err(StoreError::Truncated);
    }
    if bytes[..8] != STORE_MAGIC {
        return Err(StoreError::HeaderCorrupt);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if !(OLDEST_STORE_FORMAT_VERSION..=STORE_FORMAT_VERSION).contains(&version) {
        return Err(StoreError::UnsupportedVersion { found: version });
    }
    let stored_crc = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    if crc32(&bytes[..12]) != stored_crc {
        return Err(StoreError::HeaderCorrupt);
    }
    let mut records = Vec::new();
    let mut offset = STORE_HEADER_BYTES;
    while offset < bytes.len() {
        if records.len() >= MAX_RECORDS {
            return Err(StoreError::TooManyRecords);
        }
        if bytes.len() - offset < RECORD_HEADER_BYTES {
            return Err(StoreError::Truncated);
        }
        let kind = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        let record_version = u16::from_le_bytes([bytes[offset + 2], bytes[offset + 3]]);
        let length = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let payload_crc = u32::from_le_bytes([
            bytes[offset + 8],
            bytes[offset + 9],
            bytes[offset + 10],
            bytes[offset + 11],
        ]);
        if length > MAX_RECORD_BYTES {
            return Err(StoreError::RecordTooLarge);
        }
        offset += RECORD_HEADER_BYTES;
        if bytes.len() - offset < length {
            return Err(StoreError::Truncated);
        }
        let payload = &bytes[offset..offset + length];
        if crc32(payload) != payload_crc {
            return Err(StoreError::ChecksumMismatch {
                index: records.len(),
            });
        }
        records.push(StoredRecord {
            kind,
            version: record_version,
            payload: payload.to_vec(),
        });
        offset += length;
    }
    Ok(records)
}
fn take<'a>(bytes: &'a [u8], offset: &mut usize, count: usize) -> Result<&'a [u8], StoreError> {
    let end = offset.checked_add(count).ok_or(StoreError::Truncated)?;
    if end > bytes.len() {
        return Err(StoreError::Truncated);
    }
    let slice = &bytes[*offset..end];
    *offset = end;
    Ok(slice)
}
fn take_u16(bytes: &[u8], offset: &mut usize) -> Result<u16, StoreError> {
    let slice = take(bytes, offset, 2)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}
fn take_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, StoreError> {
    let slice = take(bytes, offset, 4)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}
fn take_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, StoreError> {
    let slice = take(bytes, offset, 8)?;
    let mut buf = [0u8; 8];
    buf.copy_from_slice(slice);
    Ok(u64::from_le_bytes(buf))
}
fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Policy {
    pub max_failed_attempts: u32,
    pub max_lockout_seconds: u32,
    pub max_elevation_seconds: u32,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            max_failed_attempts: 5,
            max_lockout_seconds: 300,
            max_elevation_seconds: 300,
        }
    }
}
impl Policy {
    const fn validate(&self) -> bool {
        self.max_failed_attempts >= 1
            && self.max_failed_attempts <= 100
            && self.max_lockout_seconds >= 1
            && self.max_lockout_seconds <= 86_400
            && self.max_elevation_seconds >= 1
            && self.max_elevation_seconds <= 86_400
    }
    pub fn encode(&self) -> Result<Vec<u8>, StoreError> {
        if !self.validate() {
            return Err(StoreError::InvalidPayload {
                kind: RECORD_POLICY,
            });
        }
        let mut out = Vec::with_capacity(12);
        push_u32(&mut out, self.max_failed_attempts);
        push_u32(&mut out, self.max_lockout_seconds);
        push_u32(&mut out, self.max_elevation_seconds);
        Ok(out)
    }
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, StoreError> {
        match version {
            POLICY_VERSION => {
                let mut offset = 0;
                let policy = Self {
                    max_failed_attempts: take_u32(bytes, &mut offset)?,
                    max_lockout_seconds: take_u32(bytes, &mut offset)?,
                    max_elevation_seconds: take_u32(bytes, &mut offset)?,
                };
                if !policy.validate() {
                    return Err(StoreError::InvalidPayload {
                        kind: RECORD_POLICY,
                    });
                }
                Ok(policy)
            }
            POLICY_LEGACY_VERSION => migrate_policy_v1(bytes),
            other => Err(StoreError::MigrationRequired {
                kind: RECORD_POLICY,
                found: other,
            }),
        }
    }
}
fn migrate_policy_v1(bytes: &[u8]) -> Result<Policy, StoreError> {
    let text = core::str::from_utf8(bytes).map_err(|_| StoreError::InvalidPayload {
        kind: RECORD_POLICY,
    })?;
    let mut policy = Policy {
        max_failed_attempts: 0,
        max_lockout_seconds: 0,
        max_elevation_seconds: 0,
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(StoreError::InvalidPayload {
            kind: RECORD_POLICY,
        })?;
        let parsed = value
            .parse::<u32>()
            .map_err(|_| StoreError::InvalidPayload {
                kind: RECORD_POLICY,
            })?;
        match key.trim() {
            "max_failed_attempts" => policy.max_failed_attempts = parsed,
            "max_lockout_seconds" => policy.max_lockout_seconds = parsed,
            "max_elevation_seconds" => policy.max_elevation_seconds = parsed,
            _ => continue,
        }
    }
    if policy.max_failed_attempts == 0 {
        policy.max_failed_attempts = Policy::default().max_failed_attempts;
    }
    if policy.max_lockout_seconds == 0 {
        policy.max_lockout_seconds = Policy::default().max_lockout_seconds;
    }
    if policy.max_elevation_seconds == 0 {
        policy.max_elevation_seconds = Policy::default().max_elevation_seconds;
    }
    if !policy.validate() {
        return Err(StoreError::InvalidPayload {
            kind: RECORD_POLICY,
        });
    }
    Ok(policy)
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedAccount {
    pub id: UserId,
    pub policy: AccountPolicy,
    pub record: vault::PasswordRecord,
}
fn hex_decode(text: &str, out: &mut [u8]) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != out.len() * 2 {
        return false;
    }
    let value = |byte: u8| -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    };
    for index in 0..out.len() {
        let high = match value(bytes[index * 2]) {
            Some(high) => high,
            None => return false,
        };
        let low = match value(bytes[index * 2 + 1]) {
            Some(low) => low,
            None => return false,
        };
        out[index] = (high << 4) | low;
    }
    true
}
pub fn encode_accounts(accounts: &[PersistedAccount]) -> Result<Vec<u8>, StoreError> {
    if accounts.len() > MAX_ACCOUNTS {
        return Err(StoreError::TooManyRecords);
    }
    let mut out = Vec::new();
    push_u16(&mut out, accounts.len() as u16);
    for account in accounts {
        if account.policy.name.len() > MAX_ACCOUNT_NAME_BYTES {
            return Err(StoreError::InvalidPayload {
                kind: RECORD_ACCOUNTS,
            });
        }
        push_u64(&mut out, account.id);
        out.push(account.policy.name.len() as u8);
        out.extend_from_slice(account.policy.name.as_bytes());
        let mut flags = 0u8;
        if account.policy.administrator {
            flags |= 1;
        }
        if account.policy.disabled {
            flags |= 2;
        }
        out.push(flags);
        out.extend_from_slice(&account.record.salt);
        out.extend_from_slice(&account.record.verifier);
    }
    Ok(out)
}
fn decode_accounts_v2(bytes: &[u8]) -> Result<Vec<PersistedAccount>, StoreError> {
    let mut offset = 0;
    let count = take_u16(bytes, &mut offset)? as usize;
    if count > MAX_ACCOUNTS {
        return Err(StoreError::TooManyRecords);
    }
    let mut accounts = Vec::with_capacity(count);
    for _ in 0..count {
        let id = take_u64(bytes, &mut offset)?;
        let name_len = take(bytes, &mut offset, 1)?[0] as usize;
        if name_len == 0 || name_len > MAX_ACCOUNT_NAME_BYTES {
            return Err(StoreError::InvalidPayload {
                kind: RECORD_ACCOUNTS,
            });
        }
        let name = core::str::from_utf8(take(bytes, &mut offset, name_len)?)
            .map_err(|_| StoreError::InvalidPayload {
                kind: RECORD_ACCOUNTS,
            })?
            .into();
        let flags = take(bytes, &mut offset, 1)?[0];
        if flags & !0b11 != 0 {
            return Err(StoreError::InvalidPayload {
                kind: RECORD_ACCOUNTS,
            });
        }
        let mut salt = [0u8; 16];
        salt.copy_from_slice(take(bytes, &mut offset, 16)?);
        let mut verifier = [0u8; 32];
        verifier.copy_from_slice(take(bytes, &mut offset, 32)?);
        accounts.push(PersistedAccount {
            id,
            policy: AccountPolicy {
                name,
                administrator: flags & 1 != 0,
                disabled: flags & 2 != 0,
            },
            record: vault::PasswordRecord { salt, verifier },
        });
    }
    Ok(accounts)
}
fn decode_accounts_v1(bytes: &[u8]) -> Result<Vec<PersistedAccount>, StoreError> {
    let text = core::str::from_utf8(bytes).map_err(|_| StoreError::InvalidPayload {
        kind: RECORD_ACCOUNTS,
    })?;
    let mut accounts = Vec::new();
    let mut next_id: UserId = 1;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('|');
        let name = parts.next().ok_or(StoreError::InvalidPayload {
            kind: RECORD_ACCOUNTS,
        })?;
        let administrator = parts.next().ok_or(StoreError::InvalidPayload {
            kind: RECORD_ACCOUNTS,
        })?;
        let salt_text = parts.next().ok_or(StoreError::InvalidPayload {
            kind: RECORD_ACCOUNTS,
        })?;
        let verifier_text = parts.next().ok_or(StoreError::InvalidPayload {
            kind: RECORD_ACCOUNTS,
        })?;
        if parts.next().is_some() {
            return Err(StoreError::InvalidPayload {
                kind: RECORD_ACCOUNTS,
            });
        }
        if name.is_empty() || name.len() > MAX_ACCOUNT_NAME_BYTES || accounts.len() >= MAX_ACCOUNTS
        {
            return Err(StoreError::InvalidPayload {
                kind: RECORD_ACCOUNTS,
            });
        }
        let administrator = match administrator {
            "admin" => true,
            "user" => false,
            _ => {
                return Err(StoreError::InvalidPayload {
                    kind: RECORD_ACCOUNTS,
                })
            }
        };
        let mut salt = [0u8; 16];
        if !hex_decode(salt_text, &mut salt) {
            return Err(StoreError::InvalidPayload {
                kind: RECORD_ACCOUNTS,
            });
        }
        let mut verifier = [0u8; 32];
        if !hex_decode(verifier_text, &mut verifier) {
            return Err(StoreError::InvalidPayload {
                kind: RECORD_ACCOUNTS,
            });
        }
        accounts.push(PersistedAccount {
            id: next_id,
            policy: AccountPolicy {
                name: String::from(name),
                administrator,
                disabled: false,
            },
            record: vault::PasswordRecord { salt, verifier },
        });
        next_id = next_id.saturating_add(1);
    }
    Ok(accounts)
}
pub fn decode_accounts(version: u16, bytes: &[u8]) -> Result<Vec<PersistedAccount>, StoreError> {
    match version {
        ACCOUNTS_VERSION => decode_accounts_v2(bytes),
        ACCOUNTS_LEGACY_VERSION => decode_accounts_v1(bytes),
        other => Err(StoreError::MigrationRequired {
            kind: RECORD_ACCOUNTS,
            found: other,
        }),
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultEntry {
    pub user: UserId,
    pub label: String,
    pub secret: vault::EncryptedSecret,
}
pub fn vault_domain(user: UserId, label: &str) -> Vec<u8> {
    let mut out = Vec::from(b"lixos/vault/v1|" as &[u8]);
    out.extend_from_slice(&user.to_le_bytes());
    out.push(b'|');
    out.extend_from_slice(label.as_bytes());
    out
}
pub fn encode_vault(entries: &[VaultEntry]) -> Result<Vec<u8>, StoreError> {
    if entries.len() > MAX_VAULT_ENTRIES {
        return Err(StoreError::TooManyRecords);
    }
    let mut out = Vec::new();
    push_u16(&mut out, entries.len() as u16);
    for entry in entries {
        if entry.label.is_empty() || entry.label.len() > MAX_VAULT_LABEL_BYTES {
            return Err(StoreError::InvalidPayload { kind: RECORD_VAULT });
        }
        if entry.secret.ciphertext.len() > MAX_SECRET_BYTES {
            return Err(StoreError::InvalidPayload { kind: RECORD_VAULT });
        }
        push_u64(&mut out, entry.user);
        out.push(entry.label.len() as u8);
        out.extend_from_slice(entry.label.as_bytes());
        push_u16(&mut out, entry.secret.version);
        out.extend_from_slice(&entry.secret.nonce);
        push_u32(&mut out, entry.secret.ciphertext.len() as u32);
        out.extend_from_slice(&entry.secret.ciphertext);
    }
    Ok(out)
}
pub fn decode_vault(version: u16, bytes: &[u8]) -> Result<Vec<VaultEntry>, StoreError> {
    if version != VAULT_VERSION {
        return Err(StoreError::MigrationRequired {
            kind: RECORD_VAULT,
            found: version,
        });
    }
    let mut offset = 0;
    let count = take_u16(bytes, &mut offset)? as usize;
    if count > MAX_VAULT_ENTRIES {
        return Err(StoreError::TooManyRecords);
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let user = take_u64(bytes, &mut offset)?;
        let label_len = take(bytes, &mut offset, 1)?[0] as usize;
        if label_len == 0 || label_len > MAX_VAULT_LABEL_BYTES {
            return Err(StoreError::InvalidPayload { kind: RECORD_VAULT });
        }
        let label = core::str::from_utf8(take(bytes, &mut offset, label_len)?)
            .map_err(|_| StoreError::InvalidPayload { kind: RECORD_VAULT })?
            .into();
        let secret_version = take_u16(bytes, &mut offset)?;
        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(take(bytes, &mut offset, 12)?);
        let length = take_u32(bytes, &mut offset)? as usize;
        if length > MAX_SECRET_BYTES {
            return Err(StoreError::InvalidPayload { kind: RECORD_VAULT });
        }
        let ciphertext = take(bytes, &mut offset, length)?.to_vec();
        entries.push(VaultEntry {
            user,
            label,
            secret: vault::EncryptedSecret {
                version: secret_version,
                nonce,
                ciphertext,
            },
        });
    }
    Ok(entries)
}
pub fn migrate_records(records: Vec<StoredRecord>) -> Result<Vec<StoredRecord>, StoreError> {
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        let migrated = match record.kind {
            RECORD_POLICY => {
                let policy = Policy::decode(record.version, &record.payload)?;
                StoredRecord {
                    kind: RECORD_POLICY,
                    version: POLICY_VERSION,
                    payload: policy.encode()?,
                }
            }
            RECORD_ACCOUNTS => {
                let accounts = decode_accounts(record.version, &record.payload)?;
                StoredRecord {
                    kind: RECORD_ACCOUNTS,
                    version: ACCOUNTS_VERSION,
                    payload: encode_accounts(&accounts)?,
                }
            }
            RECORD_VAULT => {
                if record.version != VAULT_VERSION {
                    return Err(StoreError::MigrationRequired {
                        kind: RECORD_VAULT,
                        found: record.version,
                    });
                }
                decode_vault(record.version, &record.payload)?;
                record
            }
            other => return Err(StoreError::MissingRecord(other)),
        };
        out.push(migrated);
    }
    Ok(out)
}
pub trait BlobStore {
    fn read(&mut self, path: &str) -> Result<Option<Vec<u8>>, StoreError>;
    fn write_atomic(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreError>;
}
#[derive(Default)]
pub struct MemoryBlobStore {
    entries: BTreeMap<String, Vec<u8>>,
    staged: BTreeMap<String, Vec<u8>>,
    fail_before_commit: bool,
}
impl MemoryBlobStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub const fn set_fail_before_commit(&mut self, fail: bool) {
        self.fail_before_commit = fail;
    }
    pub fn staged_paths(&self) -> Vec<String> {
        self.staged.keys().cloned().collect()
    }
    pub fn recover(&mut self, path: &str) {
        self.staged.remove(path);
    }
}
impl BlobStore for MemoryBlobStore {
    fn read(&mut self, path: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.entries.get(path).cloned())
    }
    fn write_atomic(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let temp = String::from(path) + ".tmp";
        self.staged.insert(temp.clone(), bytes.to_vec());
        if self.fail_before_commit {
            return Err(StoreError::AtomicWriteUnavailable);
        }
        self.staged.remove(&temp);
        self.entries.insert(String::from(path), bytes.to_vec());
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthStore {
    pub policy: Policy,
    pub accounts: Vec<PersistedAccount>,
    pub vault: Vec<VaultEntry>,
}
impl AuthStore {
    pub fn empty() -> Self {
        Self {
            policy: Policy::default(),
            accounts: Vec::new(),
            vault: Vec::new(),
        }
    }
    pub fn find_account(&self, id: UserId) -> Option<&PersistedAccount> {
        self.accounts.iter().find(|account| account.id == id)
    }
    pub fn load<B: BlobStore>(io: &mut B, path: &str) -> Result<Self, StoreError> {
        let bytes = io.read(path)?.ok_or(StoreError::NotFound)?;
        let records = migrate_records(decode_store(&bytes)?)?;
        let mut policy = None;
        let mut accounts = None;
        let mut vault = None;
        for record in &records {
            match record.kind {
                RECORD_POLICY => {
                    if policy.is_some() {
                        return Err(StoreError::DuplicateRecord(RECORD_POLICY));
                    }
                    policy = Some(Policy::decode(record.version, &record.payload)?);
                }
                RECORD_ACCOUNTS => {
                    if accounts.is_some() {
                        return Err(StoreError::DuplicateRecord(RECORD_ACCOUNTS));
                    }
                    accounts = Some(decode_accounts(record.version, &record.payload)?);
                }
                RECORD_VAULT => {
                    if vault.is_some() {
                        return Err(StoreError::DuplicateRecord(RECORD_VAULT));
                    }
                    vault = Some(decode_vault(record.version, &record.payload)?);
                }
                other => return Err(StoreError::MissingRecord(other)),
            }
        }
        Ok(Self {
            policy: policy.ok_or(StoreError::MissingRecord(RECORD_POLICY))?,
            accounts: accounts.ok_or(StoreError::MissingRecord(RECORD_ACCOUNTS))?,
            vault: vault.ok_or(StoreError::MissingRecord(RECORD_VAULT))?,
        })
    }
    pub fn save<B: BlobStore>(&self, io: &mut B, path: &str) -> Result<(), StoreError> {
        let records = [
            StoredRecord {
                kind: RECORD_POLICY,
                version: POLICY_VERSION,
                payload: self.policy.encode()?,
            },
            StoredRecord {
                kind: RECORD_ACCOUNTS,
                version: ACCOUNTS_VERSION,
                payload: encode_accounts(&self.accounts)?,
            },
            StoredRecord {
                kind: RECORD_VAULT,
                version: VAULT_VERSION,
                payload: encode_vault(&self.vault)?,
            },
        ];
        let bytes = encode_store(&records)?;
        io.write_atomic(path, &bytes)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    fn sample_store() -> AuthStore {
        let mut store = AuthStore::empty();
        store.policy = Policy {
            max_failed_attempts: 7,
            max_lockout_seconds: 60,
            max_elevation_seconds: 120,
        };
        store.accounts.push(PersistedAccount {
            id: 42,
            policy: AccountPolicy {
                name: String::from("ana"),
                administrator: true,
                disabled: false,
            },
            record: vault::PasswordRecord {
                salt: [1; 16],
                verifier: [2; 32],
            },
        });
        store.vault.push(VaultEntry {
            user: 42,
            label: String::from("wifi/home"),
            secret: vault::EncryptedSecret {
                version: vault::VAULT_FORMAT_VERSION,
                nonce: [3; 12],
                ciphertext: Vec::from(&[9u8; 24][..]),
            },
        });
        store
    }
    #[test]
    fn store_roundtrip_preserves_policy_accounts_and_vault() {
        let store = sample_store();
        let mut io = MemoryBlobStore::new();
        store.save(&mut io, "/var/authd/store.bin").unwrap();
        let loaded = AuthStore::load(&mut io, "/var/authd/store.bin").unwrap();
        assert_eq!(loaded, store);
    }
    #[test]
    fn missing_store_reports_not_found() {
        let mut io = MemoryBlobStore::new();
        assert_eq!(
            AuthStore::load(&mut io, "/var/authd/absent.bin"),
            Err(StoreError::NotFound)
        );
    }
    #[test]
    fn corrupt_payload_is_detected_and_store_bytes_are_untouched() {
        let store = sample_store();
        let mut io = MemoryBlobStore::new();
        store.save(&mut io, "/var/authd/store.bin").unwrap();
        let original = io.read("/var/authd/store.bin").unwrap().unwrap();
        let mut corrupted = original.clone();
        let target = corrupted.len() - 4;
        corrupted[target] ^= 0xFF;
        io.write_atomic("/var/authd/store.bin", &corrupted).unwrap();
        assert_eq!(
            AuthStore::load(&mut io, "/var/authd/store.bin"),
            Err(StoreError::ChecksumMismatch { index: 2 })
        );
        let after = io.read("/var/authd/store.bin").unwrap().unwrap();
        assert_eq!(after, corrupted);
        io.write_atomic("/var/authd/store.bin", &original).unwrap();
        assert!(AuthStore::load(&mut io, "/var/authd/store.bin").is_ok());
    }
    #[test]
    fn corrupt_record_framing_is_detected() {
        let store = sample_store();
        let records = [
            StoredRecord {
                kind: RECORD_POLICY,
                version: POLICY_VERSION,
                payload: store.policy.encode().unwrap(),
            },
            StoredRecord {
                kind: RECORD_ACCOUNTS,
                version: ACCOUNTS_VERSION,
                payload: encode_accounts(&store.accounts).unwrap(),
            },
            StoredRecord {
                kind: RECORD_VAULT,
                version: VAULT_VERSION,
                payload: encode_vault(&store.vault).unwrap(),
            },
        ];
        let mut bytes = encode_store(&records).unwrap();
        let policy_payload_len = records[0].payload.len();
        let index =
            STORE_HEADER_BYTES + RECORD_HEADER_BYTES + policy_payload_len + RECORD_HEADER_BYTES + 5;
        bytes[index] ^= 0x01;
        assert!(matches!(
            decode_store(&bytes),
            Err(StoreError::ChecksumMismatch { index: 1 })
        ));
    }
    #[test]
    fn corrupt_header_is_detected() {
        let store = sample_store();
        let mut io = MemoryBlobStore::new();
        store.save(&mut io, "/var/authd/store.bin").unwrap();
        let mut bytes = io.read("/var/authd/store.bin").unwrap().unwrap();
        bytes[3] ^= 0x20;
        io.write_atomic("/var/authd/store.bin", &bytes).unwrap();
        assert_eq!(
            AuthStore::load(&mut io, "/var/authd/store.bin"),
            Err(StoreError::HeaderCorrupt)
        );
    }
    #[test]
    fn truncated_store_is_detected() {
        let store = sample_store();
        let mut io = MemoryBlobStore::new();
        store.save(&mut io, "/var/authd/store.bin").unwrap();
        let bytes = io.read("/var/authd/store.bin").unwrap().unwrap();
        let cut = &bytes[..bytes.len() - 3];
        assert_eq!(decode_store(cut), Err(StoreError::Truncated));
    }
    #[test]
    fn unsupported_format_version_is_rejected() {
        let store = sample_store();
        let mut bytes = encode_store(&[StoredRecord {
            kind: RECORD_POLICY,
            version: POLICY_VERSION,
            payload: store.policy.encode().unwrap(),
        }])
        .unwrap();
        bytes[8] = 9;
        let stored = crc32(&bytes[..12]);
        bytes[12..16].copy_from_slice(&stored.to_le_bytes());
        assert_eq!(
            decode_store(&bytes),
            Err(StoreError::UnsupportedVersion { found: 9 })
        );
    }
    #[test]
    fn legacy_accounts_migrate_to_current_version() {
        let legacy = b"ana|admin|0102030405060708090a0b0c0d0e0f10|112233445566778899aabbccddeeff00112233445566778899aabbccddeeff00\n";
        let legacy_text = String::from_utf8(legacy.to_vec()).unwrap();
        let records = vec![
            StoredRecord {
                kind: RECORD_POLICY,
                version: POLICY_LEGACY_VERSION,
                payload: Vec::from(
                    &b"max_failed_attempts=9\nmax_lockout_seconds=45\nmax_elevation_seconds=90\n"[..],
                ),
            },
            StoredRecord {
                kind: RECORD_ACCOUNTS,
                version: ACCOUNTS_LEGACY_VERSION,
                payload: legacy_text.into_bytes(),
            },
            StoredRecord {
                kind: RECORD_VAULT,
                version: VAULT_VERSION,
                payload: encode_vault(&[]).unwrap(),
            },
        ];
        let bytes = encode_store(&records).unwrap();
        let migrated = migrate_records(decode_store(&bytes).unwrap()).unwrap();
        assert_eq!(migrated[0].version, POLICY_VERSION);
        assert_eq!(migrated[1].version, ACCOUNTS_VERSION);
        assert_eq!(migrated[2].version, VAULT_VERSION);
        let accounts = decode_accounts(migrated[1].version, &migrated[1].payload).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, 1);
        assert_eq!(accounts[0].policy.name, "ana");
        assert!(accounts[0].policy.administrator);
        assert!(!accounts[0].policy.disabled);
        assert_eq!(accounts[0].record.salt[0], 0x01);
        let policy = Policy::decode(migrated[0].version, &migrated[0].payload).unwrap();
        assert_eq!(policy.max_failed_attempts, 9);
        assert_eq!(policy.max_lockout_seconds, 45);
        assert_eq!(policy.max_elevation_seconds, 90);
    }
    #[test]
    fn legacy_store_loads_through_migration() {
        let records = vec![
            StoredRecord {
                kind: RECORD_POLICY,
                version: POLICY_LEGACY_VERSION,
                payload: Vec::from(&b"max_failed_attempts=4\n"[..]),
            },
            StoredRecord {
                kind: RECORD_ACCOUNTS,
                version: ACCOUNTS_LEGACY_VERSION,
                payload: Vec::from(&b"root|admin|00000000000000000000000000000000|0000000000000000000000000000000000000000000000000000000000000000\n"[..]),
            },
            StoredRecord {
                kind: RECORD_VAULT,
                version: VAULT_VERSION,
                payload: encode_vault(&[]).unwrap(),
            },
        ];
        let bytes = encode_store(&records).unwrap();
        let mut io = MemoryBlobStore::new();
        io.write_atomic("/var/authd/store.bin", &bytes).unwrap();
        let loaded = AuthStore::load(&mut io, "/var/authd/store.bin").unwrap();
        assert_eq!(loaded.policy.max_failed_attempts, 4);
        assert_eq!(loaded.accounts.len(), 1);
        let mut saved = MemoryBlobStore::new();
        loaded.save(&mut saved, "/var/authd/store.bin").unwrap();
        let reloaded = AuthStore::load(&mut saved, "/var/authd/store.bin").unwrap();
        assert_eq!(reloaded, loaded);
        assert_eq!(reloaded.policy.max_lockout_seconds, 300);
    }
    #[test]
    fn write_commits_only_after_atomic_commit() {
        let store = sample_store();
        let mut io = MemoryBlobStore::new();
        store.save(&mut io, "/var/authd/store.bin").unwrap();
        let first = io.read("/var/authd/store.bin").unwrap().unwrap();
        let mut next = store.clone();
        next.accounts[0].policy.disabled = true;
        io.set_fail_before_commit(true);
        assert_eq!(
            next.save(&mut io, "/var/authd/store.bin"),
            Err(StoreError::AtomicWriteUnavailable)
        );
        assert_eq!(io.read("/var/authd/store.bin").unwrap().unwrap(), first);
        assert_eq!(
            io.staged_paths(),
            vec![String::from("/var/authd/store.bin.tmp")]
        );
        io.set_fail_before_commit(false);
        next.save(&mut io, "/var/authd/store.bin").unwrap();
        assert!(io.staged_paths().is_empty());
        let reloaded = AuthStore::load(&mut io, "/var/authd/store.bin").unwrap();
        assert!(reloaded.accounts[0].policy.disabled);
    }
    fn test_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("lixos-authd-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
    #[test]
    fn std_store_writes_through_temporary_file() {
        let dir = test_dir("atomic");
        let path = dir.join("store.bin");
        let store = sample_store();
        store
            .save(&mut StdBlobStore, path.to_str().unwrap())
            .unwrap();
        let bytes = fs::read(&path).unwrap();
        let temp = dir.join("store.bin.tmp");
        assert!(!temp.exists());
        assert!(decode_store(&bytes).is_ok());
        let mut next = store.clone();
        next.policy.max_failed_attempts = 11;
        let mut io = StdBlobStore;
        io.stage(
            path.to_str().unwrap(),
            &encode_store(&[StoredRecord {
                kind: RECORD_POLICY,
                version: POLICY_VERSION,
                payload: next.policy.encode().unwrap(),
            }])
            .unwrap(),
        )
        .unwrap();
        let staged = fs::read(&path).unwrap();
        assert_eq!(staged, bytes);
        io.commit(path.to_str().unwrap()).unwrap();
        assert!(!temp.exists());
        let replaced = fs::read(&path).unwrap();
        assert_ne!(replaced, bytes);
        let _ = fs::remove_dir_all(&dir);
    }
    #[test]
    fn vault_domain_separates_users_and_labels() {
        assert_ne!(vault_domain(1, "wifi"), vault_domain(2, "wifi"));
        assert_ne!(vault_domain(1, "wifi"), vault_domain(1, "bt"));
        assert_eq!(vault_domain(1, "wifi"), vault_domain(1, "wifi"));
    }
    struct StdBlobStore;
    impl StdBlobStore {
        fn stage(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreError> {
            let temp = format!("{path}.tmp");
            fs::write(&temp, bytes).map_err(|_| StoreError::Io("stage failed"))?;
            Ok(())
        }
        fn commit(&mut self, path: &str) -> Result<(), StoreError> {
            let temp = format!("{path}.tmp");
            fs::rename(&temp, path).map_err(|_| StoreError::Io("commit failed"))?;
            Ok(())
        }
    }
    impl BlobStore for StdBlobStore {
        fn read(&mut self, path: &str) -> Result<Option<Vec<u8>>, StoreError> {
            match fs::read(path) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(_) => Err(StoreError::Io("read failed")),
            }
        }
        fn write_atomic(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreError> {
            self.stage(path, bytes)?;
            self.commit(path)
        }
    }
}
