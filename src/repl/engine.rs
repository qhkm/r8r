/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Core REPL agentic loop.
//!
//! Processes user input through the LLM with autonomous tool execution.

use crate::engine::Executor;
use crate::llm::{self, LlmConfig, StreamEvent};
use crate::storage::Storage;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;

use super::conversation::Conversation;
use super::tools;

const MAX_TOOL_CALLS_PER_TURN: usize = 20;

/// Result of processing a single user turn.
pub struct TurnResult {
    /// Final text response from the LLM.
    pub response: String,
    /// Tool calls made during this turn (name, args, result).
    pub tool_calls: Vec<ToolCallRecord>,
    /// Whether the turn was cancelled.
    pub cancelled: bool,
    /// Accumulated token usage across all LLM calls in this turn.
    pub usage: Option<crate::llm::LlmUsage>,
}

/// Record of a single tool call during a turn.
pub struct ToolCallRecord {
    pub name: String,
    pub args: Value,
    pub result: String,
}

/// Callback for streaming events to the TUI.
pub type StreamCallback = Box<dyn Fn(StreamUpdate) + Send + Sync>;

/// Events sent to the TUI during the agentic loop.
#[derive(Debug, Clone)]
pub enum StreamUpdate {
    /// A text token from the LLM.
    Token(String),
    /// A tool call is being executed.
    ToolCallStart { name: String, args: Value },
    /// A tool call completed.
    ToolCallResult { name: String, result: String },
    /// The turn is complete.
    Done,
    /// An error occurred.
    Error(String),
}

/// Run a single agentic turn: user message → LLM + tools → response.
#[allow(clippy::too_many_arguments)]
pub async fn run_turn(
    conversation: &mut Conversation,
    user_input: &str,
    system_prompt: &str,
    llm_config: &LlmConfig,
    llm_client: &reqwest::Client,
    storage: &dyn Storage,
    executor: &Arc<Executor>,
    tool_defs: &[Value],
    on_update: &StreamCallback,
) -> TurnResult {
    conversation.add_user_message(user_input);

    let mut tool_calls = Vec::new();
    let mut total_tool_calls = 0;
    let mut accumulated_usage: Option<crate::llm::LlmUsage> = None;

    loop {
        let messages = conversation.messages_for_llm();
        let tools_param = if tool_defs.is_empty() {
            None
        } else {
            Some(tool_defs)
        };

        let rx = match llm::call_llm_streaming(
            llm_client,
            llm_config,
            Some(system_prompt),
            &messages,
            tools_param,
        )
        .await
        {
            Ok(rx) => rx,
            Err(e) => {
                let error_msg = format!("LLM error: {}", e);
                on_update(StreamUpdate::Error(error_msg.clone()));
                return TurnResult {
                    response: error_msg,
                    tool_calls,
                    cancelled: false,
                    usage: accumulated_usage,
                };
            }
        };

        let (stream_result, turn_usage) = process_stream(rx, on_update).await;

        // Accumulate usage across multi-step tool calls
        if let Some(u) = turn_usage {
            let acc = accumulated_usage.get_or_insert(crate::llm::LlmUsage {
                prompt_tokens: Some(0),
                completion_tokens: Some(0),
            });
            acc.prompt_tokens = Some(acc.prompt_tokens.unwrap_or(0) + u.prompt_tokens.unwrap_or(0));
            acc.completion_tokens =
                Some(acc.completion_tokens.unwrap_or(0) + u.completion_tokens.unwrap_or(0));
        }

        match stream_result {
            StreamResult::TextResponse(text) => {
                conversation.add_assistant_message(&text);
                on_update(StreamUpdate::Done);
                return TurnResult {
                    response: text,
                    tool_calls,
                    cancelled: false,
                    usage: accumulated_usage,
                };
            }
            StreamResult::ToolCall {
                id,
                name,
                arguments,
            } => {
                if total_tool_calls >= MAX_TOOL_CALLS_PER_TURN {
                    let msg = format!(
                        "Reached tool call limit ({}). Stopping here.",
                        MAX_TOOL_CALLS_PER_TURN
                    );
                    conversation.add_assistant_message(&msg);
                    on_update(StreamUpdate::Done);
                    return TurnResult {
                        response: msg,
                        tool_calls,
                        cancelled: false,
                        usage: accumulated_usage,
                    };
                }

                let args: Value = serde_json::from_str(&arguments).unwrap_or(json!({}));
                on_update(StreamUpdate::ToolCallStart {
                    name: name.clone(),
                    args: args.clone(),
                });

                let result = tools::execute_tool_with_config(
                    &name,
                    &args,
                    storage,
                    executor,
                    Some(llm_config),
                )
                .await;
                on_update(StreamUpdate::ToolCallResult {
                    name: name.clone(),
                    result: result.clone(),
                });

                conversation.add_tool_call(&id, &name, &args);
                conversation.add_tool_result(&id, &result);

                tool_calls.push(ToolCallRecord { name, args, result });
                total_tool_calls += 1;
            }
            StreamResult::TextToolCall { name, arguments } => {
                if total_tool_calls >= MAX_TOOL_CALLS_PER_TURN {
                    let msg = format!("Reached tool call limit ({}).", MAX_TOOL_CALLS_PER_TURN);
                    conversation.add_assistant_message(&msg);
                    on_update(StreamUpdate::Done);
                    return TurnResult {
                        response: msg,
                        tool_calls,
                        cancelled: false,
                        usage: accumulated_usage,
                    };
                }

                let args = arguments;
                let call_id = format!("text_{}", total_tool_calls);
                on_update(StreamUpdate::ToolCallStart {
                    name: name.clone(),
                    args: args.clone(),
                });

                let result = tools::execute_tool_with_config(
                    &name,
                    &args,
                    storage,
                    executor,
                    Some(llm_config),
                )
                .await;
                on_update(StreamUpdate::ToolCallResult {
                    name: name.clone(),
                    result: result.clone(),
                });

                conversation.add_tool_call(&call_id, &name, &args);
                conversation.add_tool_result(&call_id, &result);

                tool_calls.push(ToolCallRecord { name, args, result });
                total_tool_calls += 1;
            }
            StreamResult::Error(e) => {
                on_update(StreamUpdate::Error(e.clone()));
                return TurnResult {
                    response: e,
                    tool_calls,
                    cancelled: false,
                    usage: accumulated_usage,
                };
            }
        }
    }
}

