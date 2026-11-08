/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Credential storage for r8r.
//!
//! Credentials live in `<data dir>/credentials.json`. Every value, and every
//! OAuth2 token, is encrypted with AES-256-GCM before it is written.
//!
//! The 256-bit key comes from `R8R_ENCRYPTION_KEY` when it is set (64 hex
//! characters are used as the key directly; any other string is hashed with
//! SHA-256). Otherwise r8r generates a random key on first use and keeps it in
//! `<data dir>/encryption.key`, readable only by the owner.
//!
//! With the default key file, encryption protects a copied or backed-up
//! `credentials.json` on its own, not someone who can read the whole data
//! directory. Set `R8R_ENCRYPTION_KEY` (for example from a secrets manager) to
//! keep the key off the disk entirely.
//!
//! Older stores kept base64-encoded values and plaintext OAuth2 tokens. They are
//! encrypted automatically the first time the store is loaded.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::{Engine, engine::general_purpose::STANDARD};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::integrations::oauth2::OAuth2Credential;

/// Environment variable holding the encryption key.
pub const ENCRYPTION_KEY_ENV: &str = "R8R_ENCRYPTION_KEY";

/// Name of the generated key file, next to `credentials.json`.
const KEY_FILE_NAME: &str = "encryption.key";

/// Prefix marking an encrypted value: `enc:base64(nonce + ciphertext)`.
const ENC_PREFIX: &str = "enc:";

/// AES-256 key length in bytes.
const KEY_LEN: usize = 32;

/// Nonce length for AES-GCM.
const NONCE_LEN: usize = 12;

/// File mode for credential files on Unix systems (owner read/write only).
const CREDENTIAL_FILE_MODE: u32 = 0o600;

/// Secure container for the encryption key that zeroizes on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
struct SecureMasterKey(Vec<u8>);

impl SecureMasterKey {
    fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// A stored credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub service: String,
    pub key: Option<String>,
    /// `enc:base64(nonce + ciphertext)`. Stores written by r8r 0.8.3 and
    /// earlier held plain base64; those are re-encrypted on load.
    pub value: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// On-disk layout of `credentials.json`.
#[derive(Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    credentials: HashMap<String, Credential>,
    /// Plaintext OAuth2 tokens from older stores. Read only; never written.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    oauth2_credentials: HashMap<String, OAuth2Credential>,
    /// OAuth2 tokens as one encrypted JSON blob (`enc:...`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    oauth2_encrypted: Option<String>,
}

/// Credential store backed by a JSON file.
#[derive(Default)]
pub struct CredentialStore {
    credentials: HashMap<String, Credential>,
    oauth2_credentials: HashMap<String, OAuth2Credential>,
    /// Encrypted OAuth2 blob that could not be opened because no key was
    /// available. Written back unchanged so it is not lost.
    oauth2_locked: Option<String>,
    /// Encryption key, once resolved (zeroized on drop).
    master_key: Option<SecureMasterKey>,
    /// Value of `R8R_ENCRYPTION_KEY` captured at load time.
    key_env: Option<String>,
    /// Where the store is persisted; `None` means the default data directory.
    path_override: Option<PathBuf>,
}

