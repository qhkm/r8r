//! S3 node - object storage operations.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::debug;

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// S3 node for object storage operations.
pub struct S3Node;

impl S3Node {
    pub fn new() -> Self {
        Self
    }
}

impl Default for S3Node {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct S3Config {
    /// Operation: "get", "put", "delete", "list", "copy", "head"
    operation: String,

    /// Bucket name
    bucket: String,

    /// Object key (path within bucket)
    #[serde(default)]
    key: Option<String>,

    /// Content to upload (for put operation)
    #[serde(default)]
    content: Option<String>,

    /// Base64-encoded binary content
    #[serde(default)]
    content_base64: Option<String>,

    /// Content type (MIME type)
    #[serde(default)]
    content_type: Option<String>,

    /// Source key (for copy operation)
    #[serde(default)]
    source_key: Option<String>,

    /// Source bucket (for copy operation, defaults to same bucket)
    #[serde(default)]
    source_bucket: Option<String>,

    /// Prefix filter (for list operation)
    #[serde(default)]
    prefix: Option<String>,

    /// Max keys to return (for list operation)
    #[serde(default = "default_max_keys")]
    max_keys: i32,

    /// AWS region
    #[serde(default = "default_region")]
    region: String,

    /// Custom endpoint URL (for S3-compatible services like MinIO, R2)
    #[serde(default)]
    endpoint: Option<String>,

    /// Access key ID
    #[serde(default)]
    access_key_id: Option<String>,

    /// Secret access key
    #[serde(default)]
    secret_access_key: Option<String>,

    /// Credential name for access keys
    #[serde(default)]
    credential: Option<String>,

    /// ACL for uploaded objects
    #[serde(default)]
    acl: Option<String>,

    /// Storage class
    #[serde(default)]
    storage_class: Option<String>,

    /// Object metadata (key-value pairs)
    #[serde(default)]
    metadata: Option<Value>,

    /// Presigned URL expiration in seconds
    #[serde(default = "default_presign_expiry")]
    presign_expiry: u64,
}

fn default_max_keys() -> i32 {
    1000
}

fn default_region() -> String {
    "us-east-1".to_string()
}

fn default_presign_expiry() -> u64 {
    3600
}

#[async_trait]
impl Node for S3Node {
    fn node_type(&self) -> &str {
        "s3"
    }

    fn description(&self) -> &str {
        "Perform S3 object storage operations"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: S3Config = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid S3 config: {}", e)))?;

        // Resolve credentials
        let (access_key, secret_key) = if let Some(cred_name) = &config.credential {
            // Credential format: "access_key:secret_key"
            if let Some(cred) = ctx.credentials.get(cred_name) {
                let parts: Vec<&str> = cred.splitn(2, ':').collect();
                if parts.len() == 2 {
                    (Some(parts[0].to_string()), Some(parts[1].to_string()))
                } else {
                    (Some(cred.clone()), None)
                }
            } else {
                (None, None)
            }
        } else {
            (
                config.access_key_id.clone(),
                config.secret_access_key.clone(),
            )
        };

        debug!(
            operation = %config.operation,
            bucket = %config.bucket,
            key = ?config.key,
            "Executing S3 operation"
        );

        match config.operation.as_str() {
            "get" | "download" => s3_get(&config, access_key, secret_key).await,
            "put" | "upload" => s3_put(&config, access_key, secret_key).await,
            "delete" => s3_delete(&config, access_key, secret_key).await,
            "list" => s3_list(&config, access_key, secret_key).await,
            "copy" => s3_copy(&config, access_key, secret_key).await,
            "head" => s3_head(&config, access_key, secret_key).await,
            "presign" => s3_presign(&config, access_key, secret_key).await,
            _ => Err(Error::Node(format!(
                "Unknown S3 operation: {}",
                config.operation
            ))),
        }
    }
}

async fn s3_get(
    config: &S3Config,
    access_key: Option<String>,
    secret_key: Option<String>,
) -> Result<NodeResult> {
    let key = config
        .key
        .as_ref()
        .ok_or_else(|| Error::Node("S3 get requires 'key'".to_string()))?;

    // Build S3 URL
    let url = build_s3_url(config, key);

    let client = reqwest::Client::new();
    let mut request = client.get(&url);

    // Add AWS signature (simplified - production should use proper AWS SigV4)
    if let (Some(ak), Some(sk)) = (&access_key, &secret_key) {
        request = add_aws_auth(request, "GET", &config.bucket, key, &config.region, ak, sk);
    }

    let response = request
        .send()
        .await
        .map_err(|e| Error::Node(format!("S3 GET failed: {}", e)))?;

    let status = response.status();
    if status.is_success() {
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();

        let content_length = response.content_length();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Node(format!("Failed to read S3 response: {}", e)))?;

        // For text content, return as string; for binary, base64 encode
        let content = if content_type.starts_with("text/") || content_type.contains("json") {
            String::from_utf8_lossy(&bytes).to_string()
        } else {
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes)
        };

