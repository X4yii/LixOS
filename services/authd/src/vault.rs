use aes_gcm_siv::{
    aead::{Aead, KeyInit, Payload},
    Aes256GcmSiv, Nonce,
};
use alloc::vec::Vec;
use argon2::{Algorithm, Argon2, Params, Version};
use subtle::ConstantTimeEq;
const ARGON_MEMORY_KIB: u32 = 19 * 1024;
const ARGON_ITERATIONS: u32 = 2;
const ARGON_LANES: u32 = 1;
const MAX_PASSWORD_BYTES: usize = 1024;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CryptoError {
    InvalidPassword,
    InvalidParameters,
    DerivationFailed,
    EncryptionFailed,
    AuthenticationFailed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PasswordRecord {
    pub salt: [u8; 16],
    pub verifier: [u8; 32],
}
impl PasswordRecord {
    pub fn create(password: &[u8], salt: [u8; 16]) -> Result<Self, CryptoError> {
        validate_password(password)?;
        let mut verifier = [0; 32];
        if let Err(error) = argon2id(password, &salt, &mut verifier) {
            erase(&mut verifier);
            return Err(error);
        }
        Ok(Self { salt, verifier })
    }
    pub fn verify(&self, password: &[u8]) -> Result<bool, CryptoError> {
        validate_password(password)?;
        let mut candidate = [0; 32];
        if let Err(error) = argon2id(password, &self.salt, &mut candidate) {
            erase(&mut candidate);
            return Err(error);
        }
        let valid = bool::from(candidate.ct_eq(&self.verifier));
        erase(&mut candidate);
        Ok(valid)
    }
}
const fn validate_password(password: &[u8]) -> Result<(), CryptoError> {
    if password.len() < 8 || password.len() > MAX_PASSWORD_BYTES {
        return Err(CryptoError::InvalidPassword);
    }
    Ok(())
}
fn argon2id(password: &[u8], salt: &[u8; 16], output: &mut [u8; 32]) -> Result<(), CryptoError> {
    let params = Params::new(ARGON_MEMORY_KIB, ARGON_ITERATIONS, ARGON_LANES, Some(32))
        .map_err(|_| CryptoError::InvalidParameters)?;
    let algorithm = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    algorithm
        .hash_password_into(password, salt, output)
        .map_err(|_| CryptoError::DerivationFailed)
}
pub fn derive_wrapping_key(password: &[u8], salt: &[u8; 16]) -> Result<VaultKey, CryptoError> {
    validate_password(password)?;
    let mut key = [0; 32];
    if let Err(error) = argon2id(password, salt, &mut key) {
        erase(&mut key);
        return Err(error);
    }
    Ok(VaultKey(key))
}
pub const VAULT_FORMAT_VERSION: u16 = 1;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptedSecret {
    pub version: u16,
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}
pub struct VaultKey([u8; 32]);
impl VaultKey {
    pub fn seal(
        &self,
        nonce: [u8; 12],
        associated_data: &[u8],
        plaintext: &[u8],
    ) -> Result<EncryptedSecret, CryptoError> {
        let cipher =
            Aes256GcmSiv::new_from_slice(&self.0).map_err(|_| CryptoError::InvalidParameters)?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: associated_data,
                },
            )
            .map_err(|_| CryptoError::EncryptionFailed)?;
        Ok(EncryptedSecret {
            version: VAULT_FORMAT_VERSION,
            nonce,
            ciphertext,
        })
    }
    pub fn open(
        &self,
        record: &EncryptedSecret,
        associated_data: &[u8],
    ) -> Result<SecretBytes, CryptoError> {
        if record.version != VAULT_FORMAT_VERSION {
            return Err(CryptoError::InvalidParameters);
        }
        let cipher =
            Aes256GcmSiv::new_from_slice(&self.0).map_err(|_| CryptoError::InvalidParameters)?;
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&record.nonce),
                Payload {
                    msg: &record.ciphertext,
                    aad: associated_data,
                },
            )
            .map_err(|_| CryptoError::AuthenticationFailed)?;
        Ok(SecretBytes(plaintext))
    }
}
pub fn wrap_vault_key(
    password: &[u8],
    wrapping_salt: &[u8; 16],
    nonce: [u8; 12],
    random_vault_key: &[u8; 32],
    associated_data: &[u8],
) -> Result<EncryptedSecret, CryptoError> {
    derive_wrapping_key(password, wrapping_salt)?.seal(nonce, associated_data, random_vault_key)
}
pub fn unlock_vault_key(
    password: &[u8],
    wrapping_salt: &[u8; 16],
    wrapped: &EncryptedSecret,
    associated_data: &[u8],
) -> Result<VaultKey, CryptoError> {
    let wrapping_key = derive_wrapping_key(password, wrapping_salt)?;
    let plaintext = wrapping_key.open(wrapped, associated_data)?;
    let key: [u8; 32] = plaintext
        .expose()
        .try_into()
        .map_err(|_| CryptoError::AuthenticationFailed)?;
    Ok(VaultKey(key))
}
pub struct SecretBytes(Vec<u8>);
impl SecretBytes {
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}
impl Drop for SecretBytes {
    fn drop(&mut self) {
        erase(&mut self.0);
    }
}
impl Drop for VaultKey {
    fn drop(&mut self) {
        erase(&mut self.0);
    }
}
fn erase(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe {
            core::ptr::write_volatile(byte, 0);
        }
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}