impl std::fmt::Debug for CredentialStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialStore")
            .field("credentials", &self.credentials)
            .field("oauth2_credentials", &"[REDACTED]")
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

    /// Get the path to the generated encryption key file.
    pub fn key_path() -> PathBuf {
        Config::data_dir().join(KEY_FILE_NAME)
    }

    /// Resolve the path this store writes to.
    fn effective_path(&self) -> PathBuf {
        self.path_override.clone().unwrap_or_else(Self::path)
    }

    fn effective_key_path(&self) -> PathBuf {
        self.effective_path().with_file_name(KEY_FILE_NAME)
    }

    /// Load the store, resolve the encryption key, and encrypt any values left
    /// unencrypted by older versions.
    pub async fn load() -> Result<Self> {
        Self::load_at(Self::path(), std::env::var(ENCRYPTION_KEY_ENV).ok()).await
    }

    async fn load_at(path: PathBuf, key_env: Option<String>) -> Result<Self> {
        let file: StoreFile = if tokio::fs::try_exists(&path)
            .await
            .map_err(|e| Error::Storage(format!("Failed to check credentials path: {}", e)))?
        {
            let content = read_file(&path, "credentials").await?;
            serde_json::from_str(&content)
                .map_err(|e| Error::Storage(format!("Failed to parse credentials: {}", e)))?
        } else {
            StoreFile::default()
        };

        let legacy_oauth2 = !file.oauth2_credentials.is_empty();
        let mut store = Self {
            credentials: file.credentials,
            oauth2_credentials: file.oauth2_credentials,
            key_env: key_env.filter(|k| !k.is_empty()),
            path_override: Some(path),
            ..Self::default()
        };
        store.master_key =
            resolve_key(&store.effective_key_path(), store.key_env.as_deref(), false)?;

        if let Some(blob) = file.oauth2_encrypted {
            match &store.master_key {
                Some(key) => {
                    let json = decrypt_value(&blob, key).map_err(|_| {
                        Error::Storage(format!(
                            "Cannot decrypt stored OAuth2 tokens: {}",
                            store.wrong_key_hint()
                        ))
                    })?;
                    let tokens: HashMap<String, OAuth2Credential> = serde_json::from_str(&json)
                        .map_err(|e| {
                            Error::Storage(format!("Failed to parse OAuth2 tokens: {}", e))
                        })?;
                    // Plaintext entries from an older store must not override
                    // newer encrypted ones.
                    for (name, token) in tokens {
                        store.oauth2_credentials.insert(name, token);
                    }
                }
                None => store.oauth2_locked = Some(blob),
            }
        }

        let has_plaintext = legacy_oauth2
            || store
                .credentials
                .values()
                .any(|c| !c.value.starts_with(ENC_PREFIX));
        if has_plaintext {
            store.save().await?;
        }

        Ok(store)
    }

    /// Make sure a key is available, generating the key file if needed.
    fn ensure_key(&mut self) -> Result<SecureMasterKey> {
        if self.master_key.is_none() {
            // Never start a second key for a store that already holds values
            // encrypted with another one: that would split it across two keys.
            let has_encrypted = self.oauth2_locked.is_some()
                || self
                    .credentials
                    .values()
                    .any(|c| c.value.starts_with(ENC_PREFIX));
            if has_encrypted {
                return Err(Error::Storage(format!(
                    "Stored credentials are encrypted, but {}",
                    self.missing_key_hint()
                )));
            }
            self.master_key =
                resolve_key(&self.effective_key_path(), self.key_env.as_deref(), true)?;
        }
        let key = self
            .master_key
            .clone()
            .ok_or_else(|| Error::Storage("No encryption key available".to_string()))?;

        // Likewise refuse a key that cannot open what is already stored.
        if let Some(existing) = self
            .credentials
            .values()
            .find(|c| c.value.starts_with(ENC_PREFIX))
            && decrypt_value(&existing.value, &key).is_err()
        {
            return Err(Error::Storage(format!(
                "Refusing to write credentials: {}",
                self.wrong_key_hint()
            )));
        }
        Ok(key)
    }

    fn missing_key_hint(&self) -> String {
        format!(
            "no encryption key is available. Set {} or restore {}",
            ENCRYPTION_KEY_ENV,
            self.effective_key_path().display()
        )
    }

    fn wrong_key_hint(&self) -> String {
        let source = if self.key_env.is_some() {
            ENCRYPTION_KEY_ENV.to_string()
        } else {
            self.effective_key_path().display().to_string()
        };
        format!("the key from {source} is not the one it was encrypted with")
    }

    /// Save credentials to disk, encrypting anything still in plaintext.
    pub async fn save(&mut self) -> Result<()> {
        let key = self.ensure_key()?;
        let rng = SystemRandom::new();

        for cred in self.credentials.values_mut() {
            if !cred.value.starts_with(ENC_PREFIX) {
                let plain = STANDARD.decode(&cred.value).map_err(|e| {
                    Error::Storage(format!(
                        "Failed to decode credential '{}': {}",
                        cred.service, e
                    ))
                })?;
                cred.value = encrypt_value(&plain, &key, &rng)?;
            }
        }

        let oauth2_encrypted = if !self.oauth2_credentials.is_empty() {
            let json = serde_json::to_string(&self.oauth2_credentials)
                .map_err(|e| Error::Storage(format!("Failed to serialize OAuth2 tokens: {}", e)))?;
            Some(encrypt_value(json.as_bytes(), &key, &rng)?)
        } else {
            self.oauth2_locked.clone()
        };

        let file = StoreFile {
            credentials: self.credentials.clone(),
            oauth2_credentials: HashMap::new(),
            oauth2_encrypted,
        };
        let content = serde_json::to_string_pretty(&file)
            .map_err(|e| Error::Storage(format!("Failed to serialize credentials: {}", e)))?;
        write_secure_file(&self.effective_path(), &content, "credentials").await
    }

    /// Set a credential.
    pub async fn set(&mut self, service: &str, key: Option<&str>, value: &str) -> Result<()> {
        let now = chrono::Utc::now();
        let master_key = self.ensure_key()?;
        let encoded_value = encrypt_value(value.as_bytes(), &master_key, &SystemRandom::new())?;

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

    /// Get a credential value (decrypted).
    pub fn get(&self, service: &str) -> Result<Option<String>> {
        let Some(cred) = self.credentials.get(service) else {
            return Ok(None);
        };

        if !cred.value.starts_with(ENC_PREFIX) {
            // Not yet re-encrypted (only possible for a store built in memory).
            let decoded = STANDARD
                .decode(&cred.value)
                .map_err(|e| Error::Storage(format!("Failed to decode credential: {}", e)))?;
            return String::from_utf8(decoded)
                .map(Some)
                .map_err(|e| Error::Storage(format!("Invalid UTF-8 in credential: {}", e)));
        }

        let master_key = self.master_key.as_ref().ok_or_else(|| {
            Error::Storage(format!(
                "Credential '{}' is encrypted, but {}",
                service,
                self.missing_key_hint()
            ))
        })?;
        decrypt_value(&cred.value, master_key)
            .map(Some)
            .map_err(|_| {
                Error::Storage(format!(
                    "Cannot decrypt credential '{}': {}",
                    service,
                    self.wrong_key_hint()
                ))
            })
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
        credential.value.starts_with(ENC_PREFIX)
    }

    pub fn get_oauth2(&self, service: &str) -> Option<&OAuth2Credential> {
        self.oauth2_credentials.get(service)
    }

    pub fn set_oauth2(&mut self, credential: OAuth2Credential) {
        self.oauth2_credentials
            .insert(credential.service.clone(), credential);
    }

    pub fn list_oauth2(&self) -> Vec<&str> {
        self.oauth2_credentials.keys().map(|s| s.as_str()).collect()
    }
}