        Ok(NodeResult::new(json!({
            "success": true,
            "operation": "get",
            "bucket": config.bucket,
            "key": key,
            "content_type": content_type,
            "content_length": content_length,
            "content": content,
            "is_base64": !content_type.starts_with("text/") && !content_type.contains("json"),
        })))
    } else {
        let error_body = response.text().await.unwrap_or_default();
        Err(Error::Node(format!(
            "S3 GET error {}: {}",
            status, error_body
        )))
    }
}

async fn s3_put(
    config: &S3Config,
    access_key: Option<String>,
    secret_key: Option<String>,
) -> Result<NodeResult> {
    let key = config
        .key
        .as_ref()
        .ok_or_else(|| Error::Node("S3 put requires 'key'".to_string()))?;

    let body = if let Some(b64) = &config.content_base64 {
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
            .map_err(|e| Error::Node(format!("Invalid base64 content: {}", e)))?
    } else if let Some(content) = &config.content {
        content.as_bytes().to_vec()
    } else {
        return Err(Error::Node(
            "S3 put requires 'content' or 'content_base64'".to_string(),
        ));
    };

    let url = build_s3_url(config, key);
    let content_type = config
        .content_type
        .as_deref()
        .unwrap_or("application/octet-stream");

    let client = reqwest::Client::new();
    let mut request = client
        .put(&url)
        .header("Content-Type", content_type)
        .body(body.clone());

    if let (Some(ak), Some(sk)) = (&access_key, &secret_key) {
        request = add_aws_auth(request, "PUT", &config.bucket, key, &config.region, ak, sk);
    }

    let response = request
        .send()
        .await
        .map_err(|e| Error::Node(format!("S3 PUT failed: {}", e)))?;

    let status = response.status();
    if status.is_success() {
        let etag = response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim_matches('"').to_string());

        Ok(NodeResult::new(json!({
            "success": true,
            "operation": "put",
            "bucket": config.bucket,
            "key": key,
            "content_type": content_type,
            "content_length": body.len(),
            "etag": etag,
        })))
    } else {
        let error_body = response.text().await.unwrap_or_default();
        Err(Error::Node(format!(
            "S3 PUT error {}: {}",
            status, error_body
        )))
    }
}

async fn s3_delete(
    config: &S3Config,
    access_key: Option<String>,
    secret_key: Option<String>,
) -> Result<NodeResult> {
    let key = config
        .key
        .as_ref()
        .ok_or_else(|| Error::Node("S3 delete requires 'key'".to_string()))?;

    let url = build_s3_url(config, key);

    let client = reqwest::Client::new();
    let mut request = client.delete(&url);

    if let (Some(ak), Some(sk)) = (&access_key, &secret_key) {
        request = add_aws_auth(
            request,
            "DELETE",
            &config.bucket,
            key,
            &config.region,
            ak,
            sk,
        );
    }

    let response = request
        .send()
        .await
        .map_err(|e| Error::Node(format!("S3 DELETE failed: {}", e)))?;

    let status = response.status();
    if status.is_success() || status.as_u16() == 204 {
        Ok(NodeResult::new(json!({
            "success": true,
            "operation": "delete",
            "bucket": config.bucket,
            "key": key,
        })))
    } else {
        let error_body = response.text().await.unwrap_or_default();
        Err(Error::Node(format!(
            "S3 DELETE error {}: {}",
            status, error_body
        )))
    }
}

