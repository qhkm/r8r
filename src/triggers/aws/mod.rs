/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! AWS SQS/SNS trigger support (feature `aws`).
//!
//! Implements AWS SigV4 request signing with the stdlib crypto stack already
//! in the dependency tree (`hmac`, `sha2`, `hex`, `base64`,
//! `percent-encoding`) so SQS receive/delete and SNS publish work without the
//! heavy `aws-sdk-*` crates (~200 transitive dependencies). Keeps the binary
//! lean, consistent with r8r's single ~5MB self-hosted binary goal.
//!
//! SQS messages are surfaced on the event bus as `sqs.<queue-name>` events;
//! each message is deleted (acknowledged) only after the event was published
//! successfully. If the process dies before deletion the visibility timeout
//! makes the message become visible again for a retry.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use reqwest::Url;
use serde_json::json;
use sha2::{Digest, Sha256};
use tracing::{debug, warn};

use crate::triggers::event::EventMessage;
use crate::triggers::event_backend::EventBackend;

type HmacSha256 = Hmac<Sha256>;

/// Characters that must be percent-encoded in AWS canonical URIs/forms.
const AWS_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'=')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}')
    .add(b'~');

/// AWS credentials used for SigV4 signing.
#[derive(Debug, Clone, Default)]
pub struct AwsCredentials {
    /// AWS access key id.
    pub access_key_id: String,
    /// AWS secret access key.
    pub secret_access_key: String,
    /// Optional session token (STS / temporary credentials).
    pub session_token: Option<String>,
    /// AWS region, e.g. `ap-southeast-1`.
    pub region: String,
}

impl AwsCredentials {
    /// Create credentials from parts.
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            session_token: None,
            region: region.into(),
        }
    }

    /// Set the optional session token.
    pub fn with_session_token(mut self, token: impl Into<String>) -> Self {
        self.session_token = Some(token.into());
        self
    }
}

// ============================================================================
// SigV4 signing
// ============================================================================

fn hmac_sha256(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Compute the canonical query string: sorted, encoded keys/values.
fn canonical_query(params: &BTreeMap<String, String>) -> String {
    params
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                utf8_percent_encode(k, AWS_ENCODE_SET),
                utf8_percent_encode(v, AWS_ENCODE_SET)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Build the SigV4 `Authorization` header plus the `X-Amz-Date` value for a
/// request. `canonical_query` must already be sorted & encoded, and
/// `payload_hash` must be the lowercase hex SHA-256 of the request body.
fn sign_v4(
    creds: &AwsCredentials,
    service: &str,
    method: &str,
    host: &str,
    canonical_path: &str,
    canonical_query: &str,
    payload_hash: &str,
) -> (String, String) {
    let now = chrono::Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();

    let canonical_headers = format!("host:{}\nx-amz-date:{}\n", host.to_lowercase(), amz_date);
    let signed_headers = "host;x-amz-date";
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method, canonical_path, canonical_query, canonical_headers, signed_headers, payload_hash
    );

    let scope = format!("{}/{}/{}/aws4_request", date_stamp, creds.region, service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz_date,
        scope,
        sha256_hex(canonical_request.as_bytes())
    );

    let k_date = hmac_sha256(
        format!("AWS4{}", creds.secret_access_key).as_bytes(),
        &date_stamp,
    );
    let k_region = hmac_sha256(&k_date, &creds.region);
    let k_service = hmac_sha256(&k_region, service);
    let k_signing = hmac_sha256(&k_service, "aws4_request");
    let signature = hex::encode(hmac_sha256(&k_signing, &string_to_sign));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        creds.access_key_id, scope, signed_headers, signature
    );
    (amz_date, authorization)
}

// ============================================================================
// SQS trigger
// ============================================================================

/// SQS trigger configuration.
#[derive(Debug, Clone)]
pub struct SqsConfig {
    /// Full queue URL, e.g. `https://sqs.ap-southeast-1.amazonaws.com/1234/orders`.
    pub queue_url: String,
    /// AWS region of the queue.
    pub region: String,
    /// Long-poll wait time in seconds (0-20). 20 = cheapest polling.
    pub wait_time_seconds: u32,
    /// Visibility timeout in seconds before an unacknowledged message retries.
    pub visibility_timeout: u32,
    /// Max messages per receive (1-10).
    pub max_messages: u32,
    /// Poll interval when `wait_time_seconds == 0` (short-poll backoff).
    pub poll_interval_ms: u64,
}

impl Default for SqsConfig {
    fn default() -> Self {
        Self {
            queue_url: String::new(),
            region: "ap-southeast-1".to_string(),
            wait_time_seconds: 20,
            visibility_timeout: 30,
            max_messages: 10,
            poll_interval_ms: 5_000,
        }
    }
}

