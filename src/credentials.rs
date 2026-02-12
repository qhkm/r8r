//! Credential storage for r8r.
//!
//! Stores credentials in ~/.r8r/credentials.json with values base64-encoded.
//! For true encryption, consider using the `ring` crate with a master key.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{Error, Result};

/// A stored credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub service: String,
    pub key: Option<String>,
    /// Base64-encoded value
    pub value: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Credential store backed by a JSON file.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CredentialStore {
    credentials: HashMap<String, Credential>,
}

impl CredentialStore {
    /// Get the path to the credentials file.
    pub fn path() -> PathBuf {
        Config::data_dir().join("credentials.json")
    }

    /// Load credentials from disk.
    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| Error::Storage(format!("Failed to read credentials: {}", e)))?;

        serde_json::from_str(&content)
            .map_err(|e| Error::Storage(format!("Failed to parse credentials: {}", e)))
    }

    /// Save credentials to disk.
    pub fn save(&self) -> Result<()> {
        let path = Self::path();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Storage(format!("Failed to create directory: {}", e)))?;
        }

        let content = serde_json::to_string_pretty(&self)
            .map_err(|e| Error::Storage(format!("Failed to serialize credentials: {}", e)))?;

        std::fs::write(&path, content)
            .map_err(|e| Error::Storage(format!("Failed to write credentials: {}", e)))
    }

    /// Set a credential.
    pub fn set(&mut self, service: &str, key: Option<&str>, value: &str) -> Result<()> {
        use base64::{engine::general_purpose::STANDARD, Engine};

        let now = chrono::Utc::now();
        let encoded_value = STANDARD.encode(value.as_bytes());

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
        self.save()
    }

    /// Get a credential value (decoded).
    pub fn get(&self, service: &str) -> Result<Option<String>> {
        use base64::{engine::general_purpose::STANDARD, Engine};

        match self.credentials.get(service) {
            Some(cred) => {
                let decoded = STANDARD
                    .decode(&cred.value)
                    .map_err(|e| Error::Storage(format!("Failed to decode credential: {}", e)))?;

                String::from_utf8(decoded)
                    .map(Some)
                    .map_err(|e| Error::Storage(format!("Invalid UTF-8 in credential: {}", e)))
            }
            None => Ok(None),
        }
    }

    /// List all credentials (without decoding values).
    pub fn list(&self) -> Vec<&Credential> {
        self.credentials.values().collect()
    }

    /// Delete a credential.
    pub fn delete(&mut self, service: &str) -> Result<bool> {
        let existed = self.credentials.remove(service).is_some();
        if existed {
            self.save()?;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_roundtrip() {
        let mut store = CredentialStore::default();
        store
            .set("test-service", Some("api_key"), "secret123")
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
}