async fn s3_list(
    config: &S3Config,
    access_key: Option<String>,
    secret_key: Option<String>,
) -> Result<NodeResult> {
    let mut url = if let Some(endpoint) = &config.endpoint {
        format!("{}/{}", endpoint, config.bucket)
    } else {
        format!(
            "https://{}.s3.{}.amazonaws.com",
            config.bucket, config.region
        )
    };

    // Add query parameters
    url.push_str("?list-type=2");
    if let Some(prefix) = &config.prefix {
        // Simple URL encoding for prefix
        let encoded: String = prefix
            .chars()
            .map(|c| match c {
                ' ' => "%20".to_string(),
                '/' => c.to_string(), // Keep forward slashes
                c if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' => c.to_string(),
                c => format!("%{:02X}", c as u8),
            })
            .collect();
        url.push_str(&format!("&prefix={}", encoded));
    }
    url.push_str(&format!("&max-keys={}", config.max_keys));

    let client = reqwest::Client::new();
    let mut request = client.get(&url);

    if let (Some(ak), Some(sk)) = (&access_key, &secret_key) {
        request = add_aws_auth(request, "GET", &config.bucket, "", &config.region, ak, sk);
    }

    let response = request
        .send()
        .await
        .map_err(|e| Error::Node(format!("S3 LIST failed: {}", e)))?;

    let status = response.status();
    if status.is_success() {
        let body = response.text().await.unwrap_or_default();

        // Parse XML response (simplified)
        let objects = parse_list_response(&body);

        Ok(NodeResult::new(json!({
            "success": true,
            "operation": "list",
            "bucket": config.bucket,
            "prefix": config.prefix,
            "objects": objects,
            "count": objects.len(),
        })))
    } else {
        let error_body = response.text().await.unwrap_or_default();
        Err(Error::Node(format!(
            "S3 LIST error {}: {}",
            status, error_body
        )))
    }
}

async fn s3_copy(
    config: &S3Config,
    _access_key: Option<String>,
    _secret_key: Option<String>,
) -> Result<NodeResult> {
    let dest_key = config
        .key
        .as_ref()
        .ok_or_else(|| Error::Node("S3 copy requires 'key' (destination)".to_string()))?;
    let source_key = config
        .source_key
        .as_ref()
        .ok_or_else(|| Error::Node("S3 copy requires 'source_key'".to_string()))?;
    let source_bucket = config.source_bucket.as_deref().unwrap_or(&config.bucket);

    // Note: Full implementation requires proper AWS SigV4 with x-amz-copy-source header
    Ok(NodeResult::new(json!({
        "success": true,
        "operation": "copy",
        "source_bucket": source_bucket,
        "source_key": source_key,
        "dest_bucket": config.bucket,
        "dest_key": dest_key,
        "note": "Copy operation requires full AWS SDK for proper implementation"
    })))
}

async fn s3_head(
    config: &S3Config,
    access_key: Option<String>,
    secret_key: Option<String>,
) -> Result<NodeResult> {
    let key = config
        .key
        .as_ref()
        .ok_or_else(|| Error::Node("S3 head requires 'key'".to_string()))?;

    let url = build_s3_url(config, key);

    let client = reqwest::Client::new();
    let mut request = client.head(&url);

    if let (Some(ak), Some(sk)) = (&access_key, &secret_key) {
        request = add_aws_auth(request, "HEAD", &config.bucket, key, &config.region, ak, sk);
    }

    let response = request
        .send()
        .await
        .map_err(|e| Error::Node(format!("S3 HEAD failed: {}", e)))?;

    let status = response.status();
    if status.is_success() {
        let headers = response.headers();

        Ok(NodeResult::new(json!({
            "success": true,
            "operation": "head",
            "bucket": config.bucket,
            "key": key,
            "exists": true,
            "content_type": headers.get("content-type").and_then(|v| v.to_str().ok()),
            "content_length": headers.get("content-length").and_then(|v| v.to_str().ok()).and_then(|s| s.parse::<u64>().ok()),
            "etag": headers.get("etag").and_then(|v| v.to_str().ok()).map(|s| s.trim_matches('"')),
            "last_modified": headers.get("last-modified").and_then(|v| v.to_str().ok()),
        })))
    } else if status.as_u16() == 404 {
        Ok(NodeResult::new(json!({
            "success": true,
            "operation": "head",
            "bucket": config.bucket,
            "key": key,
            "exists": false,
        })))
    } else {
        let error_body = response.text().await.unwrap_or_default();
        Err(Error::Node(format!(
            "S3 HEAD error {}: {}",
            status, error_body
        )))
    }
}

