use r8r::repl::conversation::Conversation;
use serde_json::json;

#[test]
fn test_new_conversation_is_empty() {
    let conv = Conversation::new(50);
    assert!(conv.messages().is_empty());
}

#[test]
fn test_add_user_message() {
    let mut conv = Conversation::new(50);
    conv.add_user_message("hello");
    assert_eq!(conv.messages().len(), 1);
    assert_eq!(conv.messages()[0]["role"], "user");
    assert_eq!(conv.messages()[0]["content"], "hello");
}

#[test]
fn test_add_assistant_message() {
    let mut conv = Conversation::new(50);
    conv.add_user_message("hello");
    conv.add_assistant_message("hi there!");
    assert_eq!(conv.messages().len(), 2);
    assert_eq!(conv.messages()[1]["role"], "assistant");
}

#[test]
fn test_add_tool_call_and_result() {
    let mut conv = Conversation::new(50);
    conv.add_user_message("list workflows");
    conv.add_tool_call("call_1", "r8r_list_workflows", &json!({}));
    conv.add_tool_result("call_1", r#"{"workflows":[]}"#);
    assert_eq!(conv.messages().len(), 3);
}

#[test]
fn test_context_window_truncation() {
    let mut conv = Conversation::new(5);
    for i in 0..10 {
        conv.add_user_message(&format!("message {}", i));
        conv.add_assistant_message(&format!("reply {}", i));
    }
    let msgs = conv.messages_for_llm();
    assert!(msgs.len() <= 5);
    let last = &msgs[msgs.len() - 1];
    assert_eq!(last["content"], "reply 9");
}

#[test]
fn test_clear() {
    let mut conv = Conversation::new(50);
    conv.add_user_message("hello");
    conv.add_assistant_message("hi");
    conv.clear();
    assert!(conv.messages().is_empty());
}

#[test]
fn test_context_window_summarizes_overflow() {
    let mut conv = Conversation::new(4);
    for i in 0..8 {
        conv.add_user_message(&format!("u{}", i));
        conv.add_assistant_message(&format!("a{}", i));
    }
    let msgs = conv.messages_for_llm();
    assert_eq!(msgs.len(), 4);
    assert_eq!(msgs[0]["role"], "system");
    assert!(msgs[0]["content"]
        .as_str()
        .unwrap()
        .contains("Previous context summarized"));
}
