//! Credential storage for r8r.
//!
//! Stores credentials in ~/.r8r/credentials.json with AES-256-GCM encryption.
//! A master key is derived from a user password using PBKDF2.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::pbkdf2;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::config::Config;
use crate::error::{Error, Result};

/// Secure container for the master key that zeroizes on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
struct SecureMasterKey(Vec<u8>);

impl SecureMasterKey {
    fn new(key: Vec<u8>) -> Self {
        Self(key)
    }

    fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Number of PBKDF2 iterations for key derivation.
const PBKDF2_ITERATIONS: u32 = 100_000;

/// Salt length in bytes.
const SALT_LEN: usize = 16;

/// Nonce length for AES-GCM.
const NONCE_LEN: usize = 12;
/// File mode for credential files on Unix systems (owner read/write only).
const CREDENTIAL_FILE_MODE: u32 = 0o600;

/// A stored credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub service: String,
    pub key: Option<String>,
    /// Encrypted or base64-encoded value (format: "enc:base64(nonce+ciphertext)" or legacy base64)
    pub value: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Master key file structure.
#[derive(Debug, Serialize, Deserialize)]
struct MasterKeyFile {
    /// Salt used for PBKDF2 key derivation
    salt: String,
    /// Encrypted master key (nonce + ciphertext, base64-encoded)
    encrypted_key: String,
}

/// Credential store backed by a JSON file.
#[derive(Default, Serialize, Deserialize)]
pub struct CredentialStore {
    credentials: HashMap<String, Credential>,
    /// Master key for encryption (not serialized, zeroized on drop)
    #[serde(skip)]
    master_key: Option<SecureMasterKey>,
}

impl std::fmt::Debug for CredentialStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialStore")
            .field("credentials", &self.credentials)
            .field(
                "master_key",
                &self.master_key.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl CredentialStore {
    /// Get the path to the credentials file.
    pub fn path() -> PathBuf {
        Config::data_dir().join("credentials.json")
    }

    /// Get the path to the master key file.
    pub fn master_key_path() -> PathBuf {
        Config::data_dir().join("master.key")
    }

    /// Load credentials from disk (without master key - can only list, not decrypt).
    pub async fn load() -> Result<Self> {
        let path = Self::path();
        if !tokio::fs::try_exists(&path)
            .await
            .map_err(|e| Error::Storage(format!("Failed to check credentials path: {}", e)))?
        {
            return Ok(Self::default());
        }

        let content = read_file(&path, "credentials").await?;

        serde_json::from_str(&content)
            .map_err(|e| Error::Storage(format!("Failed to parse credentials: {}", e)))
    }

    /// Load credentials and unlock with password.
    pub async fn load_with_password(password: &str) -> Result<Self> {
        let mut store = Self::load().await?;
        store.unlock(password).await?;
        Ok(store)
    }

    /// Check if encryption is initialized (master key file exists).
    pub fn is_encryption_initialized() -> bool {
        Self::master_key_path().exists()
    }

    /// Initialize encryption with a new password.
    /// Creates a new master key and encrypts it with the password.
    pub async fn initialize_encryption(password: &str) -> Result<()> {
        if Self::is_encryption_initialized() {
            return Err(Error::Storage(
                "Encryption already initialized. Use unlock instead.".to_string(),
            ));
        }

        let rng = SystemRandom::new();

        // Generate random salt
        let mut salt = [0u8; SALT_LEN];
        rng.fill(&mut salt)
            .map_err(|_| Error::Storage("Failed to generate salt".to_string()))?;

        // Generate random master key
        let mut master_key = [0u8; 32];
        rng.fill(&mut master_key)
            .map_err(|_| Error::Storage("Failed to generate master key".to_string()))?;

        // Derive encryption key from password
        let mut derived_key = [0u8; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            std::num::NonZeroU32::new(PBKDF2_ITERATIONS).unwrap(),
            &salt,
            password.as_bytes(),
            &mut derived_key,
        );

        // Encrypt the master key with the derived key
        let encrypted_master_key = encrypt_data(&master_key, &derived_key, &rng)?;

        // Save master key file
        let master_key_file = MasterKeyFile {
            salt: STANDARD.encode(salt),
            encrypted_key: STANDARD.encode(encrypted_master_key),
        };

        let path = Self::master_key_path();
        let content = serde_json::to_string_pretty(&master_key_file)
            .map_err(|e| Error::Storage(format!("Failed to serialize master key: {}", e)))?;
        write_secure_file(&path, &content, "master key").await?;

        Ok(())
    }

    /// Unlock the credential store with a password.
    pub async fn unlock(&mut self, password: &str) -> Result<()> {
        if !Self::is_encryption_initialized() {
            // No encryption - nothing to unlock
            return Ok(());
        }

        let path = Self::master_key_path();
        let content = read_file(&path, "master key").await?;

        let master_key_file: MasterKeyFile = serde_json::from_str(&content)
            .map_err(|e| Error::Storage(format!("Failed to parse master key: {}", e)))?;

        let salt = STANDARD
            .decode(&master_key_file.salt)
            .map_err(|_| Error::Storage("Invalid password".to_string()))?;

        let encrypted_key = STANDARD
            .decode(&master_key_file.encrypted_key)
            .map_err(|_| Error::Storage("Invalid password".to_string()))?;

        // Derive encryption key from password
        let mut derived_key = [0u8; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            std::num::NonZeroU32::new(PBKDF2_ITERATIONS).unwrap(),
            &salt,
            password.as_bytes(),
            &mut derived_key,
        );

        // Decrypt the master key
        let master_key = decrypt_data(&encrypted_key, &derived_key)
            .map_err(|_| Error::Storage("Invalid password".to_string()))?;

        self.master_key = Some(SecureMasterKey::new(master_key));
        Ok(())
    }

    /// Save credentials to disk.
    pub async fn save(&self) -> Result<()> {
        let path = Self::path();

        let content = serde_json::to_string_pretty(&self)
            .map_err(|e| Error::Storage(format!("Failed to serialize credentials: {}", e)))?;
        write_secure_file(&path, &content, "credentials").await
    }

    /// Set a credential.
    pub async fn set(&mut self, service: &str, key: Option<&str>, value: &str) -> Result<()> {
        let now = chrono::Utc::now();
        let rng = SystemRandom::new();

        // Encrypt if master key is available, otherwise use base64
        let encoded_value = if let Some(master_key) = &self.master_key {
            let encrypted = encrypt_data(value.as_bytes(), master_key.as_bytes(), &rng)?;
            format!("enc:{}", STANDARD.encode(encrypted))
        } else {
            // Legacy base64 encoding
            STANDARD.encode(value.as_bytes())
        };

        let credential = Credential {
            service: service.to_string(),
            key: key.map(|k| k.to_string()),
            value: encoded_value,
            created_at: self
                .credentials
                .get(service)
                .map(|c| c.created_at)
                .unwrap_or(now),
            updated_at: now,
        };

        self.credentials.insert(service.to_string(), credential);
        self.save().await
    }

    /// Get a credential value (decrypted/decoded).
    pub fn get(&self, service: &str) -> Result<Option<String>> {
        match self.credentials.get(service) {
            Some(cred) => {
                let value = if cred.value.starts_with("enc:") {
                    // Encrypted value
                    let master_key = self.master_key.as_ref().ok_or_else(|| {
                        Error::Storage(
                            "Credential is encrypted but store is not unlocked".to_string(),
                        )
                    })?;

                    let encrypted = STANDARD
                        .decode(&cred.value[4..])
                        .map_err(|_| Error::Storage("Failed to decrypt credential".to_string()))?;

                    let decrypted = decrypt_data(&encrypted, master_key.as_bytes())
                        .map_err(|_| Error::Storage("Failed to decrypt credential".to_string()))?;

                    String::from_utf8(decrypted)
                        .map_err(|_| Error::Storage("Failed to decrypt credential".to_string()))?
                } else {
                    // Legacy base64-encoded value
                    let decoded = STANDARD.decode(&cred.value).map_err(|e| {
                        Error::Storage(format!("Failed to decode credential: {}", e))
                    })?;

                    String::from_utf8(decoded).map_err(|e| {
                        Error::Storage(format!("Invalid UTF-8 in credential: {}", e))
                    })?
                };

                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// List all credentials (without decoding values).
    pub fn list(&self) -> Vec<&Credential> {
        self.credentials.values().collect()
    }

    /// Delete a credential.
    pub async fn delete(&mut self, service: &str) -> Result<bool> {
        let existed = self.credentials.remove(service).is_some();
        if existed {
            self.save().await?;
        }
        Ok(existed)
    }

    /// Migrate legacy base64 credentials to encrypted format.
    /// Requires the store to be unlocked with a master key.
    pub async fn migrate_to_encrypted(&mut self) -> Result<usize> {
        let master_key = self.master_key.as_ref().ok_or_else(|| {
            Error::Storage("Store must be unlocked to migrate credentials".to_string())
        })?;

        let rng = SystemRandom::new();
        let mut migrated = 0;

        let services: Vec<String> = self.credentials.keys().cloned().collect();
        for service in services {
            if let Some(cred) = self.credentials.get(&service) {
                if !cred.value.starts_with("enc:") {
                    // Legacy credential - decrypt and re-encrypt
                    let decoded = STANDARD.decode(&cred.value).map_err(|e| {
                        Error::Storage(format!("Failed to decode credential: {}", e))
                    })?;

                    let encrypted = encrypt_data(&decoded, master_key.as_bytes(), &rng)?;
                    let new_value = format!("enc:{}", STANDARD.encode(encrypted));

                    if let Some(cred) = self.credentials.get_mut(&service) {
                        cred.value = new_value;
                        cred.updated_at = chrono::Utc::now();
                        migrated += 1;
                    }
                }
            }
        }

        if migrated > 0 {
            self.save().await?;
        }

        Ok(migrated)
    }

    /// Mask a credential value for display.
    pub fn mask_value(value: &str) -> String {
        if value.len() <= 4 {
            "*".repeat(value.len())
        } else {
            format!("{}...{}", &value[..2], &value[value.len() - 2..])
        }
    }

    /// Check if this credential is encrypted.
    pub fn is_encrypted(credential: &Credential) -> bool {
        credential.value.starts_with("enc:")
    }
}

async fn read_file(path: &Path, label: &str) -> Result<String> {
    tokio::fs::read_to_string(path)
        .await
        .map_err(|e| Error::Storage(format!("Failed to read {}: {}", label, e)))
}

async fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| Error::Storage(format!("Failed to create directory: {}", e)))?;
    }
    Ok(())
}

async fn write_secure_file(path: &Path, content: &str, label: &str) -> Result<()> {
    ensure_parent_dir(path).await?;
    tokio::fs::write(path, content)
        .await
        .map_err(|e| Error::Storage(format!("Failed to write {}: {}", label, e)))?;
    set_file_permissions_owner_only(path).await
}

#[cfg(unix)]
async fn set_file_permissions_owner_only(path: &Path) -> Result<()> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    tokio::fs::set_permissions(path, Permissions::from_mode(CREDENTIAL_FILE_MODE))
        .await
        .map_err(|e| Error::Storage(format!("Failed to secure file permissions: {}", e)))
}

#[cfg(not(unix))]
async fn set_file_permissions_owner_only(_path: &Path) -> Result<()> {
    Ok(())
}

/// Encrypt data using AES-256-GCM.
fn encrypt_data(plaintext: &[u8], key: &[u8], rng: &SystemRandom) -> Result<Vec<u8>> {
    let unbound_key = UnboundKey::new(&AES_256_GCM, key)
        .map_err(|_| Error::Storage("Failed to create encryption key".to_string()))?;
    let key = LessSafeKey::new(unbound_key);

    // Generate random nonce
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut nonce_bytes)
        .map_err(|_| Error::Storage("Failed to generate nonce".to_string()))?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    // Encrypt
    let mut ciphertext = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, Aad::empty(), &mut ciphertext)
        .map_err(|_| Error::Storage("Encryption failed".to_string()))?;

    // Prepend nonce to ciphertext
    let mut result = nonce_bytes.to_vec();
    result.extend(ciphertext);
    Ok(result)
}

