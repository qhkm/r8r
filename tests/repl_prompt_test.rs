use r8r::llm::LlmProvider;
use r8r::repl::prompt::build_repl_system_prompt;
use r8r::repl::tools::build_tool_definitions;

#[test]
fn test_system_prompt_contains_role() {
    let prompt = build_repl_system_prompt(&[], &[], &[], &[], LlmProvider::Openai);
    assert!(prompt.contains("r8r workflow assistant"));
}

#[test]
fn test_system_prompt_contains_tools() {
    let tools = build_tool_definitions();
    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t["function"]["name"].as_str())
        .collect();
    let prompt = build_repl_system_prompt(&tool_names, &[], &[], &[], LlmProvider::Openai);
    assert!(prompt.contains("r8r_execute"));
    assert!(prompt.contains("r8r_list_workflows"));
}

#[test]
fn test_system_prompt_contains_node_catalog() {
    let prompt = build_repl_system_prompt(
        &[],
        &["http", "transform", "agent"],
        &[],
        &[],
        LlmProvider::Openai,
    );
    assert!(prompt.contains("http"));
    assert!(prompt.contains("transform"));
    assert!(prompt.contains("agent"));
}

#[test]
fn test_system_prompt_contains_credentials() {
    let prompt = build_repl_system_prompt(
        &[],
        &[],
        &["slack-webhook", "openai-key"],
        &[],
        LlmProvider::Openai,
    );
    assert!(prompt.contains("slack-webhook"));
    assert!(prompt.contains("openai-key"));
}

#[test]
fn test_system_prompt_ollama_includes_text_tool_format() {
    let prompt = build_repl_system_prompt(&[], &[], &[], &[], LlmProvider::Ollama);
    assert!(prompt.contains("<tool_call>"));
    assert!(prompt.contains("</tool_call>"));
}

#[test]
fn test_system_prompt_openai_no_text_tool_format() {
    let prompt = build_repl_system_prompt(&[], &[], &[], &[], LlmProvider::Openai);
    assert!(!prompt.contains("<tool_call>"));
}

#[test]
fn test_system_prompt_contains_existing_workflows() {
    let prompt = build_repl_system_prompt(
        &[],
        &[],
        &[],
        &["btc-alert", "hn-digest"],
        LlmProvider::Openai,
    );
    assert!(prompt.contains("EXISTING WORKFLOWS"));
    assert!(prompt.contains("btc-alert"));
}
