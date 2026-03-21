/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */

//! Integration tests for the IntegrationNode pipeline.

use r8r::integrations::loader::IntegrationLoader;
use r8r::integrations::node::build_http_config;
use r8r::integrations::validator::validate_and_resolve_params;
use serde_json::json;

#[test]
fn test_full_pipeline_github_create_issue() {
    let loader = IntegrationLoader::new();
    let def = loader.get("github").unwrap();
    let op = def.operations.get("create_issue").unwrap();
    let params = json!({
        "owner": "octocat",
        "repo": "hello-world",
        "title": "Test issue",
        "labels": ["bug", "help wanted"]
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("my_token"));
    assert_eq!(config["url"], "https://api.github.com/repos/octocat/hello-world/issues");
    assert_eq!(config["method"], "POST");
    assert_eq!(config["body"]["title"], "Test issue");
    assert!(config["body"]["labels"].is_array());
    assert_eq!(config["body"]["labels"][0], "bug");
    assert_eq!(config["credential"], "my_token");
    assert_eq!(config["headers"]["Accept"], "application/vnd.github+json");
}

#[test]
fn test_full_pipeline_openai_completion() {
    let loader = IntegrationLoader::new();
    let def = loader.get("openai").unwrap();
    let op = def.operations.get("chat_completion").unwrap();
    let params = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}],
        "temperature": 0.7
    });
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("openai_key"));
    assert_eq!(config["url"], "https://api.openai.com/v1/chat/completions");
    assert_eq!(config["body"]["model"], "gpt-4");
    assert!(config["body"]["messages"].is_array());
    assert_eq!(config["body"]["temperature"], 0.7);
}

#[test]
fn test_full_pipeline_slack_send_message() {
    let loader = IntegrationLoader::new();
    let def = loader.get("slack").unwrap();
    let op = def.operations.get("send_message").unwrap();
    let params = json!({"channel": "#general", "text": "Hello from r8r!"});
    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("slack_token"));
    assert_eq!(config["url"], "https://slack.com/api/chat.postMessage");
    assert_eq!(config["body"]["channel"], "#general");
    assert_eq!(config["body"]["text"], "Hello from r8r!");
}

#[test]
fn test_all_builtin_definitions_parse() {
    let loader = IntegrationLoader::new();
    let services = loader.list_all();
    assert!(services.contains(&"github".to_string()));
    assert!(services.contains(&"slack".to_string()));
    assert!(services.contains(&"openai".to_string()));
    assert!(services.contains(&"stripe".to_string()));
    assert!(services.contains(&"notion".to_string()));
    for service in &services {
        let def = loader.get(service).unwrap();
        assert!(!def.operations.is_empty(), "Service '{}' has no operations", service);
        assert!(!def.base_url.is_empty(), "Service '{}' has no base_url", service);
    }
}

#[test]
fn test_user_override_takes_precedence() {
    let loader = IntegrationLoader::new();
    let github = loader.get_builtin("github");
    assert!(github.is_some());
    assert_eq!(github.unwrap().base_url, "https://api.github.com");
}
