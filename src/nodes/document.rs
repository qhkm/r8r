/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Document node - PDF text extraction and vision-based OCR.

use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap};
use serde::Deserialize;
use serde_json::{Value, json};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

const DEFAULT_MAX_BYTES: usize = 10 * 1024 * 1024;
const DEFAULT_MAX_PAGES: usize = 200;
const DEFAULT_MODEL: &str = "gpt-4o-mini";
const DEFAULT_OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const DEFAULT_PROMPT: &str =
    "Transcribe all visible text from the image verbatim, preserving layout order.";
const DEFAULT_MIME: &str = "application/octet-stream";
const HTTP_TIMEOUT_SECS: u64 = 30;
const HTTP_CONNECT_TIMEOUT_SECS: u64 = 10;

/// Extract text from digital PDFs or images sent to a vision model.
pub struct DocumentNode;

impl DocumentNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DocumentNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DocumentMode {
    PdfText,
    Vision,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum VisionOutput {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Provider {
    #[default]
    Openai,
    Custom,
}

#[derive(Debug, Deserialize)]
struct DocumentConfig {
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    data_field: Option<String>,
    #[serde(default)]
    url: Option<String>,
    mode: DocumentMode,
    #[serde(default = "default_max_pages")]
    max_pages: usize,
    #[serde(default = "default_prompt")]
    prompt: String,
    #[serde(default)]
    output: VisionOutput,
    #[serde(default)]
    provider: Provider,
    #[serde(default = "default_model")]
    model: String,
    #[serde(default)]
    api_url: Option<String>,
    #[serde(default)]
    credential: Option<String>,
    #[serde(default = "default_max_bytes")]
    max_bytes: usize,
    #[serde(default = "default_mime")]
    mime: String,
}

fn default_max_pages() -> usize {
    DEFAULT_MAX_PAGES
}

fn default_prompt() -> String {
    DEFAULT_PROMPT.to_string()
}

fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

fn default_max_bytes() -> usize {
    DEFAULT_MAX_BYTES
}

fn default_mime() -> String {
    DEFAULT_MIME.to_string()
}

#[async_trait]
impl Node for DocumentNode {
    fn node_type(&self) -> &str {
        "document"
    }

    fn description(&self) -> &str {
        "Extract text from PDFs or use a vision model for OCR"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: DocumentConfig = serde_json::from_value(config.clone())
            .map_err(|error| Error::Node(format!("Invalid document config: {error}")))?;
        validate_source(&config)?;

        let bytes = load_document_bytes(&config, &ctx.input).await?;
        match config.mode {
            DocumentMode::PdfText => extract_pdf_text(&bytes, config.max_pages),
            DocumentMode::Vision => execute_vision(&config, ctx, &bytes).await,
        }
    }
}

fn validate_source(config: &DocumentConfig) -> Result<()> {
    let source_count = usize::from(config.data.is_some())
        + usize::from(config.data_field.is_some())
        + usize::from(config.url.is_some());
    match source_count {
        1 => Ok(()),
        0 => Err(Error::Node(
            "Document requires exactly one input source: 'data', 'data_field', or 'url'"
                .to_string(),
        )),
        _ => Err(Error::Node(
            "Document accepts exactly one input source; set only one of 'data', 'data_field', or 'url'"
                .to_string(),
        )),
    }
}

async fn load_document_bytes(config: &DocumentConfig, input: &Value) -> Result<Vec<u8>> {
    if let Some(url) = &config.url {
        return download_with_limit(url, config.max_bytes).await;
    }

    let encoded = if let Some(data) = &config.data {
        data.clone()
    } else {
        resolve_data_field(input, config.data_field.as_deref().unwrap_or_default())?
    };
    let bytes = STANDARD.decode(&encoded).map_err(|error| {
        Error::Node(format!(
            "Document input is not valid STANDARD base64: {error}"
        ))
    })?;
    ensure_size(bytes.len(), config.max_bytes)?;
    Ok(bytes)
}

fn resolve_data_field(input: &Value, path: &str) -> Result<String> {
    let parsed_input;
    let mut current = if let Value::String(value) = input {
        parsed_input = serde_json::from_str::<Value>(value).map_err(|error| {
            Error::Node(format!(
                "Cannot resolve data_field '{path}': node input is a string but not valid JSON: {error}"
            ))
        })?;
        &parsed_input
    } else {
        input
    };

    for part in path.split('.') {
        current = match current {
            Value::Object(object) => object.get(part).ok_or_else(|| {
                Error::Node(format!(
                    "Cannot resolve data_field '{path}': field '{part}' was not found"
                ))
            })?,
            Value::Array(array) => {
                let index = part.parse::<usize>().map_err(|_| {
                    Error::Node(format!(
                        "Cannot resolve data_field '{path}': '{part}' is not an array index"
                    ))
                })?;
                array.get(index).ok_or_else(|| {
                    Error::Node(format!(
                        "Cannot resolve data_field '{path}': array index {index} is out of bounds"
                    ))
                })?
            }
            _ => {
                return Err(Error::Node(format!(
                    "Cannot resolve data_field '{path}': value before '{part}' is not an object or array"
                )));
            }
        };
    }

    current.as_str().map(ToOwned::to_owned).ok_or_else(|| {
        Error::Node(format!(
            "Cannot resolve data_field '{path}': resolved value must be a base64 string"
        ))
    })
}

async fn download_with_limit(url: &str, max_bytes: usize) -> Result<Vec<u8>> {
    crate::ssrf::check_ssrf(url).map_err(Error::Node)?;
    let client = crate::ssrf::hardened_client_builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
        .build()
        .map_err(|error| Error::Node(format!("Failed to build HTTP client: {error}")))?;
    let mut response =
        client.get(url).send().await.map_err(|error| {
            Error::Node(format!("Document download failed for '{url}': {error}"))
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Node(format!(
            "Document download returned HTTP {status} for '{url}': {body}"
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(size_error(max_bytes));
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        Error::Node(format!(
            "Document download body failed for '{url}': {error}"
        ))
    })? {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(size_error(max_bytes));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn ensure_size(actual: usize, max_bytes: usize) -> Result<()> {
    if actual > max_bytes {
        Err(size_error(max_bytes))
    } else {
        Ok(())
    }
}

fn size_error(max_bytes: usize) -> Error {
    Error::Node(format!(
        "Document exceeds max_bytes limit of {max_bytes} bytes"
    ))
}

fn extract_pdf_text(bytes: &[u8], max_pages: usize) -> Result<NodeResult> {
    let document = pdf_extract::Document::load_mem(bytes)
        .map_err(|error| Error::Node(format!("Failed to parse PDF document: {error}")))?;
    let page_count = document.get_pages().len().min(max_pages);
    let mut page_texts = Vec::with_capacity(page_count);

    for page_number in 1..=page_count {
        let mut text = String::new();
        let mut output = pdf_extract::PlainTextOutput::new(&mut text);
        pdf_extract::output_doc_page(&document, &mut output, page_number as u32).map_err(
            |error| Error::Node(format!("Failed to extract PDF page {page_number}: {error}")),
        )?;
        page_texts.push(text);
    }

    let text = page_texts.join("\n\n");
    let pages = page_texts
        .into_iter()
        .enumerate()
        .map(|(index, text)| json!({"page": index + 1, "text": text}))
        .collect::<Vec<_>>();
    Ok(NodeResult::new(json!({
        "text": text,
        "pages": pages,
        "page_count": page_count,
        "source": "pdf_text",
    })))
}

async fn execute_vision(
    config: &DocumentConfig,
    ctx: &NodeContext,
    bytes: &[u8],
) -> Result<NodeResult> {
    let api_key = resolve_credential(config, ctx)?;
    let base64_data = STANDARD.encode(bytes);
    let (url, headers, body) =
        build_vision_request(config, &api_key, &config.prompt, &base64_data, &config.mime)?;
    let response = http_client()?
        .post(&url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|error| Error::Node(format!("Vision API request failed: {error}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Node(format!(
            "Vision API returned HTTP {status}: {body}"
        )));
    }
    let response: Value = response
        .json()
        .await
        .map_err(|error| Error::Node(format!("Failed to parse vision API response: {error}")))?;
    let text = response["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| {
            Error::Node("Vision API response is missing choices[0].message.content".to_string())
        })?;
    let usage = response.get("usage").cloned().unwrap_or(Value::Null);

    let data = match config.output {
        VisionOutput::Text => json!({
            "text": text,
            "source": "vision",
            "model": config.model,
            "usage": usage,
        }),
        VisionOutput::Json => json!({
            "text": text,
            "json": extract_json(text),
            "source": "vision",
            "model": config.model,
            "usage": usage,
        }),
    };
    Ok(NodeResult::new(data))
}

fn resolve_credential(config: &DocumentConfig, ctx: &NodeContext) -> Result<String> {
    if let Some(name) = &config.credential {
        return ctx
            .get_credential(name)
            .cloned()
            .ok_or_else(|| Error::Node(format!("Credential '{name}' not found")));
    }
    ctx.get_credential("openai")
        .or_else(|| ctx.get_credential("llm"))
        .cloned()
        .ok_or_else(|| {
            Error::Node(
                "No API credential found. Set 'credential' or add an 'openai'/'llm' credential"
                    .to_string(),
            )
        })
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
        .build()
        .map_err(|error| Error::Node(format!("Failed to build HTTP client: {error}")))
}

fn build_vision_request(
    config: &DocumentConfig,
    api_key: &str,
    prompt: &str,
    base64_data: &str,
    mime: &str,
) -> Result<(String, HeaderMap, Value)> {
    let url = match config.provider {
        Provider::Openai => config
            .api_url
            .clone()
            .unwrap_or_else(|| DEFAULT_OPENAI_URL.to_string()),
        Provider::Custom => config.api_url.clone().ok_or_else(|| {
            Error::Node("Document provider 'custom' requires 'api_url'".to_string())
        })?,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!("Bearer {api_key}")
            .parse()
            .map_err(|_| Error::Node("Invalid API key format".to_string()))?,
    );
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());

    let mut body = json!({
        "model": config.model,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": prompt},
                {"type": "image_url", "image_url": {
                    "url": format!("data:{mime};base64,{base64_data}")
                }}
            ]
        }]
    });
    if matches!(config.output, VisionOutput::Json) {
        body["response_format"] = json!({"type": "json_object"});
    }
    Ok((url, headers, body))
}

