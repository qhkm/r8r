/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Top-level integration definition, typically loaded from a YAML file.
///
/// Each definition describes a single external service (e.g. GitHub, Slack)
/// including its base URL, authentication methods, and available operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationDefinition {
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub docs_url: Option<String>,
    #[serde(default)]
    pub auth: Option<AuthConfig>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    pub operations: HashMap<String, OperationDef>,
}

/// Authentication configuration for an integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub methods: Vec<AuthMethod>,
}

/// Supported authentication methods.
///
/// Uses `#[serde(tag = "type")]` so the YAML/JSON representation is:
/// ```yaml
/// type: oauth2
/// authorization_url: ...
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuthMethod {
    #[serde(rename = "oauth2")]
    OAuth2 {
        authorization_url: String,
        token_url: String,
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(default)]
        flows: Vec<String>,
    },
    #[serde(rename = "bearer")]
    Bearer {
        #[serde(default)]
        description: Option<String>,
    },
    #[serde(rename = "api_key")]
    ApiKey {
        #[serde(default)]
        header_name: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
}

/// A single API operation (e.g. "list_repos", "create_issue").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDef {
    pub description: String,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub params: HashMap<String, ParamDef>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub body: Option<Value>,
    #[serde(default)]
    pub query: Option<HashMap<String, String>>,
}