/// A message received from SQS.
#[derive(Debug, Clone)]
pub struct SqsMessage {
    /// SQS message id.
    pub message_id: String,
    /// Receipt handle — must be passed to `DeleteMessage` to acknowledge.
    pub receipt_handle: String,
    /// Raw message body.
    pub body: String,
    /// SQS system attributes (e.g. `SenderId`, `SentTimestamp`, `ApproximateReceiveCount`).
    pub attributes: BTreeMap<String, String>,
}

/// SQS trigger: long-polls the queue, forwards each message to the event bus
/// as `sqs.<queue-name>`, and deletes it only after successful publication.
#[derive(Debug, Clone)]
pub struct SqsTrigger {
    config: SqsConfig,
    creds: AwsCredentials,
}

impl SqsTrigger {
    /// Create a new SQS trigger.
    pub fn new(config: SqsConfig, creds: AwsCredentials) -> Self {
        Self { config, creds }
    }

    /// Spawn a background loop that runs until the task is aborted.
    pub fn spawn(self, backend: Arc<dyn EventBackend>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.run(backend).await;
        })
    }

    async fn run(&self, backend: Arc<dyn EventBackend>) {
        let queue_name = self
            .config
            .queue_url
            .rsplit('/')
            .next()
            .unwrap_or("queue")
            .to_string();
        debug!("SQS trigger started for queue `{}`", queue_name);

        loop {
            match self.receive_messages().await {
                Ok(messages) => {
                    for msg in messages {
                        let event = EventMessage::new(
                            format!("sqs.{queue_name}"),
                            json!({
                                "message_id": msg.message_id,
                                "queue": queue_name,
                                "body": msg.body,
                                "attributes": msg.attributes,
                            }),
                        )
                        .with_source("sqs");
                        match backend.publish(event).await {
                            Ok(()) => {
                                if let Err(e) = self.delete_message(&msg.receipt_handle).await {
                                    warn!("SQS delete failed for {}: {e}", msg.message_id);
                                }
                            }
                            Err(e) => warn!("SQS event publish failed for {}: {e}", msg.message_id),
                        }
                    }
                }
                Err(e) => warn!("SQS receive failed: {e}"),
            }

            if self.config.wait_time_seconds == 0 {
                tokio::time::sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
            }
        }
    }

    /// Receive up to `max_messages` messages (long-poll).
    async fn receive_messages(&self) -> anyhow::Result<Vec<SqsMessage>> {
        let url = Url::parse(&self.config.queue_url)?;
        let host = url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("SQS queue URL missing host"))?
            .to_string();
        let path = if url.path().is_empty() {
            "/"
        } else {
            url.path()
        }
        .to_string();

        let mut params = BTreeMap::new();
        params.insert("Action".into(), "ReceiveMessage".into());
        params.insert("Version".into(), "2012-11-05".into());
        params.insert(
            "WaitTimeSeconds".into(),
            self.config.wait_time_seconds.to_string(),
        );
        params.insert(
            "VisibilityTimeout".into(),
            self.config.visibility_timeout.to_string(),
        );
        params.insert(
            "MaxNumberOfMessages".into(),
            self.config.max_messages.to_string(),
        );
        params.insert("AttributeName".into(), "All".into());
        let query = canonical_query(&params);

        let (amz_date, auth) = sign_v4(
            &self.creds,
            "sqs",
            "GET",
            &host,
            &path,
            &query,
            sha256_hex(b"").as_str(),
        );

        let request_url = format!("https://{host}{path}?{query}");
        let client = reqwest::Client::new();
        let resp = client
            .get(&request_url)
            .header("X-Amz-Date", &amz_date)
            .header("Authorization", &auth)
            .header(
                "X-Amz-Security-Token",
                self.creds.session_token.as_deref().unwrap_or(""),
            )
            .send()
            .await?
            .error_for_status()?;
        let xml = resp.text().await?;
        Ok(parse_receive_response(&xml))
    }

    /// Delete (acknowledge) a processed message.
    async fn delete_message(&self, receipt_handle: &str) -> anyhow::Result<()> {
        let url = Url::parse(&self.config.queue_url)?;
        let host = url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("SQS queue URL missing host"))?
            .to_string();
        let path = if url.path().is_empty() {
            "/"
        } else {
            url.path()
        }
        .to_string();

        let mut params = BTreeMap::new();
        params.insert("Action".into(), "DeleteMessage".into());
        params.insert("Version".into(), "2012-11-05".into());
        params.insert("ReceiptHandle".into(), receipt_handle.to_string());
        let query = canonical_query(&params);

        let (amz_date, auth) = sign_v4(
            &self.creds,
            "sqs",
            "GET",
            &host,
            &path,
            &query,
            sha256_hex(b"").as_str(),
        );

        let request_url = format!("https://{host}{path}?{query}");
        let client = reqwest::Client::new();
        client
            .get(&request_url)
            .header("X-Amz-Date", &amz_date)
            .header("Authorization", &auth)
            .header(
                "X-Amz-Security-Token",
                self.creds.session_token.as_deref().unwrap_or(""),
            )
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