fn extract_json(content: &str) -> Option<Value> {
    let trimmed = content.trim();
    let unfenced = if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else {
        trimmed
    };
    serde_json::from_str::<Value>(unfenced)
        .ok()
        .filter(Value::is_object)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn minimal_pdf() -> Vec<u8> {
        let content = "BT /F1 18 Tf 72 720 Td (Hello r8r document) Tj ET\n";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
            format!(
                "<< /Length {} >>\nstream\n{content}endstream",
                content.len()
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0];
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    fn config(value: Value) -> DocumentConfig {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn pdf_text_extracts_each_page() {
        let result = extract_pdf_text(&minimal_pdf(), 200).unwrap();
        assert!(
            result.data["text"]
                .as_str()
                .unwrap()
                .contains("Hello r8r document")
        );
        assert_eq!(result.data["page_count"], 1);
        assert!(
            result.data["pages"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Hello r8r document")
        );
    }

    #[tokio::test]
    async fn pdf_text_accepts_base64_data() {
        let node = DocumentNode::new();
        let result = node
            .execute(
                &json!({"mode": "pdf_text", "data": STANDARD.encode(minimal_pdf())}),
                &NodeContext::new("test", "test"),
            )
            .await
            .unwrap();
        assert_eq!(result.data["page_count"], 1);
        assert!(
            result.data["text"]
                .as_str()
                .unwrap()
                .contains("Hello r8r document")
        );
    }

    #[tokio::test]
    async fn data_field_resolves_attachment_array() {
        let node = DocumentNode::new();
        let input = json!({"attachments": [{"data": STANDARD.encode(minimal_pdf())}]});
        let result = node
            .execute(
                &json!({"mode": "pdf_text", "data_field": "attachments.0.data"}),
                &NodeContext::new("test", "test").with_input(input),
            )
            .await
            .unwrap();
        assert!(
            result.data["pages"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Hello r8r document")
        );
    }

    #[test]
    fn vision_request_has_openai_content_parts_and_json_format() {
        let config = config(json!({
            "mode": "vision",
            "data": "AA==",
            "output": "json",
            "mime": "image/png"
        }));
        let (url, headers, body) =
            build_vision_request(&config, "secret", "Read it", "YWJj", &config.mime).unwrap();
        assert_eq!(url, DEFAULT_OPENAI_URL);
        assert_eq!(headers[AUTHORIZATION], "Bearer secret");
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert_eq!(body["messages"][0]["content"][0]["text"], "Read it");
        assert_eq!(body["messages"][0]["content"][1]["type"], "image_url");
        assert!(
            body["messages"][0]["content"][1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(body["response_format"]["type"], "json_object");
    }

    #[test]
    fn json_extraction_strips_markdown_fences() {
        assert_eq!(
            extract_json("```json\n{\"text\":\"ok\"}\n```"),
            Some(json!({"text": "ok"}))
        );
        assert_eq!(extract_json("not json"), None);
        assert_eq!(extract_json("[1, 2]"), None);
    }

    #[test]
    fn credential_resolution_uses_named_then_fallback_credentials() {
        let named = config(json!({"mode": "vision", "data": "AA==", "credential": "ocr"}));
        let mut credentials = HashMap::new();
        credentials.insert("ocr".to_string(), "named-key".to_string());
        credentials.insert("openai".to_string(), "fallback-key".to_string());
        let ctx = NodeContext::new("test", "test").with_credentials(credentials);
        assert_eq!(resolve_credential(&named, &ctx).unwrap(), "named-key");

        let fallback = config(json!({"mode": "vision", "data": "AA=="}));
        assert_eq!(resolve_credential(&fallback, &ctx).unwrap(), "fallback-key");
    }

    #[tokio::test]
    async fn config_errors_are_actionable() {
        let node = DocumentNode::new();
        let ctx = NodeContext::new("test", "test");
        let cases = [
            (json!({"mode": "pdf_text"}), "exactly one input source"),
            (
                json!({"mode": "pdf_text", "data": "AA==", "url": "https://example.com"}),
                "set only one",
            ),
            (
                json!({"mode": "unknown", "data": "AA=="}),
                "unknown variant",
            ),
            (
                json!({"mode": "pdf_text", "data": "%%%"}),
                "not valid STANDARD base64",
            ),
            (
                json!({"mode": "pdf_text", "data": "YWI=", "max_bytes": 1}),
                "exceeds max_bytes",
            ),
        ];
        for (config, expected) in cases {
            let error = node.execute(&config, &ctx).await.unwrap_err().to_string();
            assert!(
                error.contains(expected),
                "'{error}' did not contain '{expected}'"
            );
        }
    }

    #[test]
    fn document_node_type_and_registry_registration() {
        assert_eq!(DocumentNode::new().node_type(), "document");
        assert!(super::super::NodeRegistry::new().has("document"));
    }
}