/// Decrypt data using AES-256-GCM.
fn decrypt_data(ciphertext: &[u8], key: &[u8]) -> std::result::Result<Vec<u8>, ()> {
    if ciphertext.len() < NONCE_LEN {
        return Err(());
    }

    let (nonce_bytes, encrypted) = ciphertext.split_at(NONCE_LEN);
    let nonce_array: [u8; NONCE_LEN] = nonce_bytes.try_into().map_err(|_| ())?;
    let nonce = Nonce::assume_unique_for_key(nonce_array);

    let unbound_key = UnboundKey::new(&AES_256_GCM, key).map_err(|_| ())?;
    let key = LessSafeKey::new(unbound_key);

    let mut data = encrypted.to_vec();
    let plaintext = key
        .open_in_place(nonce, Aad::empty(), &mut data)
        .map_err(|_| ())?;

    Ok(plaintext.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_credential_roundtrip() {
        let mut store = CredentialStore::default();
        store
            .set("test-service", Some("api_key"), "secret123")
            .await
            .unwrap();

        let value = store.get("test-service").unwrap();
        assert_eq!(value, Some("secret123".to_string()));
    }

    #[test]
    fn test_mask_value() {
        assert_eq!(CredentialStore::mask_value("ab"), "**");
        assert_eq!(CredentialStore::mask_value("abcd"), "****");
        assert_eq!(CredentialStore::mask_value("abcde"), "ab...de");
        assert_eq!(CredentialStore::mask_value("secret123"), "se...23");
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let rng = SystemRandom::new();
        let key = [0u8; 32]; // Test key
        let plaintext = b"Hello, World!";

        let encrypted = encrypt_data(plaintext, &key, &rng).unwrap();
        let decrypted = decrypt_data(&encrypted, &key).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn test_encrypted_credential_roundtrip() {
        let mut store = CredentialStore {
            master_key: Some(SecureMasterKey::new(vec![0u8; 32])),
            ..CredentialStore::default()
        };

        store
            .set("encrypted-service", None, "encrypted_secret")
            .await
            .unwrap();

        // Verify it's encrypted
        let cred = store.credentials.get("encrypted-service").unwrap();
        assert!(cred.value.starts_with("enc:"));

        // Verify we can decrypt
        let value = store.get("encrypted-service").unwrap();
        assert_eq!(value, Some("encrypted_secret".to_string()));
    }

    #[tokio::test]
    async fn test_migration() {
        let mut store = CredentialStore::default();

        // Create a legacy credential (no master key)
        store
            .set("legacy-service", None, "legacy_secret")
            .await
            .unwrap();

        // Verify it's NOT encrypted
        let cred = store.credentials.get("legacy-service").unwrap();
        assert!(!cred.value.starts_with("enc:"));

        // Set master key and migrate
        store.master_key = Some(SecureMasterKey::new(vec![0u8; 32]));
        let migrated = store.migrate_to_encrypted().await.unwrap();
        assert_eq!(migrated, 1);

        // Verify it's now encrypted
        let cred = store.credentials.get("legacy-service").unwrap();
        assert!(cred.value.starts_with("enc:"));

        // Verify we can still decrypt
        let value = store.get("legacy-service").unwrap();
        assert_eq!(value, Some("legacy_secret".to_string()));
    }
}
