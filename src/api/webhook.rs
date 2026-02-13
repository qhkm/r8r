//! Webhook signature verification.
//!
//! Supports multiple signature verification schemes used by common services:
//!
//! - **GitHub**: HMAC-SHA256, header `X-Hub-Signature-256`
//! - **Stripe**: HMAC-SHA256 with timestamp, header `Stripe-Signature`
//! - **Slack**: HMAC-SHA256 with timestamp, header `X-Slack-Signature`
//! - **Generic**: Custom HMAC signature verification
//!
//! ## Example
//!
//! ```yaml
//! triggers:
//!   - type: webhook
//!     config:
//!       path: /hooks/github
//!       signature:
//!         type: github
//!         secret: ${GITHUB_WEBHOOK_SECRET}
//! ```

use ring::hmac;
use subtle::ConstantTimeEq;

use crate::error::{Error, Result};

/// Webhook signature configuration.
#[derive(Debug, Clone)]
pub struct SignatureConfig {
    /// Signature type/scheme
    pub scheme: SignatureScheme,
    /// Secret key for HMAC
    pub secret: String,
    /// Tolerance for timestamp verification (in seconds)
    pub timestamp_tolerance: u64,
}

impl Default for SignatureConfig {
    fn default() -> Self {
        Self {
            scheme: SignatureScheme::Generic,
            secret: String::new(),
            timestamp_tolerance: 300, // 5 minutes
        }
    }
}

/// Supported signature schemes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureScheme {
    /// GitHub webhook signature (X-Hub-Signature-256)
    GitHub,
    /// Stripe webhook signature (Stripe-Signature)
    Stripe,
    /// Slack webhook signature (X-Slack-Signature)
    Slack,
    /// Generic HMAC-SHA256 (configurable header)
    Generic,
}

impl SignatureScheme {
    /// Get the header name for this scheme.
    pub fn header_name(&self) -> &'static str {
        match self {
            SignatureScheme::GitHub => "x-hub-signature-256",
            SignatureScheme::Stripe => "stripe-signature",
            SignatureScheme::Slack => "x-slack-signature",
            SignatureScheme::Generic => "x-signature",
        }
    }
}

impl std::str::FromStr for SignatureScheme {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "github" => Ok(SignatureScheme::GitHub),
            "stripe" => Ok(SignatureScheme::Stripe),
            "slack" => Ok(SignatureScheme::Slack),
            "generic" | "hmac" => Ok(SignatureScheme::Generic),
            _ => Err(()),
        }
    }
}

/// Verify a webhook signature.
pub fn verify_signature(
    config: &SignatureConfig,
    signature_header: &str,
    body: &[u8],
    timestamp_header: Option<&str>,
) -> Result<()> {
    match config.scheme {
        SignatureScheme::GitHub => verify_github_signature(&config.secret, signature_header, body),
        SignatureScheme::Stripe => verify_stripe_signature(
            &config.secret,
            signature_header,
            body,
            config.timestamp_tolerance,
        ),
        SignatureScheme::Slack => verify_slack_signature(
            &config.secret,
            signature_header,
            body,
            timestamp_header,
            config.timestamp_tolerance,
        ),
        SignatureScheme::Generic => {
            verify_generic_signature(&config.secret, signature_header, body)
        }
    }
}

/// Verify GitHub webhook signature (X-Hub-Signature-256).
///
/// Format: `sha256=<hex-signature>`
fn verify_github_signature(secret: &str, signature_header: &str, body: &[u8]) -> Result<()> {
    // Parse signature: sha256=...
    let expected_signature = signature_header
        .strip_prefix("sha256=")
        .ok_or_else(|| Error::Validation("Invalid GitHub signature format".to_string()))?;

    let expected_bytes = hex::decode(expected_signature)
        .map_err(|_| Error::Validation("Invalid signature hex encoding".to_string()))?;

    // Compute HMAC-SHA256
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let computed = hmac::sign(&key, body);

    // Constant-time comparison
    if computed.as_ref().ct_eq(&expected_bytes).unwrap_u8() != 1 {
        return Err(Error::Validation("Invalid webhook signature".to_string()));
    }

    Ok(())
}

