/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Parser for n8n workflow JSON export format.
//!
//! Deserialises an n8n JSON export into typed Rust structs so the
//! migration layer can inspect nodes, connections and credentials
//! without reaching into raw `serde_json::Value` everywhere.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};

/// Top-level n8n workflow export.
#[derive(Debug, Deserialize)]
pub struct N8nWorkflow {
    pub name: String,
    pub nodes: Vec<N8nNode>,
    #[serde(default)]
    pub connections: HashMap<String, N8nNodeConnections>,
    #[serde(default)]
    pub settings: Option<Value>,
}

/// A single node inside an n8n workflow.
#[derive(Debug, Clone, Deserialize)]
pub struct N8nNode {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
    #[serde(rename = "typeVersion", default)]
    pub type_version: u32,
    #[serde(default)]
    pub position: Vec<i32>,
    #[serde(default)]
    pub parameters: Value,
    #[serde(default)]
    pub credentials: Option<HashMap<String, N8nCredentialRef>>,
}

impl N8nNode {
    /// Extract the short node type from the fully-qualified name.
    ///
    /// `"n8n-nodes-base.httpRequest"` → `"httpRequest"`
    pub fn node_type(&self) -> &str {
        self.type_name.rsplit('.').next().unwrap_or(&self.type_name)
    }
}

/// Reference to a credential set attached to a node.
#[derive(Debug, Clone, Deserialize)]
pub struct N8nCredentialRef {
    pub id: String,
    pub name: String,
}

/// Connection outputs for a single node, keyed by output type.
#[derive(Debug, Clone, Deserialize)]
pub struct N8nNodeConnections {
    #[serde(default)]
    pub main: Vec<Vec<N8nConnectionTarget>>,
}

/// A single connection target (downstream node + input index).
#[derive(Debug, Clone, Deserialize)]
pub struct N8nConnectionTarget {
    pub node: String,
    #[serde(rename = "type", default)]
    pub conn_type: String,
    #[serde(default)]
    pub index: usize,
}

/// Parse raw bytes into a typed [`N8nWorkflow`].
pub fn parse_n8n_workflow(input: &[u8]) -> Result<N8nWorkflow> {
    serde_json::from_slice(input)
        .map_err(|e| Error::Parse(format!("Failed to parse n8n workflow JSON: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_n8n_workflow() {
        let json = r#"{
            "name": "My Workflow",
            "nodes": [
                {
                    "id": "1",
                    "name": "Start",
                    "type": "n8n-nodes-base.start",
                    "typeVersion": 1,
                    "position": [250, 300],
                    "parameters": {}
                },
                {
                    "id": "2",
                    "name": "HTTP Request",
                    "type": "n8n-nodes-base.httpRequest",
                    "typeVersion": 3,
                    "position": [450, 300],
                    "parameters": {
                        "url": "https://example.com/api"
                    }
                }
            ],
            "connections": {
                "Start": {
                    "main": [
                        [
                            { "node": "HTTP Request", "type": "main", "index": 0 }
                        ]
                    ]
                }
            }
        }"#;

        let wf = parse_n8n_workflow(json.as_bytes()).unwrap();

        assert_eq!(wf.name, "My Workflow");
        assert_eq!(wf.nodes.len(), 2);

        // First node
        assert_eq!(wf.nodes[0].name, "Start");
        assert_eq!(wf.nodes[0].type_name, "n8n-nodes-base.start");
        assert_eq!(wf.nodes[0].type_version, 1);

        // Second node
        assert_eq!(wf.nodes[1].name, "HTTP Request");
        assert_eq!(wf.nodes[1].type_name, "n8n-nodes-base.httpRequest");
        assert_eq!(
            wf.nodes[1].parameters["url"].as_str().unwrap(),
            "https://example.com/api"
        );

        // Connection
        let conn = wf.connections.get("Start").unwrap();
        assert_eq!(conn.main.len(), 1);
        assert_eq!(conn.main[0].len(), 1);
        assert_eq!(conn.main[0][0].node, "HTTP Request");
        assert_eq!(conn.main[0][0].conn_type, "main");
        assert_eq!(conn.main[0][0].index, 0);
    }

    #[test]
    fn test_parse_node_with_credentials() {
        let json = r##"{
            "name": "Credential Workflow",
            "nodes": [
                {
                    "id": "1",
                    "name": "Slack",
                    "type": "n8n-nodes-base.slack",
                    "typeVersion": 2,
                    "position": [250, 300],
                    "parameters": { "channel": "#general" },
                    "credentials": {
                        "slackApi": {
                            "id": "cred-42",
                            "name": "My Slack Token"
                        }
                    }
                }
            ],
            "connections": {}
        }"##;

        let wf = parse_n8n_workflow(json.as_bytes()).unwrap();
        assert_eq!(wf.nodes.len(), 1);

        let creds = wf.nodes[0].credentials.as_ref().unwrap();
        let slack_cred = creds.get("slackApi").unwrap();
        assert_eq!(slack_cred.id, "cred-42");
        assert_eq!(slack_cred.name, "My Slack Token");
    }

    #[test]
    fn test_node_type_extraction() {
        let node = N8nNode {
            id: "1".into(),
            name: "HTTP".into(),
            type_name: "n8n-nodes-base.httpRequest".into(),
            type_version: 3,
            position: vec![],
            parameters: Value::Null,
            credentials: None,
        };

        assert_eq!(node.node_type(), "httpRequest");
    }

    #[test]
    fn test_node_type_community_package() {
        let node = N8nNode {
            id: "2".into(),
            name: "Custom".into(),
            type_name: "n8n-nodes-custom.myNode".into(),
            type_version: 1,
            position: vec![],
            parameters: Value::Null,
            credentials: None,
        };

        assert_eq!(node.node_type(), "myNode");
    }
}
