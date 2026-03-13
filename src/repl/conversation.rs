/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Conversation history manager for the REPL.
//!
//! Tracks multi-turn messages and manages context window size.

use serde_json::{json, Value};

/// Manages the conversation history for a REPL session.
#[derive(Clone)]
pub struct Conversation {
    /// Full message history (for persistence).
    messages: Vec<Value>,
    /// Maximum messages to send to the LLM.
    max_context_messages: usize,
}

impl Conversation {
    /// Create a new empty conversation with a context window limit.
    pub fn new(max_context_messages: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_context_messages,
        }
    }

    /// Get all messages (full history).
    pub fn messages(&self) -> &[Value] {
        &self.messages
    }

    /// Get messages truncated to the context window for sending to the LLM.
    pub fn messages_for_llm(&self) -> Vec<Value> {
        if self.messages.len() <= self.max_context_messages {
            return self.messages.clone();
        }

        if self.max_context_messages <= 1 {
            return vec![json!({
                "role": "system",
                "content": "Conversation summary: context window too small; only latest turn retained."
            })];
        }

        let keep_tail = self.max_context_messages - 1;
        let split_at = self.messages.len().saturating_sub(keep_tail);
        let older = &self.messages[..split_at];
        let newer = &self.messages[split_at..];

        let mut user_count = 0usize;
        let mut assistant_count = 0usize;
        let mut tool_call_count = 0usize;
        let mut tool_result_count = 0usize;
        let mut snippets = Vec::new();

        for m in older {
            let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("unknown");
            match role {
                "user" => user_count += 1,
                "assistant" => {
                    if m.get("tool_calls").is_some() {
                        tool_call_count += 1;
                    } else {
                        assistant_count += 1;
                    }
                }
                "tool" => tool_result_count += 1,
                _ => {}
            }

            if snippets.len() < 6 && (role == "user" || role == "assistant") {
                if let Some(content) = m.get("content").and_then(|v| v.as_str()) {
                    let compact = content.replace('\n', " ");
                    let trimmed = compact.chars().take(100).collect::<String>();
                    snippets.push(format!("{}: {}", role, trimmed));
                }
            }
        }

        let mut summary = format!(
            "Previous context summarized: {} user messages, {} assistant messages, {} tool calls, {} tool results.",
            user_count, assistant_count, tool_call_count, tool_result_count
        );
        if !snippets.is_empty() {
            summary.push_str("\nHighlights:\n");
            for s in snippets {
                summary.push_str("- ");
                summary.push_str(&s);
                summary.push('\n');
            }
        }

        let mut compact = Vec::with_capacity(self.max_context_messages);
        compact.push(json!({
            "role": "system",
            "content": summary.trim()
        }));
        compact.extend_from_slice(newer);
        compact
    }

    /// Add a user message.
    pub fn add_user_message(&mut self, content: &str) {
        self.messages.push(json!({
            "role": "user",
            "content": content
        }));
    }

    /// Add an assistant text message.
    pub fn add_assistant_message(&mut self, content: &str) {
        self.messages.push(json!({
            "role": "assistant",
            "content": content
        }));
    }

    /// Add an assistant tool call.
    pub fn add_tool_call(&mut self, call_id: &str, tool_name: &str, arguments: &Value) {
        self.messages.push(json!({
            "role": "assistant",
            "tool_calls": [{
                "id": call_id,
                "type": "function",
                "function": {
                    "name": tool_name,
                    "arguments": arguments.to_string()
                }
            }]
        }));
    }

    /// Add a tool result.
    pub fn add_tool_result(&mut self, call_id: &str, result: &str) {
        self.messages.push(json!({
            "role": "tool",
            "tool_call_id": call_id,
            "content": result
        }));
    }

    /// Clear all messages.
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Load messages from storage (for session resume).
    pub fn load_from_stored(&mut self, messages: Vec<Value>) {
        self.messages = messages;
    }

    /// Get the total number of messages.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Check if conversation is empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}
