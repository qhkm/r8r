use r8r::engine::Executor;
use r8r::nodes::NodeRegistry;
use r8r::repl::conversation::Conversation;
use r8r::repl::input::{parse_input, InputCommand};
use r8r::repl::tools::{build_tool_definitions, execute_tool};
use r8r::storage::SqliteStorage;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn test_tool_execution_list_workflows() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> =
        std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());
    let executor = Arc::new(Executor::new(NodeRegistry::new(), storage.clone()));

    let result = execute_tool("r8r_list_workflows", &json!({}), &storage, &executor).await;
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(v["workflows"].is_array());
}

#[tokio::test]
async fn test_tool_execution_validate_valid_yaml() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> =
        std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());
    let executor = Arc::new(Executor::new(NodeRegistry::new(), storage.clone()));

    let yaml = "name: test\nnodes:\n  - id: step1\n    type: transform\n    config:\n      expression: '\"hello\"'\n";
    let result = execute_tool("r8r_validate", &json!({"yaml": yaml}), &storage, &executor).await;
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], true);
}

#[tokio::test]
async fn test_tool_execution_validate_invalid_yaml() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> =
        std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());
    let executor = Arc::new(Executor::new(NodeRegistry::new(), storage.clone()));

    let result = execute_tool(
        "r8r_validate",
        &json!({"yaml": "not valid yaml [[["}),
        &storage,
        &executor,
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], false);
}

#[tokio::test]
async fn test_tool_execution_unknown_tool() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> =
        std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());
    let executor = Arc::new(Executor::new(NodeRegistry::new(), storage.clone()));

    let result = execute_tool("r8r_nonexistent", &json!({}), &storage, &executor).await;
    assert!(result.contains("Unknown tool"));
}

#[tokio::test]
async fn test_conversation_with_tool_calls() {
    let mut conv = Conversation::new(50);
    conv.add_user_message("list my workflows");
    conv.add_tool_call("call_1", "r8r_list_workflows", &json!({}));
    conv.add_tool_result("call_1", r#"{"workflows":[],"count":0}"#);
    conv.add_assistant_message("You have no workflows yet. Would you like to create one?");

    let messages = conv.messages_for_llm();
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[3]["role"], "assistant");
}

#[test]
fn test_tool_definitions_valid_json() {
    let defs = build_tool_definitions();
    for def in &defs {
        assert_eq!(def["type"].as_str(), Some("function"));
        assert!(def["function"]["name"].is_string());
        assert!(def["function"]["description"].is_string());
        assert!(def["function"]["parameters"].is_object());
    }
}

#[test]
fn test_slash_command_routing() {
    assert!(matches!(parse_input("/list"), InputCommand::List));
    assert!(matches!(parse_input("/help"), InputCommand::Help));
    assert!(matches!(parse_input("/exit"), InputCommand::Exit));
    assert!(matches!(parse_input("/quit"), InputCommand::Exit));
    assert!(matches!(parse_input("/q"), InputCommand::Exit));

    assert!(matches!(
        parse_input("list my workflows"),
        InputCommand::NaturalLanguage(_)
    ));
    assert!(matches!(
        parse_input("help me create something"),
        InputCommand::NaturalLanguage(_)
    ));
}