/// Definition of a single parameter within an operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDef {
    #[serde(default)]
    pub required: bool,
    #[serde(rename = "type", default)]
    pub param_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(rename = "enum", default)]
    pub enum_values: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_definition() {
        let yaml = r#"
version: 1
name: httpbin
base_url: https://httpbin.org
operations:
  get_ip:
    description: Returns the requester's IP address
    method: GET
    path: /ip
"#;

        let def: IntegrationDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.version, 1);
        assert_eq!(def.name, "httpbin");
        assert_eq!(def.base_url, "https://httpbin.org");
        assert!(def.display_name.is_none());
        assert!(def.description.is_none());
        assert!(def.auth.is_none());
        assert!(def.headers.is_none());

        let op = def
            .operations
            .get("get_ip")
            .expect("get_ip operation missing");
        assert_eq!(op.method, "GET");
        assert_eq!(op.path, "/ip");
        assert_eq!(op.description, "Returns the requester's IP address");
        assert!(op.params.is_empty());
        assert!(op.body.is_none());
        assert!(op.query.is_none());
    }

    #[test]
    fn test_parse_full_definition_with_auth() {
        let yaml = r#"
version: 1
name: github
display_name: GitHub API
description: GitHub REST API v3
base_url: https://api.github.com
docs_url: https://docs.github.com/en/rest
auth:
  methods:
    - type: oauth2
      authorization_url: https://github.com/login/oauth/authorize
      token_url: https://github.com/login/oauth/access_token
      scopes:
        - repo
        - user
      flows:
        - authorization_code
    - type: bearer
      description: Personal access token
headers:
  Accept: application/vnd.github.v3+json
  X-GitHub-Api-Version: "2022-11-28"
operations:
  list_repos:
    description: List repositories for the authenticated user
    method: GET
    path: /user/repos
    query:
      sort: updated
      per_page: "30"
  create_issue:
    description: Create an issue in a repository
    method: POST
    path: /repos/{owner}/{repo}/issues
    params:
      owner:
        required: true
        type: string
        description: Repository owner
      repo:
        required: true
        type: string
        description: Repository name
    body:
      title: "{{ title }}"
      body: "{{ body }}"
    headers:
      Content-Type: application/json
"#;

        let def: IntegrationDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.name, "github");
        assert_eq!(def.display_name.as_deref(), Some("GitHub API"));
        assert_eq!(def.description.as_deref(), Some("GitHub REST API v3"));
        assert_eq!(
            def.docs_url.as_deref(),
            Some("https://docs.github.com/en/rest")
        );

        // Auth
        let auth = def.auth.as_ref().expect("auth should be present");
        assert_eq!(auth.methods.len(), 2);

        match &auth.methods[0] {
            AuthMethod::OAuth2 {
                authorization_url,
                token_url,
                scopes,
                flows,
            } => {
                assert_eq!(
                    authorization_url,
                    "https://github.com/login/oauth/authorize"
                );
                assert_eq!(token_url, "https://github.com/login/oauth/access_token");
                assert_eq!(scopes, &["repo", "user"]);
                assert_eq!(flows, &["authorization_code"]);
            }
            other => panic!("expected OAuth2, got {:?}", other),
        }

        match &auth.methods[1] {
            AuthMethod::Bearer { description } => {
                assert_eq!(description.as_deref(), Some("Personal access token"));
            }
            other => panic!("expected Bearer, got {:?}", other),
        }

        // Global headers
        let headers = def.headers.as_ref().expect("headers should be present");
        assert_eq!(
            headers.get("Accept").unwrap(),
            "application/vnd.github.v3+json"
        );
        assert_eq!(headers.get("X-GitHub-Api-Version").unwrap(), "2022-11-28");

        // Operations
        assert_eq!(def.operations.len(), 2);

        let list_repos = def
            .operations
            .get("list_repos")
            .expect("list_repos missing");
        assert_eq!(list_repos.method, "GET");
        assert_eq!(list_repos.path, "/user/repos");
        let query = list_repos.query.as_ref().expect("query should be present");
        assert_eq!(query.get("sort").unwrap(), "updated");
        assert_eq!(query.get("per_page").unwrap(), "30");

        let create_issue = def
            .operations
            .get("create_issue")
            .expect("create_issue missing");
        assert_eq!(create_issue.method, "POST");
        assert!(create_issue.body.is_some());
        let op_headers = create_issue
            .headers
            .as_ref()
            .expect("operation headers missing");
        assert_eq!(op_headers.get("Content-Type").unwrap(), "application/json");
    }

    #[test]
    fn test_parse_param_types() {
        let yaml = r#"
version: 1
name: test-api
base_url: https://api.example.com
operations:
  search:
    description: Search with typed parameters
    method: GET
    path: /search
    params:
      query:
        required: true
        type: string
        description: Search query string
      page:
        required: false
        type: integer
        description: Page number
        default: 1
      tags:
        required: false
        type: array
        description: Filter by tags
      include_archived:
        required: false
        type: boolean
        description: Include archived results
        default: false
      status:
        type: string
        description: Filter by status
        enum:
          - active
          - inactive
          - pending
"#;

        let def: IntegrationDefinition = serde_yaml::from_str(yaml).unwrap();
        let op = def
            .operations
            .get("search")
            .expect("search operation missing");
        assert_eq!(op.params.len(), 5);

        // String param
        let query = op.params.get("query").expect("query param missing");
        assert!(query.required);
        assert_eq!(query.param_type.as_deref(), Some("string"));
        assert_eq!(query.description.as_deref(), Some("Search query string"));
        assert!(query.default.is_none());

        // Integer param with default
        let page = op.params.get("page").expect("page param missing");
        assert!(!page.required);
        assert_eq!(page.param_type.as_deref(), Some("integer"));
        assert_eq!(page.default.as_ref().unwrap(), &serde_json::json!(1));

        // Array param
        let tags = op.params.get("tags").expect("tags param missing");
        assert_eq!(tags.param_type.as_deref(), Some("array"));

        // Boolean param with default
        let include_archived = op
            .params
            .get("include_archived")
            .expect("include_archived param missing");
        assert_eq!(include_archived.param_type.as_deref(), Some("boolean"));
        assert_eq!(
            include_archived.default.as_ref().unwrap(),
            &serde_json::json!(false)
        );

        // Enum param
        let status = op.params.get("status").expect("status param missing");
        let enum_vals = status.enum_values.as_ref().expect("enum_values missing");
        assert_eq!(enum_vals, &["active", "inactive", "pending"]);
    }
}