/// Parse an SQS `ReceiveMessage` XML response into messages.
fn parse_receive_response(xml: &str) -> Vec<SqsMessage> {
    let mut messages = Vec::new();
    if xml.trim().is_empty() {
        return messages;
    }

    let mut reader = quick_xml::Reader::from_str(xml);
    let mut buf = Vec::new();
    // State for the current <Message> element.
    let mut current: Option<SqsMessage> = None;
    // Track elements inside a message.
    let mut in_message = false;
    let mut in_attribute = false;
    let mut element_path: Vec<String> = Vec::new();
    let mut attr_name = String::new();
    let mut attr_value = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                element_path.push(name.clone());
                match name.as_str() {
                    "Message" => {
                        in_message = true;
                        current = Some(SqsMessage {
                            message_id: String::new(),
                            receipt_handle: String::new(),
                            body: String::new(),
                            attributes: BTreeMap::new(),
                        });
                    }
                    "Attribute" => in_attribute = true,
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Text(e)) => {
                let text = e.unescape().unwrap_or_default().into_owned();
                if in_attribute {
                    match element_path.last().map(|s| s.as_str()) {
                        Some("Name") => attr_name = text,
                        Some("Value") => attr_value = text,
                        _ => {}
                    }
                } else if in_message {
                    let msg = current.as_mut().expect("in_message implies current");
                    match element_path.last().map(|s| s.as_str()) {
                        Some("MessageId") => msg.message_id = text,
                        Some("ReceiptHandle") => msg.receipt_handle = text,
                        Some("Body") => msg.body = text,
                        _ => {}
                    }
                }
            }
            Ok(quick_xml::events::Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "Attribute" && in_attribute {
                    if !attr_name.is_empty() {
                        if let Some(msg) = current.as_mut() {
                            msg.attributes.insert(attr_name.clone(), attr_value.clone());
                        }
                    }
                    in_attribute = false;
                    attr_name.clear();
                    attr_value.clear();
                }
                if name == "Message" && in_message {
                    if let Some(msg) = current.take() {
                        if !msg.receipt_handle.is_empty() || !msg.message_id.is_empty() {
                            messages.push(msg);
                        }
                    }
                    in_message = false;
                }
                element_path.pop();
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(e) => {
                warn!("SQS XML parse error: {e}");
                break;
            }
            _ => {}
        }
        buf.clear();
    }
    messages
}

// ============================================================================
// SNS publisher (SNS -> SQS subscriptions are handled by AWS; r8r publishes
// events out to SNS so downstream systems/subscriptions can react).
// ============================================================================

/// SNS publisher: publishes r8r event data to an SNS topic.
#[derive(Debug, Clone)]
pub struct SnsPublisher {
    topic_arn: String,
    creds: AwsCredentials,
}

impl SnsPublisher {
    /// Create a publisher for the given topic ARN.
    pub fn new(topic_arn: impl Into<String>, creds: AwsCredentials) -> Self {
        Self {
            topic_arn: topic_arn.into(),
            creds,
        }
    }

