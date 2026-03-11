use r8r::llm::{LlmConfig, LlmProvider};
use r8r::storage::SqliteStorage;

#[tokio::test]
async fn test_create_and_get_repl_session() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> = std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());

    let session_id = storage.create_repl_session("gpt-4o").await.unwrap();
    assert!(!session_id.is_empty());

    let session = storage.get_repl_session(&session_id).await.unwrap();
    assert!(session.is_some());
    let session = session.unwrap();
    assert_eq!(session.model, "gpt-4o");
    assert!(session.summary.is_none());
}

#[tokio::test]
async fn test_save_and_list_repl_messages() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> = std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());
    let session_id = storage.create_repl_session("gpt-4o").await.unwrap();

    storage
        .save_repl_message(&session_id, "user", "hello", None, None)
        .await
        .unwrap();
    storage
        .save_repl_message(&session_id, "assistant", "hi there!", None, None)
        .await
        .unwrap();

    let messages = storage.list_repl_messages(&session_id, 50).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].role, "assistant");
}

#[tokio::test]
async fn test_list_repl_sessions() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> = std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());
    storage.create_repl_session("gpt-4o").await.unwrap();
    storage
        .create_repl_session("claude-sonnet-4-20250514")
        .await
        .unwrap();

    let sessions = storage.list_repl_sessions(10).await.unwrap();
    assert_eq!(sessions.len(), 2);
}

#[tokio::test]
async fn test_update_repl_session_summary() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> = std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());
    let session_id = storage.create_repl_session("gpt-4o").await.unwrap();

    storage
        .update_repl_session_summary(&session_id, "Bitcoin price alerting")
        .await
        .unwrap();

    let session = storage
        .get_repl_session(&session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.summary.unwrap(), "Bitcoin price alerting");
}

#[tokio::test]
async fn test_save_and_get_repl_llm_config() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> = std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());
    let cfg = LlmConfig {
        provider: LlmProvider::Anthropic,
        model: Some("claude-sonnet-4-20250514".to_string()),
        api_key: Some("sk-ant-test".to_string()),
        endpoint: Some("https://api.anthropic.com/v1/messages".to_string()),
        temperature: Some(0.2),
        max_tokens: Some(2048),
        timeout_seconds: 90,
    };

    storage.save_repl_llm_config(&cfg).await.unwrap();
    let loaded = storage.get_repl_llm_config().await.unwrap().unwrap();
    assert_eq!(loaded.provider, LlmProvider::Anthropic);
    assert_eq!(loaded.model.as_deref(), Some("claude-sonnet-4-20250514"));
    assert_eq!(loaded.api_key.as_deref(), Some("sk-ant-test"));
    assert_eq!(
        loaded.endpoint.as_deref(),
        Some("https://api.anthropic.com/v1/messages")
    );
    assert_eq!(loaded.temperature, Some(0.2));
    assert_eq!(loaded.max_tokens, Some(2048));
    assert_eq!(loaded.timeout_seconds, 90);
}

#[tokio::test]
async fn repl_message_stores_run_id() {
    use r8r::storage::SqliteStorage;
    let db = SqliteStorage::open_in_memory().unwrap();
    let sess_id = db.create_repl_session("test-model").await.unwrap();
    db.save_repl_message(&sess_id, "user", "hello", None, Some("exec-abc"))
        .await
        .unwrap();
    let msgs = db.list_repl_messages(&sess_id, 10).await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].run_id.as_deref(), Some("exec-abc"));
}

#[tokio::test]
async fn repl_message_run_id_defaults_to_none() {
    use r8r::storage::SqliteStorage;
    let db = SqliteStorage::open_in_memory().unwrap();
    let sess_id = db.create_repl_session("test-model").await.unwrap();
    db.save_repl_message(&sess_id, "user", "hello", None, None)
        .await
        .unwrap();
    let msgs = db.list_repl_messages(&sess_id, 10).await.unwrap();
    assert_eq!(msgs[0].run_id, None);
}

#[test]
fn redact_credentials_replaces_sensitive_keys() {
    use r8r::repl::tui::redact_credentials;
    use serde_json::json;

    let input = json!({
        "url": "https://api.example.com",
        "api_key": "sk-super-secret",
        "token": "bearer-abc123",
        "password": "hunter2",
        "username": "alice",
        "nested": {
            "secret": "shh",
            "data": "safe"
        }
    });

    let output = redact_credentials(&input);

    assert_eq!(output["url"], "https://api.example.com");
    assert_eq!(output["username"], "alice");
    assert_eq!(output["api_key"], "[REDACTED]");
    assert_eq!(output["token"], "[REDACTED]");
    assert_eq!(output["password"], "[REDACTED]");
    assert_eq!(output["nested"]["secret"], "[REDACTED]");
    assert_eq!(output["nested"]["data"], "safe");

    // Verify S3-style "key" field is NOT redacted (bare "key" was removed from sensitive list)
    let s3_input = serde_json::json!({"key": "path/to/object.txt", "bucket": "my-bucket"});
    let s3_output = redact_credentials(&s3_input);
    assert_eq!(s3_output["key"], "path/to/object.txt", "bare 'key' field should not be redacted");
    assert_eq!(s3_output["bucket"], "my-bucket");
}
