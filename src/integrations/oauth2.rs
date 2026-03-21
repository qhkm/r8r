/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const REFRESH_WINDOW_MINUTES: i64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Credential {
    pub service: String,
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub token_type: String,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl OAuth2Credential {
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(expires) => {
                let threshold = Utc::now() + chrono::Duration::minutes(REFRESH_WINDOW_MINUTES);
                expires <= threshold
            }
            None => false,
        }
    }

    pub fn can_refresh(&self) -> bool {
        self.refresh_token.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_credential(expires_at: Option<DateTime<Utc>>) -> OAuth2Credential {
        let now = Utc::now();
        OAuth2Credential {
            service: "test-service".to_string(),
            provider: "test-provider".to_string(),
            access_token: "access_token_123".to_string(),
            refresh_token: Some("refresh_token_456".to_string()),
            expires_at,
            token_type: "Bearer".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn test_oauth2_credential_serialization() {
        let cred = make_credential(Some(Utc::now() + Duration::hours(1)));
        let json = serde_json::to_string(&cred).unwrap();
        let deserialized: OAuth2Credential = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.service, cred.service);
        assert_eq!(deserialized.provider, cred.provider);
        assert_eq!(deserialized.access_token, cred.access_token);
        assert_eq!(deserialized.refresh_token, cred.refresh_token);
        assert_eq!(deserialized.expires_at, cred.expires_at);
        assert_eq!(deserialized.token_type, cred.token_type);
        assert_eq!(deserialized.scopes, cred.scopes);
    }

    #[test]
    fn test_is_expired_true() {
        // Token expired 1 minute ago
        let cred = make_credential(Some(Utc::now() - Duration::minutes(1)));
        assert!(cred.is_expired());
    }

    #[test]
    fn test_is_expired_near_expiry() {
        // Token expires in 3 minutes (within the 5-minute refresh window)
        let cred = make_credential(Some(Utc::now() + Duration::minutes(3)));
        assert!(cred.is_expired());
    }

    #[test]
    fn test_is_expired_false() {
        // Token expires in 1 hour (well outside refresh window)
        let cred = make_credential(Some(Utc::now() + Duration::hours(1)));
        assert!(!cred.is_expired());
    }

    #[test]
    fn test_no_expiry_not_expired() {
        // No expiry set means not expired
        let cred = make_credential(None);
        assert!(!cred.is_expired());
    }
}