/// Resolve the encryption key: `R8R_ENCRYPTION_KEY` first, then the key file.
/// With `create`, a missing key file is generated.
fn resolve_key(
    key_path: &Path,
    key_env: Option<&str>,
    create: bool,
) -> Result<Option<SecureMasterKey>> {
    if let Some(env) = key_env {
        let env = env.trim();
        let bytes = match hex::decode(env) {
            Ok(b) if b.len() == KEY_LEN => b,
            _ => ring::digest::digest(&ring::digest::SHA256, env.as_bytes())
                .as_ref()
                .to_vec(),
        };
        return Ok(Some(SecureMasterKey(bytes)));
    }

    match std::fs::read_to_string(key_path) {
        Ok(content) => {
            let bytes = hex::decode(content.trim())
                .ok()
                .filter(|b| b.len() == KEY_LEN)
                .ok_or_else(|| {
                    Error::Storage(format!(
                        "{} is not a valid key (expected {} hex characters)",
                        key_path.display(),
                        KEY_LEN * 2
                    ))
                })?;
            Ok(Some(SecureMasterKey(bytes)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if create {
                create_key_file(key_path).map(Some)
            } else {
                Ok(None)
            }
        }
        Err(e) => Err(Error::Storage(format!(
            "Failed to read {}: {}",
            key_path.display(),
            e
        ))),
    }
}

/// Generate a random key and write it, owner-only, without overwriting an
/// existing file.
fn create_key_file(key_path: &Path) -> Result<SecureMasterKey> {
    use std::io::Write;

    let mut bytes = vec![0u8; KEY_LEN];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| Error::Storage("Failed to generate encryption key".to_string()))?;

    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::Storage(format!("Failed to create directory: {}", e)))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(CREDENTIAL_FILE_MODE);
    }
    match options.open(key_path) {
        Ok(mut f) => {
            let mut hex_key = hex::encode(&bytes);
            let written = writeln!(f, "{hex_key}");
            hex_key.zeroize();
            written.map_err(|e| {
                Error::Storage(format!("Failed to write {}: {}", key_path.display(), e))
            })?;
            Ok(SecureMasterKey(bytes))
        }
        // Another process created it first; use theirs.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            bytes.zeroize();
            resolve_key(key_path, None, false)?
                .ok_or_else(|| Error::Storage("Encryption key file vanished".to_string()))
        }
        Err(e) => Err(Error::Storage(format!(
            "Failed to create {}: {}",
            key_path.display(),
            e
        ))),
    }
}