    /// Publish a message to the SNS topic.
    pub async fn publish(&self, subject: &str, message: &serde_json::Value) -> anyhow::Result<()> {
        let url = Url::parse("https://sns.ap-southeast-1.amazonaws.com/")?;
        let host = "sns.ap-southeast-1.amazonaws.com".to_string();
        let path = "/".to_string();

        let mut params = BTreeMap::new();
        params.insert("Action".into(), "Publish".into());
        params.insert("Version".into(), "2010-03-31".into());
        params.insert("TopicArn".into(), self.topic_arn.clone());
        params.insert("Subject".into(), subject.to_string());
        params.insert("Message".into(), message.to_string());
        let query = canonical_query(&params);

        let payload_hash = sha256_hex(query.as_bytes());
        let (amz_date, auth) = sign_v4(&self.creds, "sns", "POST", &host, &path, "", &payload_hash);

        let client = reqwest::Client::new();
        client
            .post(url)
            .header("X-Amz-Date", &amz_date)
            .header("Authorization", &auth)
            .header(
                "X-Amz-Security-Token",
                self.creds.session_token.as_deref().unwrap_or(""),
            )
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(query)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sigv4_is_deterministic() {
        let creds = AwsCredentials::new(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "us-east-1",
        );
        let (d1, a1) = sign_v4(
            &creds,
            "sqs",
            "GET",
            "sqs.us-east-1.amazonaws.com",
            "/",
            "Action=ReceiveMessage&Version=2012-11-05",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        );
        let (d2, a2) = sign_v4(
            &creds,
            "sqs",
            "GET",
            "sqs.us-east-1.amazonaws.com",
            "/",
            "Action=ReceiveMessage&Version=2012-11-05",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        );
        assert_eq!(d1, d2);
        assert_eq!(a1, a2);
        // Authorization header shape: Credential=<key>/<scope>, SignedHeaders=..., Signature=64 hex
        let prefix = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/".to_string();
        assert!(a1.starts_with(&prefix), "unexpected auth header: {a1}");
        assert!(a1.contains(", SignedHeaders=host;x-amz-date, Signature="));
        let sig = a1.split("Signature=").nth(1).unwrap();
        assert_eq!(sig.len(), 64, "signature must be 64 hex chars");
    }

    #[test]
    fn sigv4_differs_between_region_and_service() {
        let creds = AwsCredentials::new("AKIDEXAMPLE", "secret", "us-east-1");
        let (_, a_sqs) = sign_v4(
            &creds,
            "sqs",
            "GET",
            "sqs.us-east-1.amazonaws.com",
            "/",
            "",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        );
        let creds2 = AwsCredentials::new("AKIDEXAMPLE", "secret", "ap-southeast-1");
        let (_, a_sns) = sign_v4(
            &creds2,
            "sns",
            "POST",
            "sns.ap-southeast-1.amazonaws.com",
            "/",
            "",
            "abc",
        );
        assert_ne!(a_sqs, a_sns);
    }

    #[test]
    fn canonical_query_sorts_and_encodes() {
        let mut params = BTreeMap::new();
        params.insert("B".to_string(), "2".to_string());
        params.insert("A".to_string(), "hello world".to_string());
        params.insert("ReceiptHandle".to_string(), "a+b/c=d".to_string());
        let q = canonical_query(&params);
        assert!(q.starts_with("A=hello%20world&B=2&"));
        assert!(q.contains("ReceiptHandle=a%2Bb%2Fc%3Dd"));
    }

    #[test]
    fn parses_sqs_receive_response_with_attributes() {
        let xml = r#"<?xml version="1.0"?>
<ReceiveMessageResponse xmlns="http://queue.amazonaws.com/doc/2012-11-05/">
  <ReceiveMessageResult>
    <Message>
      <MessageId>5fea7756-0ea4-451a-a703-a558b933e274</MessageId>
      <ReceiptHandle>MbZj6wDWli+JvwwJaBV+3dcjk2/Y2/1+2h</ReceiptHandle>
      <MD5OfBody>fafb00f5732ab283681e124bf8747ed1</MD5OfBody>
      <Body>{"order_id": 42}</Body>
      <Attribute>
        <Name>SenderId</Name>
        <Value>123456789012</Value>
      </Attribute>
      <Attribute>
        <Name>ApproximateReceiveCount</Name>
        <Value>1</Value>
      </Attribute>
    </Message>
    <Message>
      <MessageId>second-message</MessageId>
      <ReceiptHandle>handle-2</ReceiptHandle>
      <Body>plain text</Body>
    </Message>
  </ReceiveMessageResult>
</ReceiveMessageResponse>"#;

        let messages = parse_receive_response(xml);
        assert_eq!(messages.len(), 2);
        let m1 = &messages[0];
        assert_eq!(m1.message_id, "5fea7756-0ea4-451a-a703-a558b933e274");
        assert_eq!(m1.receipt_handle, "MbZj6wDWli+JvwwJaBV+3dcjk2/Y2/1+2h");
        assert_eq!(m1.body, "{\"order_id\": 42}");
        assert_eq!(
            m1.attributes.get("SenderId").map(|s| s.as_str()),
            Some("123456789012")
        );
        assert_eq!(
            m1.attributes
                .get("ApproximateReceiveCount")
                .map(|s| s.as_str()),
            Some("1")
        );
        assert_eq!(messages[1].message_id, "second-message");
        assert_eq!(messages[1].body, "plain text");
    }

    #[test]
    fn parses_empty_response_as_no_messages() {
        let messages = parse_receive_response("");
        assert!(messages.is_empty());
        let messages = parse_receive_response(
            r#"<ReceiveMessageResponse><ReceiveMessageResult/></ReceiveMessageResponse>"#,
        );
        assert!(messages.is_empty());
    }

    #[test]
    fn sns_publish_creates_encoded_form_body() {
        // No network call — verify body encoding logic via canonical_query.
        let mut params = BTreeMap::new();
        params.insert(
            "TopicArn".to_string(),
            "arn:aws:sns:ap-southeast-1:123:x".to_string(),
        );
        params.insert("Message".to_string(), json!({"a": 1}).to_string());
        let q = canonical_query(&params);
        assert!(q.starts_with("Message=%7B%22a%22%3A1%7D&"));
        assert!(q.contains("TopicArn=arn%3Aaws%3Asns%3Aap-southeast-1%3A123%3Ax"));
    }
}
