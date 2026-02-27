//! Integration tests for the prompt-to-workflow generator module.
//!
//! These tests verify prompt assembly, YAML extraction, and the validation
//! pipeline without making live LLM calls.

use r8r::generator::context::GeneratorContext;
use r8r::generator::prompt;

#[test]
fn test_system_prompt_contains_all_registered_nodes() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let ctx = rt.block_on(GeneratorContext::build());
    let system = prompt::build_system_prompt(&ctx);

    // Must mention key node types
    assert!(system.contains("http"), "System prompt missing http node");
    assert!(system.contains("transform"), "System prompt missing transform node");
    assert!(system.contains("agent"), "System prompt missing agent node");
    assert!(system.contains("filter"), "System prompt missing filter node");
    assert!(system.contains("switch"), "System prompt missing switch node");
}

#[test]
fn test_system_prompt_contains_rules() {
    let ctx = GeneratorContext {
        node_catalog: vec![("http".to_string(), "HTTP requests".to_string())],
        credential_names: vec![],
        examples: vec![],
    };
    let system = prompt::build_system_prompt(&ctx);

    assert!(system.contains("RULES:"), "System prompt missing rules section");
    assert!(system.contains("env.VAR_NAME"), "System prompt missing env var instruction");
    assert!(system.contains("depends_on"), "System prompt missing depends_on instruction");
}

#[test]
fn test_extract_yaml_from_various_formats() {
    // Clean YAML
    assert_eq!(
        prompt::extract_yaml("name: test\nnodes: []"),
        "name: test\nnodes: []"
    );

    // With yaml fences
    assert_eq!(
        prompt::extract_yaml("```yaml\nname: test\nnodes: []\n```"),
        "name: test\nnodes: []"
    );

    // With plain fences
    assert_eq!(
        prompt::extract_yaml("```\nname: test\nnodes: []\n```"),
        "name: test\nnodes: []"
    );

    // With surrounding whitespace
    assert_eq!(
        prompt::extract_yaml("  \n  name: test\n  "),
        "name: test"
    );
}

#[test]
fn test_create_prompt_includes_user_intent() {
    let p = prompt::build_create_prompt("monitor HN and send to Slack");
    assert!(p.contains("monitor HN and send to Slack"));
    assert!(p.contains("YAML"));
}

#[test]
fn test_refine_prompt_includes_current_and_change() {
    let p = prompt::build_refine_prompt(
        "name: my-wf\nnodes:\n  - id: a\n    type: http",
        "add retry logic",
    );
    assert!(p.contains("name: my-wf"));
    assert!(p.contains("add retry logic"));
}

#[test]
fn test_fix_prompt_includes_yaml_and_errors() {
    let p = prompt::build_fix_prompt("name: bad\nnodes: []", "Workflow must have at least one node");
    assert!(p.contains("name: bad"));
    assert!(p.contains("Workflow must have at least one node"));
}

#[test]
fn test_system_prompt_omits_credentials_when_empty() {
    let ctx = GeneratorContext {
        node_catalog: vec![("http".to_string(), "HTTP".to_string())],
        credential_names: vec![],
        examples: vec![],
    };
    let prompt = prompt::build_system_prompt(&ctx);
    assert!(!prompt.contains("CONFIGURED CREDENTIALS"));
}

#[test]
fn test_system_prompt_includes_credentials_when_present() {
    let ctx = GeneratorContext {
        node_catalog: vec![("http".to_string(), "HTTP".to_string())],
        credential_names: vec!["openai-key".to_string(), "slack-token".to_string()],
        examples: vec![],
    };
    let prompt = prompt::build_system_prompt(&ctx);
    assert!(prompt.contains("CONFIGURED CREDENTIALS"));
    assert!(prompt.contains("openai-key"));
    assert!(prompt.contains("slack-token"));
}