fn encrypt_value(plaintext: &[u8], key: &SecureMasterKey, rng: &SystemRandom) -> Result<String> {
    let encrypted = encrypt_data(plaintext, key.as_bytes(), rng)?;
    Ok(format!("{ENC_PREFIX}{}", STANDARD.encode(encrypted)))
}

fn decrypt_value(value: &str, key: &SecureMasterKey) -> std::result::Result<String, ()> {
    let encoded = value.strip_prefix(ENC_PREFIX).ok_or(())?;
    let encrypted = STANDARD.decode(encoded).map_err(|_| ())?;
    let plain = decrypt_data(&encrypted, key.as_bytes())?;
    String::from_utf8(plain).map_err(|_| ())
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

    const HEX_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    fn store_at(dir: &Path) -> CredentialStore {
        CredentialStore {
            path_override: Some(dir.join("credentials.json")),
            ..CredentialStore::default()
        }
    }

    fn raw_file(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("credentials.json")).unwrap()
    }

    #[tokio::test]
    async fn set_encrypts_and_creates_owner_only_key_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = store_at(tmp.path());
        store
            .set("test-service", Some("api_key"), "secret123")
            .await
            .unwrap();

        assert_eq!(
            store.get("test-service").unwrap().as_deref(),
            Some("secret123")
        );
        let raw = raw_file(tmp.path());
        assert!(!raw.contains("secret123"));
        assert!(!raw.contains(&STANDARD.encode("secret123")));
        assert!(raw.contains("\"enc:"));

        let key_file = tmp.path().join(KEY_FILE_NAME);
        assert_eq!(std::fs::read_to_string(&key_file).unwrap().trim().len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key_file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn reload_decrypts_with_key_file() {
        let tmp = tempfile::tempdir().unwrap();
        store_at(tmp.path()).set("svc", None, "v1").await.unwrap();

        let store = CredentialStore::load_at(tmp.path().join("credentials.json"), None)
            .await
            .unwrap();
        assert_eq!(store.get("svc").unwrap().as_deref(), Some("v1"));
    }

    #[tokio::test]
    async fn env_key_is_used_and_no_key_file_is_written() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("credentials.json");
        let mut store = CredentialStore::load_at(path.clone(), Some(HEX_KEY.into()))
            .await
            .unwrap();
        store.set("svc", None, "v1").await.unwrap();
        assert!(!tmp.path().join(KEY_FILE_NAME).exists());

        let store = CredentialStore::load_at(path.clone(), Some(HEX_KEY.into()))
            .await
            .unwrap();
        assert_eq!(store.get("svc").unwrap().as_deref(), Some("v1"));

        // A passphrase (not 64 hex chars) also works, hashed to a key.
        let path = tmp.path().join("other.json");
        let mut store = CredentialStore::load_at(path.clone(), Some("correct horse".into()))
            .await
            .unwrap();
        store.set("other", None, "v2").await.unwrap();
        let store = CredentialStore::load_at(path, Some("correct horse".into()))
            .await
            .unwrap();
        assert_eq!(store.get("other").unwrap().as_deref(), Some("v2"));
    }

    #[tokio::test]
    async fn missing_or_wrong_key_gives_clear_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("credentials.json");
        let mut store = CredentialStore::load_at(path.clone(), Some(HEX_KEY.into()))
            .await
            .unwrap();
        store.set("svc", None, "v1").await.unwrap();

        // No env key and no key file: listing works, reading explains why not.
        let store = CredentialStore::load_at(path.clone(), None).await.unwrap();
        assert_eq!(store.list().len(), 1);
        let err = store.get("svc").unwrap_err().to_string();
        assert!(err.contains(ENCRYPTION_KEY_ENV), "{err}");

        let mut store = CredentialStore::load_at(path.clone(), Some("wrong".into()))
            .await
            .unwrap();
        let err = store.get("svc").unwrap_err().to_string();
        assert!(err.contains("not the one it was encrypted with"), "{err}");

        // Writing with the wrong key would split the store across two keys.
        let before = raw_file(tmp.path());
        let err = store.set("new", None, "v2").await.unwrap_err().to_string();
        assert!(err.contains("not the one it was encrypted with"), "{err}");
        assert_eq!(raw_file(tmp.path()), before);
    }

    #[tokio::test]
    async fn legacy_base64_and_plaintext_oauth2_are_encrypted_on_load() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("credentials.json");
        let now = chrono::Utc::now().to_rfc3339();
        let legacy = format!(
            r#"{{
  "credentials": {{
    "old": {{"service": "old", "key": null, "value": "{}", "created_at": "{now}", "updated_at": "{now}"}}
  }},
  "oauth2_credentials": {{
    "gsheets": {{"service": "gsheets", "provider": "google", "access_token": "ya29.TOKEN",
      "refresh_token": "1//REFRESH", "expires_at": null, "token_type": "Bearer",
      "scopes": [], "created_at": "{now}", "updated_at": "{now}"}}
  }}
}}"#,
            STANDARD.encode("legacy_secret")
        );
        std::fs::write(&path, legacy).unwrap();

        let store = CredentialStore::load_at(path.clone(), None).await.unwrap();
        assert_eq!(store.get("old").unwrap().as_deref(), Some("legacy_secret"));
        assert_eq!(
            store.get_oauth2("gsheets").unwrap().access_token,
            "ya29.TOKEN"
        );

        let raw = raw_file(tmp.path());
        assert!(!raw.contains(&STANDARD.encode("legacy_secret")));
        assert!(!raw.contains("ya29.TOKEN") && !raw.contains("1//REFRESH"));
        assert!(raw.contains("oauth2_encrypted"));

        let store = CredentialStore::load_at(path, None).await.unwrap();
        assert_eq!(store.get("old").unwrap().as_deref(), Some("legacy_secret"));
        assert_eq!(
            store
                .get_oauth2("gsheets")
                .unwrap()
                .refresh_token
                .as_deref(),
            Some("1//REFRESH")
        );
    }

    #[tokio::test]
    async fn without_the_key_writes_are_refused_instead_of_starting_a_new_key() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("credentials.json");
        let mut store = CredentialStore::load_at(path.clone(), Some(HEX_KEY.into()))
            .await
            .unwrap();
        store.set("svc", None, "v1").await.unwrap();
        store.set_oauth2(crate::integrations::oauth2::OAuth2Credential {
            service: "gsheets".into(),
            provider: "google".into(),
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: None,
            token_type: "Bearer".into(),
            scopes: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        });
        store.save().await.unwrap();
        let before = raw_file(tmp.path());

        let mut store = CredentialStore::load_at(path.clone(), None).await.unwrap();
        assert!(store.get_oauth2("gsheets").is_none());
        let err = store.set("new", None, "v2").await.unwrap_err().to_string();
        assert!(err.contains(ENCRYPTION_KEY_ENV), "{err}");
        assert!(store.delete("svc").await.is_err());
        assert_eq!(raw_file(tmp.path()), before);
        assert!(!tmp.path().join(KEY_FILE_NAME).exists());

        let store = CredentialStore::load_at(path, Some(HEX_KEY.into()))
            .await
            .unwrap();
        assert_eq!(store.get("svc").unwrap().as_deref(), Some("v1"));
        assert_eq!(store.get_oauth2("gsheets").unwrap().access_token, "tok");
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
        let key = [0u8; 32];
        let encrypted = encrypt_data(b"Hello, World!", &key, &rng).unwrap();
        assert_eq!(decrypt_data(&encrypted, &key).unwrap(), b"Hello, World!");
    }
}
