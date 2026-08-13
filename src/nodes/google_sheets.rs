/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Google Sheets node - read/write values in Google Sheets via Sheets API v4.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::debug;

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Google Sheets node for reading and appending spreadsheet values.
pub struct GoogleSheetsNode;

impl GoogleSheetsNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GoogleSheetsNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SheetsAction {
    /// Read values from a range
    Read,
    /// Append values to a range
    Append,
    /// Write values to a range (overwrite)
    Update,
}

#[derive(Debug, Deserialize)]
struct SheetsConfig {
    /// Action to perform: "read", "append", "update"
    #[serde(default)]
    action: Option<SheetsAction>,

    /// Spreadsheet ID (from the sheet URL)
    spreadsheet_id: String,

    /// Sheet range in A1 notation (e.g., "Sheet1!A1:C10")
    range: String,

    /// Values (for append/update): array of rows, each row an array of cells
    #[serde(default)]
    values: Option<Vec<Vec<Value>>>,

    /// Credential name (OAuth2 access token for Google Sheets)
    #[serde(default)]
    credential: Option<String>,

    /// Inline access token (alternative to credential)
    #[serde(default)]
    token: Option<String>,
}

/// Resolve the Google Sheets access token from the node context.
fn resolve_access_token(config: &SheetsConfig, ctx: &NodeContext) -> Option<String> {
    if let Some(token) = &config.token {
        return Some(token.clone());
    }

    if let Some(cred_name) = &config.credential {
        // Prefer OAuth2 credential store (proper token with refresh)
        if let Some(oauth) = ctx.oauth2_credentials.get(cred_name) {
            return Some(oauth.access_token.clone());
        }
        // Fall back to plain credential store (e.g., API key or raw token)
        if let Some(plain) = ctx.credentials.get(cred_name) {
            return Some(plain.clone());
        }
    }

    None
}

#[async_trait]
impl Node for GoogleSheetsNode {
    fn node_type(&self) -> &str {
        "google-sheets"
    }

    fn description(&self) -> &str {
        "Read, append, or update values in Google Sheets"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: SheetsConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid google-sheets config: {}", e)))?;

        let action = config.action.unwrap_or(SheetsAction::Read);

        let token = resolve_access_token(&config, ctx).ok_or_else(|| {
            Error::Node(
                "Google Sheets requires 'token' or 'credential' (OAuth2 access token)".to_string(),
            )
        })?;

        let client = reqwest::Client::new();
        let base = format!(
            "https://sheets.googleapis.com/v4/spreadsheets/{}/values/{}",
            config.spreadsheet_id, config.range
        );

        debug!(
            action = ?action,
            spreadsheet_id = %config.spreadsheet_id,
            range = %config.range,
            "Google Sheets request"
        );

        match action {
            SheetsAction::Read => {
                let url = format!("{}?valueRenderOption=UNFORMATTED_VALUE", base);
                let response = client
                    .get(&url)
                    .bearer_auth(&token)
                    .send()
                    .await
                    .map_err(|e| Error::Node(format!("Google Sheets read failed: {}", e)))?;

                let status = response.status();
                let body: Value = response.json().await.unwrap_or(json!({}));

                if status.is_success() {
                    Ok(NodeResult::new(json!({
                        "success": true,
                        "range": body["range"],
                        "values": body["values"].as_array().cloned().unwrap_or_default(),
                    })))
                } else {
                    Err(Error::Node(format!(
                        "Google Sheets read error: {}",
                        body["error"]["message"].as_str().unwrap_or("unknown")
                    )))
                }
            }
            SheetsAction::Append => {
                let values = config.values.clone().ok_or_else(|| {
                    Error::Node("Google Sheets 'append' requires 'values'".to_string())
                })?;

                let url = format!(
                    "{}:append?valueInputOption=USER_ENTERED&insertDataOption=INSERT_ROWS",
                    base
                );
                let payload = json!({ "values": values });

                let response = client
                    .post(&url)
                    .bearer_auth(&token)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| Error::Node(format!("Google Sheets append failed: {}", e)))?;

                let status = response.status();
                let body: Value = response.json().await.unwrap_or(json!({}));

                if status.is_success() {
                    Ok(NodeResult::new(json!({
                        "success": true,
                        "updates": {
                            "updated_range": body["updates"]["updatedRange"],
                            "updated_rows": body["updates"]["updatedRows"],
                        }
                    })))
                } else {
                    Err(Error::Node(format!(
                        "Google Sheets append error: {}",
                        body["error"]["message"].as_str().unwrap_or("unknown")
                    )))
                }
            }
            SheetsAction::Update => {
                let values = config.values.clone().ok_or_else(|| {
                    Error::Node("Google Sheets 'update' requires 'values'".to_string())
                })?;

                let url = format!("{}?valueInputOption=USER_ENTERED", base);
                let payload = json!({ "values": values });

                let response = client
                    .put(&url)
                    .bearer_auth(&token)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| Error::Node(format!("Google Sheets update failed: {}", e)))?;

                let status = response.status();
                let body: Value = response.json().await.unwrap_or(json!({}));

                if status.is_success() {
                    Ok(NodeResult::new(json!({
                        "success": true,
                        "updates": {
                            "updated_range": body["updates"]["updatedRange"],
                            "updated_rows": body["updates"]["updatedRows"],
                        }
                    })))
                } else {
                    Err(Error::Node(format!(
                        "Google Sheets update error: {}",
                        body["error"]["message"].as_str().unwrap_or("unknown")
                    )))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sheets_config_parse_read() {
        let config = json!({
            "action": "read",
            "spreadsheet_id": "abc123",
            "range": "Sheet1!A1:C10",
            "token": "ya29.abc"
        });

        let parsed: SheetsConfig = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.action, Some(SheetsAction::Read));
        assert_eq!(parsed.spreadsheet_id, "abc123");
        assert_eq!(parsed.range, "Sheet1!A1:C10");
    }

    #[tokio::test]
    async fn test_sheets_config_parse_append() {
        let config = json!({
            "action": "append",
            "spreadsheet_id": "abc123",
            "range": "Sheet1!A1",
            "values": [["Name", "Score"], ["Ali", 90]],
            "token": "ya29.abc"
        });

        let parsed: SheetsConfig = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.action, Some(SheetsAction::Append));
        assert_eq!(parsed.values.as_ref().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_sheets_missing_credential() {
        let node = GoogleSheetsNode::new();
        let config = json!({
            "action": "read",
            "spreadsheet_id": "abc123",
            "range": "Sheet1!A1:C10"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
    }
}