enum StreamResult {
    TextResponse(String),
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    TextToolCall {
        name: String,
        arguments: Value,
    },
    Error(String),
}

async fn process_stream(
    mut rx: mpsc::Receiver<StreamEvent>,
    on_update: &StreamCallback,
) -> (StreamResult, Option<crate::llm::LlmUsage>) {
    let mut text_buffer = String::new();
    let mut tool_call_id = String::new();
    let mut tool_call_name = String::new();
    let mut tool_call_args = String::new();
    let mut in_tool_call = false;
    let mut usage: Option<crate::llm::LlmUsage> = None;

    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::TextDelta(delta) => {
                if !in_tool_call {
                    on_update(StreamUpdate::Token(delta.clone()));
                    text_buffer.push_str(&delta);
                }
            }
            StreamEvent::ToolCallStart { id, name } => {
                in_tool_call = true;
                tool_call_id = id;
                tool_call_name = name;
                tool_call_args.clear();
            }
            StreamEvent::ToolCallDelta { arguments, .. } => {
                tool_call_args.push_str(&arguments);
            }
            StreamEvent::Usage(u) => {
                usage = Some(u);
            }
            StreamEvent::Done => break,
        }
    }

    if in_tool_call && !tool_call_name.is_empty() {
        return (
            StreamResult::ToolCall {
                id: tool_call_id,
                name: tool_call_name,
                arguments: tool_call_args,
            },
            usage,
        );
    }

    if let Some((name, args)) = tools::parse_text_tool_call(&text_buffer) {
        return (
            StreamResult::TextToolCall {
                name,
                arguments: args,
            },
            usage,
        );
    }

    if text_buffer.is_empty() {
        (
            StreamResult::Error("Empty response from LLM".to_string()),
            usage,
        )
    } else {
        (StreamResult::TextResponse(text_buffer), usage)
    }
}