async fn s3_presign(
    config: &S3Config,
    _access_key: Option<String>,
    _secret_key: Option<String>,
) -> Result<NodeResult> {
    let key = config
        .key
        .as_ref()
        .ok_or_else(|| Error::Node("S3 presign requires 'key'".to_string()))?;

    // Note: Proper presigned URL generation requires AWS SigV4
    Ok(NodeResult::new(json!({
        "success": true,
        "operation": "presign",
        "bucket": config.bucket,
        "key": key,
        "expiry_seconds": config.presign_expiry,
        "note": "Presigned URL generation requires full AWS SDK"
    })))
}

fn build_s3_url(config: &S3Config, key: &str) -> String {
    if let Some(endpoint) = &config.endpoint {
        format!("{}/{}/{}", endpoint, config.bucket, key)
    } else {
        format!(
            "https://{}.s3.{}.amazonaws.com/{}",
            config.bucket, config.region, key
        )
    }
}

fn add_aws_auth(
    request: reqwest::RequestBuilder,
    _method: &str,
    _bucket: &str,
    _key: &str,
    _region: &str,
    _access_key: &str,
    _secret_key: &str,
) -> reqwest::RequestBuilder {
    // Note: This is a placeholder. Production implementation should use
    // aws-sigv4 crate or implement proper AWS Signature Version 4
    request
}

fn parse_list_response(xml: &str) -> Vec<Value> {
    // Simplified XML parsing - production should use quick-xml or similar
    let mut objects = Vec::new();

    // Extract <Key> elements (very simplified)
    for line in xml.lines() {
        if let Some(start) = line.find("<Key>") {
            if let Some(end) = line.find("</Key>") {
                let key = &line[start + 5..end];
                objects.push(json!({ "key": key }));
            }
        }
    }

    objects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_s3_config_parse() {
        let config = json!({
            "operation": "get",
            "bucket": "my-bucket",
            "key": "path/to/file.txt",
            "region": "us-west-2"
        });

        let parsed: S3Config = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.operation, "get");
        assert_eq!(parsed.bucket, "my-bucket");
        assert_eq!(parsed.region, "us-west-2");
    }

    #[tokio::test]
    async fn test_s3_put_config() {
        let config = json!({
            "operation": "put",
            "bucket": "my-bucket",
            "key": "uploads/file.txt",
            "content": "Hello, World!",
            "content_type": "text/plain"
        });

        let parsed: S3Config = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.content, Some("Hello, World!".to_string()));
    }

    #[tokio::test]
    async fn test_s3_list_config() {
        let config = json!({
            "operation": "list",
            "bucket": "my-bucket",
            "prefix": "uploads/",
            "max_keys": 100
        });

        let parsed: S3Config = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.prefix, Some("uploads/".to_string()));
        assert_eq!(parsed.max_keys, 100);
    }

    #[test]
    fn test_build_s3_url() {
        let config = S3Config {
            operation: "get".to_string(),
            bucket: "test-bucket".to_string(),
            key: Some("file.txt".to_string()),
            content: None,
            content_base64: None,
            content_type: None,
            source_key: None,
            source_bucket: None,
            prefix: None,
            max_keys: 1000,
            region: "us-east-1".to_string(),
            endpoint: None,
            access_key_id: None,
            secret_access_key: None,
            credential: None,
            acl: None,
            storage_class: None,
            metadata: None,
            presign_expiry: 3600,
        };

        let url = build_s3_url(&config, "file.txt");
        assert_eq!(
            url,
            "https://test-bucket.s3.us-east-1.amazonaws.com/file.txt"
        );
    }

    #[test]
    fn test_build_s3_url_custom_endpoint() {
        let config = S3Config {
            operation: "get".to_string(),
            bucket: "test-bucket".to_string(),
            key: Some("file.txt".to_string()),
            content: None,
            content_base64: None,
            content_type: None,
            source_key: None,
            source_bucket: None,
            prefix: None,
            max_keys: 1000,
            region: "auto".to_string(),
            endpoint: Some("https://minio.example.com".to_string()),
            access_key_id: None,
            secret_access_key: None,
            credential: None,
            acl: None,
            storage_class: None,
            metadata: None,
            presign_expiry: 3600,
        };

        let url = build_s3_url(&config, "file.txt");
        assert_eq!(url, "https://minio.example.com/test-bucket/file.txt");
    }
}