/// Verify Stripe webhook signature (Stripe-Signature).
///
/// Format: `t=<timestamp>,v1=<signature>,v0=<old-signature>`
fn verify_stripe_signature(
    secret: &str,
    signature_header: &str,
    body: &[u8],
    tolerance: u64,
) -> Result<()> {
    // Parse signature components
    let mut timestamp: Option<i64> = None;
    let mut signatures: Vec<Vec<u8>> = Vec::new();

    for part in signature_header.split(',') {
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| Error::Validation("Invalid Stripe signature format".to_string()))?;

        match key {
            "t" => {
                timestamp = Some(value.parse().map_err(|_| {
                    Error::Validation("Invalid Stripe timestamp".to_string())
                })?);
            }
            "v1" => {
                let sig = hex::decode(value)
                    .map_err(|_| Error::Validation("Invalid signature hex encoding".to_string()))?;
                signatures.push(sig);
            }
            _ => {} // Ignore other keys (v0, etc.)
        }
    }

    let timestamp =
        timestamp.ok_or_else(|| Error::Validation("Missing Stripe timestamp".to_string()))?;

    // Check timestamp tolerance
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    if (now - timestamp).unsigned_abs() > tolerance {
        return Err(Error::Validation("Webhook timestamp too old".to_string()));
    }

    // Compute expected signature
    // Stripe uses: HMAC-SHA256(secret, timestamp + "." + payload)
    let signed_payload = format!("{}.{}", timestamp, String::from_utf8_lossy(body));
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let computed = hmac::sign(&key, signed_payload.as_bytes());

    // Check against any v1 signature
    for sig in &signatures {
        if computed.as_ref().ct_eq(sig).unwrap_u8() == 1 {
            return Ok(());
        }
    }

    Err(Error::Validation("Invalid webhook signature".to_string()))
}

/// Verify Slack webhook signature (X-Slack-Signature).
///
/// Format: `v0=<signature>`
/// Signed payload: `v0:<timestamp>:<body>`
fn verify_slack_signature(
    secret: &str,
    signature_header: &str,
    body: &[u8],
    timestamp_header: Option<&str>,
    tolerance: u64,
) -> Result<()> {
    // Get timestamp from X-Slack-Request-Timestamp header
    let timestamp_str =
        timestamp_header.ok_or_else(|| Error::Validation("Missing Slack timestamp".to_string()))?;

    let timestamp: i64 = timestamp_str
        .parse()
        .map_err(|_| Error::Validation("Invalid Slack timestamp".to_string()))?;

    // Check timestamp tolerance
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    if (now - timestamp).unsigned_abs() > tolerance {
        return Err(Error::Validation("Webhook timestamp too old".to_string()));
    }

    // Parse signature
    let expected_signature = signature_header
        .strip_prefix("v0=")
        .ok_or_else(|| Error::Validation("Invalid Slack signature format".to_string()))?;

    let expected_bytes = hex::decode(expected_signature)
        .map_err(|_| Error::Validation("Invalid signature hex encoding".to_string()))?;

    // Compute signature
    let signed_payload = format!("v0:{}:{}", timestamp, String::from_utf8_lossy(body));
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let computed = hmac::sign(&key, signed_payload.as_bytes());

    // Constant-time comparison
    if computed.as_ref().ct_eq(&expected_bytes).unwrap_u8() != 1 {
        return Err(Error::Validation("Invalid webhook signature".to_string()));
    }

    Ok(())
}

/// Verify generic HMAC-SHA256 signature.
///
/// Supports formats: `sha256=<hex>` or plain `<hex>`
fn verify_generic_signature(secret: &str, signature_header: &str, body: &[u8]) -> Result<()> {
    // Strip prefix if present
    let signature = signature_header
        .strip_prefix("sha256=")
        .unwrap_or(signature_header);

    let expected_bytes = hex::decode(signature)
        .map_err(|_| Error::Validation("Invalid signature hex encoding".to_string()))?;

    // Compute HMAC-SHA256
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let computed = hmac::sign(&key, body);

    // Constant-time comparison
    if computed.as_ref().ct_eq(&expected_bytes).unwrap_u8() != 1 {
        return Err(Error::Validation("Invalid webhook signature".to_string()));
    }

    Ok(())
}

