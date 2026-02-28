use r8r::llm::{
    parse_anthropic_sse_line, parse_ollama_ndjson_line, parse_openai_sse_line, StreamEvent,
};

#[test]
fn test_parse_openai_sse_text_delta() {
    let line = r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#;
    let event = parse_openai_sse_line(line);
    assert!(matches!(event, Some(StreamEvent::TextDelta(t)) if t == "Hello"));
}

#[test]
fn test_parse_openai_sse_done() {
    let line = "data: [DONE]";
    let event = parse_openai_sse_line(line);
    assert!(matches!(event, Some(StreamEvent::Done)));
}

#[test]
fn test_parse_openai_sse_tool_call() {
    let line = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"r8r_list_workflows","arguments":""}}]}}]}"#;
    let event = parse_openai_sse_line(line);
    assert!(matches!(event, Some(StreamEvent::ToolCallStart { .. })));
}

#[test]
fn test_parse_anthropic_sse_text_delta() {
    let line =
        r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"World"}}"#;
    let event = parse_anthropic_sse_line(line);
    assert!(matches!(event, Some(StreamEvent::TextDelta(t)) if t == "World"));
}

#[test]
fn test_parse_anthropic_sse_tool_use() {
    let line = r#"data: {"type":"content_block_start","content_block":{"type":"tool_use","id":"toolu_123","name":"r8r_execute","input":{}}}"#;
    let event = parse_anthropic_sse_line(line);
    assert!(matches!(event, Some(StreamEvent::ToolCallStart { .. })));
}

#[test]
fn test_parse_ollama_ndjson() {
    let line = r#"{"message":{"content":"Hi"},"done":false}"#;
    let event = parse_ollama_ndjson_line(line);
    assert!(matches!(event, Some(StreamEvent::TextDelta(t)) if t == "Hi"));
}

#[test]
fn test_parse_ollama_ndjson_done() {
    let line = r#"{"message":{"content":""},"done":true}"#;
    let event = parse_ollama_ndjson_line(line);
    assert!(matches!(event, Some(StreamEvent::Done)));
}