/// Compute HMAC-SHA256 signature for outgoing webhooks.
pub fn compute_signature(secret: &str, body: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let signature = hmac::sign(&key, body);
    format!("sha256={}", hex::encode(signature.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_github_signature() {
        let secret = "test-secret";
        let body = b"Hello, World!";

        // Compute expected signature
        let signature = compute_signature(secret, body);

        // Verify
        let config = SignatureConfig {
            scheme: SignatureScheme::GitHub,
            secret: secret.to_string(),
            ..Default::default()
        };

        assert!(verify_signature(&config, &signature, body, None).is_ok());
    }

    #[test]
    fn test_github_signature_invalid() {
        let config = SignatureConfig {
            scheme: SignatureScheme::GitHub,
            secret: "test-secret".to_string(),
            ..Default::default()
        };

        let result = verify_signature(
            &config,
            "sha256=0000000000000000000000000000000000000000000000000000000000000000",
            b"Hello, World!",
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_generic_signature() {
        let secret = "my-secret";
        let body = b"test payload";

        let signature = compute_signature(secret, body);

        let config = SignatureConfig {
            scheme: SignatureScheme::Generic,
            secret: secret.to_string(),
            ..Default::default()
        };

        assert!(verify_signature(&config, &signature, body, None).is_ok());
    }

    #[test]
    fn test_generic_signature_without_prefix() {
        let secret = "my-secret";
        let body = b"test payload";

        // Compute signature without prefix
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
        let sig = hmac::sign(&key, body);
        let signature = hex::encode(sig.as_ref());

        let config = SignatureConfig {
            scheme: SignatureScheme::Generic,
            secret: secret.to_string(),
            ..Default::default()
        };

        assert!(verify_signature(&config, &signature, body, None).is_ok());
    }

    #[test]
    fn test_stripe_signature() {
        let secret = "whsec_test_secret";
        let body = b"{\"type\":\"payment_intent.succeeded\"}";

        // Get current timestamp
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Compute signature
        let signed_payload = format!("{}.{}", timestamp, String::from_utf8_lossy(body));
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
        let sig = hmac::sign(&key, signed_payload.as_bytes());
        let signature_hex = hex::encode(sig.as_ref());

        let signature_header = format!("t={},v1={}", timestamp, signature_hex);

        let config = SignatureConfig {
            scheme: SignatureScheme::Stripe,
            secret: secret.to_string(),
            timestamp_tolerance: 300,
        };

        assert!(verify_signature(&config, &signature_header, body, None).is_ok());
    }

    #[test]
    fn test_stripe_signature_old_timestamp() {
        let secret = "whsec_test_secret";
        let body = b"{\"type\":\"payment_intent.succeeded\"}";

        // Use old timestamp (10 minutes ago)
        let old_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 600;

        // Compute signature with old timestamp
        let signed_payload = format!("{}.{}", old_timestamp, String::from_utf8_lossy(body));
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
        let sig = hmac::sign(&key, signed_payload.as_bytes());
        let signature_hex = hex::encode(sig.as_ref());

        let signature_header = format!("t={},v1={}", old_timestamp, signature_hex);

        let config = SignatureConfig {
            scheme: SignatureScheme::Stripe,
            secret: secret.to_string(),
            timestamp_tolerance: 300,
        };

        let result = verify_signature(&config, &signature_header, body, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too old"));
    }

    #[test]
    fn test_slack_signature() {
        let secret = "xoxb-slack-secret";
        let body = b"token=test&event=message";

        // Get current timestamp
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();

        // Compute signature
        let signed_payload = format!("v0:{}:{}", timestamp, String::from_utf8_lossy(body));
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
        let sig = hmac::sign(&key, signed_payload.as_bytes());
        let signature = format!("v0={}", hex::encode(sig.as_ref()));

        let config = SignatureConfig {
            scheme: SignatureScheme::Slack,
            secret: secret.to_string(),
            timestamp_tolerance: 300,
        };

        assert!(verify_signature(&config, &signature, body, Some(&timestamp)).is_ok());
    }

    #[test]
    fn test_scheme_from_str() {
        use std::str::FromStr;
        
        assert_eq!(SignatureScheme::from_str("github"), Ok(SignatureScheme::GitHub));
        assert_eq!(SignatureScheme::from_str("STRIPE"), Ok(SignatureScheme::Stripe));
        assert_eq!(SignatureScheme::from_str("Slack"), Ok(SignatureScheme::Slack));
        assert_eq!(SignatureScheme::from_str("generic"), Ok(SignatureScheme::Generic));
        assert_eq!(SignatureScheme::from_str("hmac"), Ok(SignatureScheme::Generic));
        assert!(SignatureScheme::from_str("unknown").is_err());
    }

    #[test]
    fn test_header_names() {
        assert_eq!(SignatureScheme::GitHub.header_name(), "x-hub-signature-256");
        assert_eq!(SignatureScheme::Stripe.header_name(), "stripe-signature");
        assert_eq!(SignatureScheme::Slack.header_name(), "x-slack-signature");
        assert_eq!(SignatureScheme::Generic.header_name(), "x-signature");
    }
}
