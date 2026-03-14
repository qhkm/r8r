/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! REPL TUI layout and event loop.

use crate::engine::Executor;
use crate::llm::{self, LlmConfig, LlmProvider};
use crate::repl::conversation::Conversation;
use crate::repl::engine::{self, StreamUpdate, TurnResult};
use crate::repl::input::{parse_input, slash_commands, InputCommand};
use crate::repl::tools;
use crate::storage::{ExecutionTrace, ReplMessage, Storage};
use crate::workflow::parse_workflow;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Terminal;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

fn estimate_cost(
    model: &str,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
) -> Option<f64> {
    let p = prompt_tokens? as f64;
    let c = completion_tokens? as f64;
    // Rates per 1M tokens (input, output)
    let (input_rate, output_rate) = match model {
        m if m.contains("gpt-4o-mini") => (0.15, 0.60),
        m if m.contains("gpt-4o") => (2.50, 10.00),
        m if m.contains("claude-sonnet") => (3.00, 15.00),
        m if m.contains("claude-haiku") => (0.80, 4.00),
        m if m.contains("claude-opus") => (15.00, 75.00),
        _ => return None, // Unknown model (e.g., Ollama local)
    };
    Some((p * input_rate + c * output_rate) / 1_000_000.0)
}

fn format_usage_line(usage: &crate::llm::LlmUsage, model: &str) -> Option<String> {
    let prompt = usage.prompt_tokens;
    let completion = usage.completion_tokens;
    if prompt.is_none() && completion.is_none() {
        return None;
    }
    let p = prompt.map(|v| format!("{}", v)).unwrap_or("?".into());
    let c = completion.map(|v| format!("{}", v)).unwrap_or("?".into());
    let cost = estimate_cost(model, prompt, completion);
    let cost_str = cost
        .map(|c| format!(" \u{00b7} ~${:.3}", c))
        .unwrap_or_default();
    Some(format!(
        "[{} in \u{00b7} {} out{} \u{00b7} {}]",
        p, c, cost_str, model
    ))
}

const MAX_CONTEXT_MESSAGES: usize = 80;
const CTRL_C_EXIT_WINDOW: Duration = Duration::from_millis(1500);

/// Walks a JSON value and replaces values whose keys match common credential
/// patterns with `"[REDACTED]"`. Works recursively for nested objects.
pub fn redact_credentials(v: &serde_json::Value) -> serde_json::Value {
    const SENSITIVE: &[&str] = &[
        "password",
        "token",
        "secret",
        "api_key",
        "apikey",
        "authorization",
        "credential",
        "private_key",
        "access_key",
        "secret_key",
    ];

    match v {
        serde_json::Value::Object(map) => {
            let redacted: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, val)| {
                    let lower = k.to_lowercase();
                    if SENSITIVE.iter().any(|s| lower.contains(s)) {
                        (
                            k.clone(),
                            serde_json::Value::String("[REDACTED]".to_string()),
                        )
                    } else {
                        (k.clone(), redact_credentials(val))
                    }
                })
                .collect();
            serde_json::Value::Object(redacted)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(redact_credentials).collect())
        }
        other => other.clone(),
    }
}

/// A single quick-action option in the contextual action bar.
#[derive(Clone)]
struct QuickAction {
    label: String,
    /// Either a slash command (e.g. "/view yaml") or natural language prompt for AI.
    command: QuickActionCommand,
}

#[derive(Clone)]
enum QuickActionCommand {
    Slash(String),
    Prompt(String),
}

/// Active quick-action bar state.
#[derive(Clone)]
struct QuickActionBar {
    actions: Vec<QuickAction>,
    selected: usize,
}

impl QuickActionBar {
    fn new(actions: Vec<QuickAction>) -> Self {
        Self {
            actions,
            selected: 0,
        }
    }

    fn move_left(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        } else {
            self.selected = self.actions.len().saturating_sub(1);
        }
    }

    fn move_right(&mut self) {
        if self.selected + 1 < self.actions.len() {
            self.selected += 1;
        } else {
            self.selected = 0;
        }
    }

    fn selected_action(&self) -> Option<&QuickAction> {
        self.actions.get(self.selected)
    }
}

fn actions_workflow_valid(name: &str) -> Vec<QuickAction> {
    vec![
        QuickAction {
            label: "View YAML".to_string(),
            command: QuickActionCommand::Slash("/view yaml".to_string()),
        },
        QuickAction {
            label: "Run".to_string(),
            command: QuickActionCommand::Slash(format!("/run {}", name)),
        },
        QuickAction {
            label: "Improve".to_string(),
            command: QuickActionCommand::Prompt(
                "Improve this workflow - add better error handling, retry logic, or optimize the node structure".to_string(),
            ),
        },
        QuickAction {
            label: "Export".to_string(),
            command: QuickActionCommand::Slash(format!("/export yaml ./workflows/{}.yaml", name)),
        },
    ]
}

fn actions_workflow_invalid(name: &str) -> Vec<QuickAction> {
    vec![
        QuickAction {
            label: "Fix errors".to_string(),
            command: QuickActionCommand::Prompt(
                "Fix the validation errors in this workflow and regenerate it".to_string(),
            ),
        },
        QuickAction {
            label: "View YAML".to_string(),
            command: QuickActionCommand::Slash("/view yaml".to_string()),
        },
        QuickAction {
            label: "Regenerate".to_string(),
            command: QuickActionCommand::Prompt(format!(
                "Regenerate the workflow '{}' from scratch with a corrected approach",
                name
            )),
        },
    ]
}

fn actions_workflow_saved(name: &str) -> Vec<QuickAction> {
    vec![
        QuickAction {
            label: "Run".to_string(),
            command: QuickActionCommand::Slash(format!("/run {}", name)),
        },
        QuickAction {
            label: "View YAML".to_string(),
            command: QuickActionCommand::Slash("/view yaml".to_string()),
        },
        QuickAction {
            label: "View DAG".to_string(),
            command: QuickActionCommand::Slash(format!("/dag {}", name)),
        },
        QuickAction {
            label: "Export".to_string(),
            command: QuickActionCommand::Slash(format!("/export yaml ./workflows/{}.yaml", name)),
        },
    ]
}

fn actions_execution_done(workflow: &str, execution_id: &str) -> Vec<QuickAction> {
    vec![
        QuickAction {
            label: "View trace".to_string(),
            command: QuickActionCommand::Slash(format!("/trace {}", execution_id)),
        },
        QuickAction {
            label: "Re-run".to_string(),
            command: QuickActionCommand::Slash(format!("/run {}", workflow)),
        },
        QuickAction {
            label: "View DAG".to_string(),
            command: QuickActionCommand::Slash(format!("/dag {}", workflow)),
        },
    ]
}

#[derive(Clone, Copy)]
enum MessageKind {
    User,
    Assistant,
    Tool,
    System,
    Error,
}

struct DisplayMessage {
    kind: MessageKind,
    text: String,
}

#[derive(Default, Clone)]
struct TurnOutcome {
    generated_name: Option<String>,
    generated_valid: Option<bool>,
    generated_nodes: Option<usize>,
    validated: Option<bool>,
    saved_name: Option<String>,
    tool_durations: Vec<String>,
}

#[derive(Clone, Copy)]
enum ContextMode {
    Dag,
    Yaml,
    Trace,
}

impl ContextMode {
    fn title(self) -> &'static str {
        match self {
            Self::Dag => "Context/DAG",
            Self::Yaml => "Context/YAML",
            Self::Trace => "Context/Trace",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Dag => Self::Yaml,
            Self::Yaml => Self::Trace,
            Self::Trace => Self::Dag,
        }
    }
}

#[derive(Clone, Copy)]
enum InspectorTab {
    Log,
    Context,
    Tools,
}

impl InspectorTab {
    fn title(self) -> &'static str {
        match self {
            Self::Log => "Inspector/Log",
            Self::Context => "Inspector/Context",
            Self::Tools => "Inspector/Tools",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Log => Self::Context,
            Self::Context => Self::Tools,
            Self::Tools => Self::Log,
        }
    }
}

/// Write-capable tool names. When writes are disarmed, these trigger a confirmation.
#[allow(dead_code)]
const WRITE_TOOLS: &[&str] = &[
    "r8r_create_workflow",
    "r8r_save_workflow",
    "r8r_delete_workflow",
    "r8r_execute",
    "r8r_run_and_wait",
    "r8r_approve",
];

/// A pending write action awaiting user confirmation.
#[allow(dead_code)]
struct PendingWriteConfirm {
    /// Friendly description of what will happen.
    summary: String,
    /// The natural-language input that triggered it.
    original_input: String,
}

/// REPL UI state.
/// Accumulated token usage for the current REPL session.
#[derive(Debug, Default)]
pub struct SessionUsage {
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub persisted_total_tokens: u64,
    pub turns_with_usage: u32,
    pub turns_without_usage: u32,
}

pub struct ReplApp {
    pub session_id: String,
    /// Most recent execution_id returned by a tool call in this session.
    pub last_run_id: Option<String>,
    pub model: String,
    pub workflow_count: usize,
    pub input: String,
    pub show_context: bool,
    pub should_quit: bool,
    pub busy: bool,
    pub conversation: Conversation,
    messages: Vec<DisplayMessage>,
    log_lines: Vec<String>,
    tool_lines: Vec<String>,
    dag_lines: Vec<String>,
    yaml_lines: Vec<String>,
    trace_lines: Vec<String>,
    context_mode: ContextMode,
    inspector_tab: InspectorTab,
    assistant_streaming_index: Option<usize>,
    spinner_index: usize,
    last_ctrl_c: Option<Instant>,
    autocomplete: Vec<(String, String)>,
    autocomplete_index: usize,
    conversation_scroll: usize,
    turn_outcome: TurnOutcome,
    tool_start_times: HashMap<String, Instant>,
    busy_since: Option<Instant>,
    inspector_scroll: usize,
    quick_actions: Option<QuickActionBar>,
    /// Saved action bar to restore after view-only commands (View YAML, View DAG, etc.)
    sticky_actions: Option<QuickActionBar>,
    /// Guardrails: whether write-capable tool calls are permitted this session.
    /// Default: false (disarmed — safe mode). Set via /arm.
    pub turn_usage: Option<crate::llm::LlmUsage>,
    pub session_usage: SessionUsage,
    pub writes_armed: bool,
    /// Guardrails: operator mode shows policy/gate status in the status bar.
    pub operator_mode: bool,
    /// Guardrails: pending confirmation for a write-capable action.
    /// When Some, the next Enter either confirms or the user can type 'n' to cancel.
    pending_write_confirm: Option<PendingWriteConfirm>,
}

impl ReplApp {
    pub fn new(
        session_id: String,
        model: String,
        conversation: Conversation,
        workflow_count: usize,
    ) -> Self {
        Self {
            session_id,
            model,
            workflow_count,
            input: String::new(),
            show_context: true,
            should_quit: false,
            busy: false,
            conversation,
            messages: Vec::new(),
            log_lines: vec!["Ready.".to_string()],
            tool_lines: vec!["No tool calls yet.".to_string()],
            dag_lines: vec!["No DAG loaded.".to_string()],
            yaml_lines: vec!["No YAML loaded.".to_string()],
            trace_lines: vec!["No trace loaded.".to_string()],
            context_mode: ContextMode::Dag,
            inspector_tab: InspectorTab::Log,
            assistant_streaming_index: None,
            spinner_index: 0,
            last_ctrl_c: None,
            autocomplete: Vec::new(),
            autocomplete_index: 0,
            conversation_scroll: 0,
            turn_outcome: TurnOutcome::default(),
            tool_start_times: HashMap::new(),
            busy_since: None,
            inspector_scroll: 0,
            quick_actions: None,
            sticky_actions: None,
            turn_usage: None,
            session_usage: SessionUsage::default(),
            writes_armed: false,
            operator_mode: false,
            pending_write_confirm: None,
            last_run_id: None,
        }
    }

    fn push_message(&mut self, kind: MessageKind, text: impl Into<String>) {
        self.messages.push(DisplayMessage {
            kind,
            text: text.into(),
        });
    }

    fn append_stream_token(&mut self, token: &str) {
        if let Some(idx) = self.assistant_streaming_index {
            if let Some(msg) = self.messages.get_mut(idx) {
                msg.text.push_str(token);
                return;
            }
        }

        self.messages.push(DisplayMessage {
            kind: MessageKind::Assistant,
            text: token.to_string(),
        });
        self.assistant_streaming_index = Some(self.messages.len() - 1);
    }

    fn push_log(&mut self, line: impl Into<String>) {
        self.log_lines.push(line.into());
        if self.log_lines.len() > 300 {
            let drain = self.log_lines.len() - 300;
            self.log_lines.drain(0..drain);
        }
    }

    fn push_tool_line(&mut self, line: impl Into<String>) {
        if self.tool_lines.len() == 1 && self.tool_lines[0] == "No tool calls yet." {
            self.tool_lines.clear();
        }
        self.tool_lines.push(line.into());
        if self.tool_lines.len() > 200 {
            let drain = self.tool_lines.len() - 200;
            self.tool_lines.drain(0..drain);
        }
    }

    fn active_inspector_lines(&self) -> &[String] {
        match self.inspector_tab {
            InspectorTab::Log => &self.log_lines,
            InspectorTab::Context => match self.context_mode {
                ContextMode::Dag => &self.dag_lines,
                ContextMode::Yaml => &self.yaml_lines,
                ContextMode::Trace => &self.trace_lines,
            },
            InspectorTab::Tools => &self.tool_lines,
        }
    }

    fn refresh_autocomplete(&mut self) {
        let old_selected = self
            .autocomplete
            .get(self.autocomplete_index)
            .map(|(cmd, _)| cmd.clone())
            .unwrap_or_default();
        self.autocomplete.clear();
        self.autocomplete_index = 0;

        if self.busy {
            return;
        }

        let prefix = self.input.trim();
        if !prefix.starts_with('/') || prefix.contains(' ') {
            return;
        }

        let mut matches: Vec<(String, String)> = slash_commands()
            .into_iter()
            .map(|(cmd, desc)| {
                (
                    cmd.split_whitespace().next().unwrap_or(cmd).to_string(),
                    desc.to_string(),
                )
            })
            .filter(|(cmd, _)| cmd.starts_with(prefix))
            .collect();
        matches.sort_by(|a, b| a.0.cmp(&b.0));
        matches.dedup_by(|a, b| a.0 == b.0);
        self.autocomplete = matches;

        if !old_selected.is_empty() {
            if let Some(pos) = self
                .autocomplete
                .iter()
                .position(|(cmd, _)| cmd == &old_selected)
            {
                self.autocomplete_index = pos;
            }
        }
    }

    fn move_autocomplete(&mut self, delta: isize) -> bool {
        if self.autocomplete.is_empty() {
            return false;
        }
        let len = self.autocomplete.len() as isize;
        let mut idx = self.autocomplete_index as isize + delta;
        while idx < 0 {
            idx += len;
        }
        while idx >= len {
            idx -= len;
        }
        self.autocomplete_index = idx as usize;
        true
    }

    fn apply_autocomplete(&mut self) -> bool {
        if self.autocomplete.is_empty() {
            return false;
        }
        let idx = self.autocomplete_index.min(self.autocomplete.len() - 1);
        let completed = self.autocomplete[idx].0.clone();
        self.input = format!("{} ", completed);
        self.refresh_autocomplete();
        true
    }

    fn scroll_conversation_up(&mut self, lines: usize) {
        self.conversation_scroll = self.conversation_scroll.saturating_add(lines);
    }

    fn scroll_conversation_down(&mut self, lines: usize) {
        self.conversation_scroll = self.conversation_scroll.saturating_sub(lines);
    }

    fn jump_to_latest(&mut self) {
        self.conversation_scroll = 0;
    }

    fn scroll_inspector_up(&mut self, lines: usize) {
        self.inspector_scroll = self.inspector_scroll.saturating_sub(lines);
    }

    fn scroll_inspector_down(&mut self, lines: usize) {
        self.inspector_scroll = self.inspector_scroll.saturating_add(lines);
    }
}

/// Events consumed by the REPL event loop.
pub enum ReplEvent {
    Stream(StreamUpdate),
    TurnComplete {
        conversation: Conversation,
        result: TurnResult,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BusyStage {
    Planning,
    RunningTools,
    Drafting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BusyStateView {
    stage: BusyStage,
    headline: String,
    detail: String,
    support: Option<String>,
}

fn active_busy_tools(app: &ReplApp) -> Vec<String> {
    let mut tools: Vec<String> = app.tool_start_times.keys().cloned().collect();
    tools.sort();
    tools
}

fn busy_progress_text(stage: BusyStage, spinner: &str) -> String {
    let (plan, tools, reply) = match stage {
        BusyStage::Planning => (spinner, "○", "○"),
        BusyStage::RunningTools => ("✓", spinner, "○"),
        BusyStage::Drafting => ("✓", "✓", spinner),
    };
    format!("plan {}  ->  tools {}  ->  reply {}", plan, tools, reply)
}

fn busy_progress_text_for_width(stage: BusyStage, spinner: &str, max_len: usize) -> String {
    let full = busy_progress_text(stage, spinner);
    if full.chars().count() <= max_len {
        return full;
    }

    let stage_label = match stage {
        BusyStage::Planning => "1/3 planning",
        BusyStage::RunningTools => "2/3 tools",
        BusyStage::Drafting => "3/3 reply",
    };
    let compact = format!("{} {}", spinner, stage_label);
    if compact.chars().count() <= max_len {
        compact
    } else {
        truncate_for_width(&compact, max_len)
    }
}

fn busy_cancel_hint(max_len: usize) -> String {
    let full = "Ctrl+C cancels the current turn.";
    if full.chars().count() <= max_len {
        full.to_string()
    } else {
        truncate_for_width("Ctrl+C cancels.", max_len)
    }
}

fn build_busy_state_view(app: &ReplApp) -> BusyStateView {
    let active_tools = active_busy_tools(app);
    let completed_tools = app.turn_outcome.tool_durations.len();
    let latest_completed = app.turn_outcome.tool_durations.last().cloned();

    if app.assistant_streaming_index.is_some() {
        return BusyStateView {
            stage: BusyStage::Drafting,
            headline: "Drafting reply".to_string(),
            detail: if completed_tools > 0 {
                format!(
                    "Turning {} tool result{} into the final response.",
                    completed_tools,
                    if completed_tools == 1 { "" } else { "s" }
                )
            } else {
                "Composing a direct response.".to_string()
            },
            support: latest_completed.map(|entry| format!("Latest finished: {}", entry)),
        };
    }

    if !active_tools.is_empty() {
        let headline = if active_tools.len() == 1 {
            format!("Running {}", active_tools[0])
        } else {
            format!("Running {} tools", active_tools.len())
        };
        let detail = if active_tools.len() == 1 {
            format!("{} is currently in flight.", active_tools[0])
        } else {
            format!("Active tools: {}", active_tools.join(", "))
        };
        return BusyStateView {
            stage: BusyStage::RunningTools,
            headline,
            detail,
            support: latest_completed.map(|entry| format!("Latest finished: {}", entry)),
        };
    }

    let detail = if let Some(last_tool) = latest_completed
        .as_deref()
        .and_then(|entry| entry.split_whitespace().next())
    {
        format!(
            "Reviewing {} and deciding whether to answer directly or call another tool.",
            last_tool
        )
    } else {
        "Reading your prompt and choosing the first action.".to_string()
    };

    BusyStateView {
        stage: BusyStage::Planning,
        headline: "Planning next step".to_string(),
        detail,
        support: latest_completed.map(|entry| format!("Latest finished: {}", entry)),
    }
}

fn build_busy_input_lines(app: &ReplApp, spinner: &str, text_width: usize) -> Vec<Line<'static>> {
    let busy_state = build_busy_state_view(app);
    let elapsed = app
        .busy_since
        .map(|t| format_duration(t.elapsed()))
        .unwrap_or_else(|| "0ms".to_string());
    let headline = format!("{} {} ({})", spinner, busy_state.headline, elapsed);
    let mut lines = Vec::new();

    for line in wrap_and_truncate_for_width(&headline, text_width) {
        lines.push(Line::from(Span::styled(
            line,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
    }
    for line in wrap_and_truncate_for_width(&busy_state.detail, text_width) {
        lines.push(Line::from(Span::styled(
            line,
            Style::default().fg(Color::Gray),
        )));
    }
    for line in wrap_and_truncate_for_width(
        &busy_progress_text_for_width(busy_state.stage, spinner, text_width),
        text_width,
    ) {
        lines.push(Line::from(Span::styled(
            line,
            Style::default().fg(Color::Cyan),
        )));
    }

    let support = busy_state
        .support
        .unwrap_or_else(|| busy_cancel_hint(text_width));
    for line in wrap_and_truncate_for_width(&support, text_width) {
        lines.push(Line::from(Span::styled(
            line,
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines
}

fn parse_markdown_bold_spans(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;
    let mut bold = false;

    while let Some(idx) = rest.find("**") {
        let (head, tail) = rest.split_at(idx);
        if !head.is_empty() {
            if bold {
                spans.push(Span::styled(
                    head.to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw(head.to_string()));
            }
        }
        rest = &tail[2..];
        bold = !bold;
    }

    if !rest.is_empty() {
        if bold {
            let mut text = String::from("**");
            text.push_str(rest);
            spans.push(Span::raw(text));
        } else {
            spans.push(Span::raw(rest.to_string()));
        }
    }

    spans
}

fn parse_markdown_lines(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut in_code_block = false;

    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        out.push((line.to_string(), in_code_block));
    }

    if out.is_empty() {
        out.push((String::new(), false));
    }
    out
}

fn strip_markdown_bold_markers(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(idx) = rest.find("**") {
        let (head, tail) = rest.split_at(idx);
        out.push_str(head);
        rest = &tail[2..];
    }
    out.push_str(rest);
    out
}

fn normalize_markdown_line(text: &str) -> String {
    let trimmed = text.trim_start();
    let without_heading = if let Some(rest) = trimmed.strip_prefix("#### ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("### ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("## ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("# ") {
        rest
    } else {
        trimmed
    };

    let mut out = without_heading.replace("**", "");
    out = out.replace('`', "");
    if let Some(rest) = out.trim_start().strip_prefix("- ") {
        out = format!("• {}", rest);
    }
    out
}

fn extract_fenced_yaml_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut in_yaml = false;
    let mut current = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if !in_yaml && (trimmed.starts_with("```yaml") || trimmed == "``` yaml") {
            in_yaml = true;
            current.clear();
            continue;
        }
        if in_yaml && trimmed.starts_with("```") {
            in_yaml = false;
            if !current.is_empty() {
                blocks.push(current.join("\n"));
            }
            continue;
        }
        if in_yaml {
            current.push(line.to_string());
        }
    }
    blocks
}

fn strip_fenced_yaml_from_text(text: &str) -> String {
    let mut lines = Vec::new();
    let mut in_yaml = false;
    let mut replaced = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if !in_yaml && (trimmed.starts_with("```yaml") || trimmed == "``` yaml") {
            in_yaml = true;
            if !replaced {
                lines.push(
                    "[YAML moved to Inspector/Context. Use /view yaml or Ctrl+Y.]".to_string(),
                );
                replaced = true;
            }
            continue;
        }
        if in_yaml && trimmed.starts_with("```") {
            in_yaml = false;
            continue;
        }
        if !in_yaml {
            lines.push(line.to_string());
        }
    }
    lines.join("\n").trim().to_string()
}

fn compact_generate_tool_result(result: &str) -> Option<(String, Vec<String>)> {
    let v: Value = serde_json::from_str(result).ok()?;
    let yaml = v.get("yaml")?.as_str()?.to_string();
    let valid = v.get("valid").and_then(|x| x.as_bool()).unwrap_or(false);
    let summary = v.get("summary").and_then(|x| x.as_str()).unwrap_or("");
    let name = parse_workflow(&yaml)
        .ok()
        .map(|wf| wf.name)
        .unwrap_or_else(|| {
            yaml.lines()
                .find_map(|line| line.strip_prefix("name:").map(|s| s.trim().to_string()))
                .unwrap_or_else(|| "generated-workflow".to_string())
        });
    let headline = if valid {
        format!("generated {} (valid)", name)
    } else {
        format!("generated {} (has validation issues)", name)
    };
    let summary_preview = one_line_preview(summary, 84);
    let details = if summary.is_empty() {
        vec![
            format!("name: {}", name),
            "use /view yaml to inspect".to_string(),
        ]
    } else {
        vec![
            format!("name: {}", name),
            format!("summary: {}", summary_preview),
            "use /view yaml to inspect".to_string(),
        ]
    };
    let mut yaml_lines: Vec<String> = yaml.lines().map(|s| s.to_string()).collect();
    if yaml_lines.is_empty() {
        yaml_lines.push("No YAML generated.".to_string());
    }
    Some((
        format!("{} | {}", headline, details.join(" | ")),
        yaml_lines,
    ))
}

fn one_line_preview(input: &str, max_len: usize) -> String {
    let single = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if single.chars().count() > max_len {
        format!("{}...", single.chars().take(max_len).collect::<String>())
    } else {
        single
    }
}

fn truncate_for_width(input: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }
    let mut chars = input.chars();
    let truncated: String = chars.by_ref().take(max_len).collect();
    if chars.next().is_some() && max_len > 1 {
        format!(
            "{}…",
            truncated.chars().take(max_len - 1).collect::<String>()
        )
    } else {
        truncated
    }
}

fn wrap_and_truncate_for_width(input: &str, max_len: usize) -> Vec<String> {
    wrap_for_width(input, max_len)
        .into_iter()
        .map(|line| truncate_for_width(&line, max_len))
        .collect()
}

fn wrap_for_width(input: &str, max_len: usize) -> Vec<String> {
    if max_len == 0 {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    for raw_line in input.lines() {
        if raw_line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in raw_line.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{} {}", current, word)
            };
            if candidate.chars().count() <= max_len {
                current = candidate;
            } else {
                if !current.is_empty() {
                    out.push(current);
                }
                // Preserve whole words; if a single token is too long, keep it intact.
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }

    if out.is_empty() {
        out.push(String::new());
    }

    out
}

fn style_tool_wrapped_line(line: &str, default_color: Color) -> Vec<Span<'static>> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let mut spans = Vec::new();

    for (idx, token) in tokens.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::raw(" "));
        }

        let normalized = token
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '_' && c != '-')
            .to_lowercase();

        let style = if *token == "→" {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if token.starts_with('/') {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if normalized == "name" || normalized == "summary" {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if normalized == "valid" || normalized == "created" {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else if normalized == "invalid" || normalized == "error" {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else if idx > 0 && tokens[idx - 1] == "generated" {
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(default_color)
        };

        spans.push(Span::styled((*token).to_string(), style));
    }

    spans
}

fn matches_ctrl_shortcut(key: &KeyEvent, ch: char, ctrl_byte: u8) -> bool {
    (key.code == KeyCode::Char(ch) && key.modifiers.contains(KeyModifiers::CONTROL))
        || matches!(key.code, KeyCode::Char(raw) if raw == ctrl_byte as char)
}

fn build_titled_top_border(inner_width: usize, title: &str) -> String {
    let mut label = format!(" {} ", title);
    if label.len() > inner_width {
        label = truncate_for_width(&label, inner_width);
    }
    let pad = inner_width.saturating_sub(label.len());
    let left = pad / 2;
    let right = pad.saturating_sub(left);
    format!("╭{}{}{}╮", "─".repeat(left), label, "─".repeat(right))
}

fn build_bottom_border(inner_width: usize) -> String {
    format!("╰{}╯", "─".repeat(inner_width))
}

fn format_duration(d: Duration) -> String {
    if d.as_secs() >= 60 {
        let mins = d.as_secs() / 60;
        let secs = d.as_secs() % 60;
        format!("{}m{}s", mins, secs)
    } else if d.as_secs() >= 1 {
        format!("{:.1}s", d.as_secs_f64())
    } else {
        format!("{}ms", d.as_millis())
    }
}

fn style_inspector_line(line: &str) -> Line<'static> {
    let text = line.to_string();
    if text.starts_with('✓') {
        Line::from(Span::styled(text, Style::default().fg(Color::Green)))
    } else if text.starts_with('!') {
        Line::from(Span::styled(text, Style::default().fg(Color::Red)))
    } else if text.starts_with('…') {
        Line::from(Span::styled(text, Style::default().fg(Color::Yellow)))
    } else if text.starts_with("Calling LLM") {
        Line::from(Span::styled(text, Style::default().fg(Color::Cyan)))
    } else {
        Line::from(Span::styled(text, Style::default().fg(Color::Gray)))
    }
}

fn style_yaml_value(value: &str) -> Style {
    let v = value.trim_start();
    if v.starts_with('#') {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC)
    } else if v.starts_with('"') || v.starts_with('\'') {
        Style::default().fg(Color::Green)
    } else if matches!(v, "true" | "false" | "null") || v.parse::<f64>().is_ok() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    }
}

fn style_yaml_inspector_line(line: &str) -> Line<'static> {
    let indent_len = line.chars().take_while(|c| c.is_whitespace()).count();
    let indent = " ".repeat(indent_len);
    let trimmed = line.trim_start();

    if trimmed.starts_with('#') {
        return Line::from(vec![
            Span::raw(indent),
            Span::styled(
                trimmed.to_string(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]);
    }

    if let Some(rest) = trimmed.strip_prefix("- ") {
        if let Some((key, value)) = rest.split_once(':') {
            return Line::from(vec![
                Span::raw(indent),
                Span::styled("- ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    key.trim().to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(":", Style::default().fg(Color::Gray)),
                Span::styled(value.to_string(), style_yaml_value(value)),
            ]);
        }
        return Line::from(vec![
            Span::raw(indent),
            Span::styled("- ", Style::default().fg(Color::Yellow)),
            Span::styled(rest.to_string(), Style::default().fg(Color::White)),
        ]);
    }

    if let Some((key, value)) = trimmed.split_once(':') {
        return Line::from(vec![
            Span::raw(indent),
            Span::styled(
                key.trim().to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":", Style::default().fg(Color::Gray)),
            Span::styled(value.to_string(), style_yaml_value(value)),
        ]);
    }

    Line::from(Span::styled(
        line.to_string(),
        Style::default().fg(Color::Gray),
    ))
}

fn compact_tool_start(name: &str, args: &Value) -> String {
    let obj = match args.as_object() {
        Some(o) => o,
        None => return format!("start {}", name),
    };

    let mut parts = Vec::new();
    if let Some(workflow) = obj.get("workflow").and_then(|v| v.as_str()) {
        parts.push(format!("workflow={}", workflow));
    }
    if let Some(name_arg) = obj.get("name").and_then(|v| v.as_str()) {
        parts.push(format!("name={}", name_arg));
    }
    if let Some(yaml) = obj
        .get("yaml")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("workflow_yaml").and_then(|v| v.as_str()))
    {
        parts.push(format!("yaml={} lines", yaml.lines().count()));
    }
    if let Some(prompt) = obj.get("prompt").and_then(|v| v.as_str()) {
        parts.push(format!("prompt={}", one_line_preview(prompt, 80)));
    }

    if parts.is_empty() {
        format!("start {}", name)
    } else {
        format!("start {} ({})", name, parts.join(", "))
    }
}

fn compact_tool_result(name: &str, result: &str) -> String {
    let parsed: Value = match serde_json::from_str(result) {
        Ok(v) => v,
        Err(_) => return format!("result {} {}", name, one_line_preview(result, 120)),
    };

    if let Some(err) = parsed.get("error").and_then(|v| v.as_str()) {
        return format!("result {} error={}", name, one_line_preview(err, 120));
    }

    match name {
        "r8r_validate" => {
            let valid = parsed
                .get("valid")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let wf_name = parsed
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let errors = parsed
                .get("errors")
                .and_then(|v| v.as_array())
                .map(|v| v.len())
                .unwrap_or(0);
            if valid {
                format!("result {} valid name={}", name, wf_name)
            } else {
                format!("result {} invalid errors={}", name, errors)
            }
        }
        "r8r_create_workflow" => {
            let created = parsed
                .get("created")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let wf_name = parsed
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            if created {
                format!("result {} created name={}", name, wf_name)
            } else {
                format!("result {} not-created", name)
            }
        }
        "r8r_list_workflows" => {
            let count = parsed.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("result {} count={}", name, count)
        }
        _ => {
            let keys = parsed
                .as_object()
                .map(|o| o.keys().cloned().collect::<Vec<_>>().join(","))
                .unwrap_or_else(|| "value".to_string());
            format!("result {} ok keys={}", name, keys)
        }
    }
}

fn parse_generate_result(result: &str) -> Option<(String, bool, usize)> {
    let parsed: Value = serde_json::from_str(result).ok()?;
    let yaml = parsed.get("yaml")?.as_str()?;
    let valid = parsed
        .get("valid")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let wf = parse_workflow(yaml).ok()?;
    Some((wf.name, valid, wf.nodes.len()))
}

fn parse_validate_result(result: &str) -> Option<bool> {
    let parsed: Value = serde_json::from_str(result).ok()?;
    parsed.get("valid").and_then(|v| v.as_bool())
}

fn parse_create_result(result: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(result).ok()?;
    let created = parsed
        .get("created")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !created {
        return None;
    }
    parsed
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn build_result_card(app: &ReplApp) -> Option<String> {
    let workflow_name = app
        .turn_outcome
        .saved_name
        .clone()
        .or_else(|| app.turn_outcome.generated_name.clone())?;

    let status = app
        .turn_outcome
        .validated
        .or(app.turn_outcome.generated_valid)
        .map(|valid| if valid { "✓ valid" } else { "✗ invalid" })
        .unwrap_or("○ unknown");

    let nodes = app.turn_outcome.generated_nodes.or_else(|| {
        parse_workflow(&app.yaml_lines.join("\n"))
            .ok()
            .map(|wf| wf.nodes.len())
    });

    let saved_icon = if app.turn_outcome.saved_name.is_some() {
        "✓ yes"
    } else {
        "○ no"
    };

    let mut lines = vec![
        "Result".to_string(),
        format!("Workflow:  {}", workflow_name),
        format!("Status:    {}", status),
    ];
    if let Some(node_count) = nodes {
        lines.push(format!("Nodes:     {}", node_count));
    }
    lines.push(format!("Saved:     {}", saved_icon));
    lines.push(String::new());
    lines.push("Actions:".to_string());
    lines.push("  /view yaml".to_string());
    lines.push(format!("  /export yaml ./workflows/{}.yaml", workflow_name));
    lines.push(format!("  /run {}", workflow_name));
    if !app.turn_outcome.tool_durations.is_empty() {
        lines.push(String::new());
        lines.push("Durations:".to_string());
        for (idx, d) in app.turn_outcome.tool_durations.iter().enumerate() {
            lines.push(format!("  {}. {}", idx + 1, d));
        }
    }
    Some(lines.join("\n"))
}

fn build_quick_actions(outcome: &TurnOutcome) -> Option<QuickActionBar> {
    if let Some(name) = &outcome.saved_name {
        return Some(QuickActionBar::new(actions_workflow_saved(name)));
    }
    if let Some(name) = &outcome.generated_name {
        let valid = outcome
            .validated
            .or(outcome.generated_valid)
            .unwrap_or(false);
        return if valid {
            Some(QuickActionBar::new(actions_workflow_valid(name)))
        } else {
            Some(QuickActionBar::new(actions_workflow_invalid(name)))
        };
    }
    None
}

/// Render the REPL layout.
pub fn render(frame: &mut ratatui::Frame<'_>, app: &ReplApp) {
    let suggestion_rows = app.autocomplete.len().min(8);
    let input_height: u16 = if suggestion_rows > 0 {
        // Borders + input row + heading + suggestion rows
        (2 + 1 + 1 + suggestion_rows) as u16
    } else if app.busy {
        6
    } else {
        3
    };

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(input_height),
        ])
        .split(frame.area());

    let spinner = ["◐", "◓", "◑", "◒"][app.spinner_index % 4];
    let short_session = if app.session_id.len() > 8 {
        &app.session_id[..8]
    } else {
        &app.session_id
    };
    let model_display = if app.model == "unknown" {
        "not set".to_string()
    } else {
        app.model.clone()
    };
    let (status_text, status_style) = if app.busy {
        let busy_state = build_busy_state_view(app);
        let elapsed = app
            .busy_since
            .map(|t| format_duration(t.elapsed()))
            .unwrap_or_else(|| "0s".to_string());
        (
            format!(
                "{} {} {}",
                spinner,
                busy_state.headline.to_lowercase(),
                elapsed
            ),
            Style::default().fg(Color::Yellow),
        )
    } else {
        ("ready".to_string(), Style::default().fg(Color::Green))
    };
    let model_style = if app.model == "unknown" {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    };
    let status_help = if root[0].width < 120 {
        "  Tab·Ctrl+Y·Ctrl+↑↓·Ctrl+C"
    } else {
        "  Tab:panel  Ctrl+Y:yaml  Ctrl+I:toggle  ↑↓:scroll  Ctrl+↑↓:inspector  Ctrl+C:cancel"
    };
    let (arm_text, arm_style) = if app.writes_armed {
        (
            " ⚡armed",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (" 🔒safe", Style::default().fg(Color::Green))
    };
    let operator_spans: Vec<Span<'_>> = if app.operator_mode {
        vec![
            Span::styled(" · ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "OPERATOR",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]
    } else {
        vec![]
    };
    let mut status_spans = vec![
        Span::styled(
            " r8r chat ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::styled(" ", Style::default()),
        Span::styled(model_display, model_style),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("#{}", short_session),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} workflows", app.workflow_count),
            Style::default().fg(Color::White),
        ),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(status_text, status_style),
        Span::styled(arm_text, arm_style),
    ];
    status_spans.extend(operator_spans);
    status_spans.push(Span::styled(
        status_help,
        Style::default().fg(Color::DarkGray),
    ));
    let status = Paragraph::new(Line::from(status_spans));
    frame.render_widget(status, root[0]);

    let main_chunks = if app.show_context && root[1].width >= 80 {
        let left_pct = if root[1].width < 130 {
            60
        } else if app.busy {
            72
        } else {
            66
        };
        let right_pct = 100 - left_pct;
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(left_pct),
                Constraint::Percentage(right_pct),
            ])
            .split(root[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100), Constraint::Length(0)])
            .split(root[1])
    };

    let mut conversation_lines: Vec<Line<'_>> = Vec::new();
    let card_width = main_chunks[0].width.saturating_sub(8) as usize;
    let card_inner_width = card_width.saturating_sub(2);
    let card_text_width = card_inner_width.saturating_sub(2);

    for (msg_idx, m) in app.messages.iter().enumerate() {
        if matches!(m.kind, MessageKind::User) && msg_idx > 0 {
            let separator_width = card_width.saturating_sub(2);
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "·".repeat(separator_width.min(60)),
                    Style::default().fg(Color::Rgb(60, 66, 76)),
                ),
            ]));
            conversation_lines.push(Line::from(""));
        }
        let (label, badge_style, rail_style) = match m.kind {
            MessageKind::User => (
                "YOU",
                Style::default().fg(Color::Black).bg(Color::Cyan),
                Style::default().fg(Color::Cyan),
            ),
            MessageKind::Assistant => (
                "AI",
                Style::default().fg(Color::Black).bg(Color::Green),
                Style::default().fg(Color::Green),
            ),
            MessageKind::Tool => (
                "TOOL",
                Style::default().fg(Color::Black).bg(Color::Yellow),
                Style::default().fg(Color::Yellow),
            ),
            MessageKind::System => (
                "SYS",
                Style::default().fg(Color::Black).bg(Color::Magenta),
                Style::default().fg(Color::Magenta),
            ),
            MessageKind::Error => (
                "ERR",
                Style::default().fg(Color::White).bg(Color::Red),
                Style::default().fg(Color::Red),
            ),
        };
        let badge = format!(" {:<4} ", label);
        conversation_lines.push(Line::from(vec![
            Span::styled(badge, badge_style),
            Span::styled(" ", Style::default().fg(Color::DarkGray)),
        ]));
        if matches!(m.kind, MessageKind::System) && m.text.starts_with("Result\n") {
            let top = build_titled_top_border(card_inner_width, "Result");
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(top, Style::default().fg(Color::Magenta)),
            ]));
            for line in m.text.lines().skip(1) {
                let content = truncate_for_width(line, card_text_width);
                let padded = format!("{:<width$}", content, width = card_text_width);
                if line.starts_with("  /") {
                    conversation_lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("│ ", Style::default().fg(Color::Magenta)),
                        Span::styled(padded, Style::default().fg(Color::Cyan)),
                        Span::styled(" │", Style::default().fg(Color::Magenta)),
                    ]));
                } else if line.contains("✓") {
                    conversation_lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("│ ", Style::default().fg(Color::Magenta)),
                        Span::styled(padded, Style::default().fg(Color::Green)),
                        Span::styled(" │", Style::default().fg(Color::Magenta)),
                    ]));
                } else if line.contains("✗") {
                    conversation_lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("│ ", Style::default().fg(Color::Magenta)),
                        Span::styled(padded, Style::default().fg(Color::Red)),
                        Span::styled(" │", Style::default().fg(Color::Magenta)),
                    ]));
                } else if let Some((label, value)) = line.split_once(':') {
                    let label_str = format!("{}:", label);
                    let value_padded = format!(
                        "{:<width$}",
                        value,
                        width = card_text_width.saturating_sub(label_str.len())
                    );
                    conversation_lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("│ ", Style::default().fg(Color::Magenta)),
                        Span::styled(label_str, Style::default().fg(Color::DarkGray)),
                        Span::styled(value_padded, Style::default().fg(Color::White)),
                        Span::styled(" │", Style::default().fg(Color::Magenta)),
                    ]));
                } else {
                    conversation_lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("│ ", Style::default().fg(Color::Magenta)),
                        Span::styled(padded, Style::default()),
                        Span::styled(" │", Style::default().fg(Color::Magenta)),
                    ]));
                }
            }
            let bottom = build_bottom_border(card_inner_width);
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(bottom, Style::default().fg(Color::Magenta)),
            ]));
            conversation_lines.push(Line::from(""));
            continue;
        }

        if matches!(m.kind, MessageKind::Assistant) {
            let card_border = Color::Rgb(125, 160, 125);
            let card_text = Color::Rgb(232, 238, 232);
            let top = build_titled_top_border(card_inner_width, "AI Response");
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(top, Style::default().fg(card_border)),
            ]));
            let empty = format!("{:<width$}", "", width = card_text_width);
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("│ ", Style::default().fg(card_border)),
                Span::styled(empty.clone(), Style::default().fg(card_text)),
                Span::styled(" │", Style::default().fg(card_border)),
            ]));
            for (line, is_code) in parse_markdown_lines(&m.text) {
                let cleaned = if is_code {
                    line
                } else {
                    normalize_markdown_line(&strip_markdown_bold_markers(&line))
                };
                for wrapped in wrap_for_width(&cleaned, card_text_width) {
                    let padded = format!("{:<width$}", wrapped, width = card_text_width);
                    let content_style = if is_code {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default().fg(card_text)
                    };
                    conversation_lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("│ ", Style::default().fg(card_border)),
                        Span::styled(padded, content_style),
                        Span::styled(" │", Style::default().fg(card_border)),
                    ]));
                }
            }
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("│ ", Style::default().fg(card_border)),
                Span::styled(empty, Style::default().fg(card_text)),
                Span::styled(" │", Style::default().fg(card_border)),
            ]));
            let bottom = build_bottom_border(card_inner_width);
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(bottom, Style::default().fg(card_border)),
            ]));
            conversation_lines.push(Line::from(""));
            continue;
        }

        if matches!(m.kind, MessageKind::Tool) {
            let card_border = Color::Rgb(196, 162, 84);
            let card_text = Color::Rgb(243, 236, 214);
            let top = build_titled_top_border(card_inner_width, "Tool Output");
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(top, Style::default().fg(card_border)),
            ]));
            let empty = format!("{:<width$}", "", width = card_text_width);
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("│ ", Style::default().fg(card_border)),
                Span::styled(empty.clone(), Style::default().fg(card_text)),
                Span::styled(" │", Style::default().fg(card_border)),
            ]));
            for line in m.text.lines() {
                for wrapped in wrap_for_width(line, card_text_width) {
                    let lowered = wrapped.to_lowercase();
                    let base_color = if lowered.contains("error") || lowered.contains("invalid") {
                        Color::Red
                    } else if lowered.contains("(valid)") || lowered.contains("created") {
                        Color::Green
                    } else {
                        card_text
                    };
                    let mut row = vec![
                        Span::raw("  "),
                        Span::styled("│ ", Style::default().fg(card_border)),
                    ];
                    row.extend(style_tool_wrapped_line(&wrapped, base_color));
                    let pad = card_text_width.saturating_sub(wrapped.chars().count());
                    if pad > 0 {
                        row.push(Span::raw(" ".repeat(pad)));
                    }
                    row.push(Span::styled(" │", Style::default().fg(card_border)));
                    conversation_lines.push(Line::from(row));
                }
            }
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("│ ", Style::default().fg(card_border)),
                Span::styled(empty, Style::default().fg(card_text)),
                Span::styled(" │", Style::default().fg(card_border)),
            ]));
            let bottom = build_bottom_border(card_inner_width);
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(bottom, Style::default().fg(card_border)),
            ]));
            conversation_lines.push(Line::from(""));
            continue;
        }

        let text = parse_markdown_lines(&m.text);
        for (line, is_code) in text {
            let mut spans = vec![Span::styled("  ", Style::default())];
            if is_code {
                spans.push(Span::styled("▏ ", Style::default().fg(Color::Gray)));
                spans.push(Span::styled(line, Style::default().fg(Color::Cyan)));
            } else {
                spans.push(Span::styled("▎ ", rail_style));
                spans.extend(parse_markdown_bold_spans(&line));
            }
            conversation_lines.push(Line::from(spans));
        }
        conversation_lines.push(Line::from(""));
    }

    if app.busy {
        let busy_state = build_busy_state_view(app);
        let elapsed = app
            .busy_since
            .map(|t| format_duration(t.elapsed()))
            .unwrap_or_else(|| "0ms".to_string());
        let title = format!("AI {} {}", spinner, elapsed);
        conversation_lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                build_titled_top_border(card_inner_width, &title),
                Style::default().fg(Color::Green),
            ),
        ]));
        for line in wrap_and_truncate_for_width(&busy_state.headline, card_text_width) {
            let padded = format!("{:<width$}", line, width = card_text_width);
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("│ ", Style::default().fg(Color::Green)),
                Span::styled(
                    padded,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" │", Style::default().fg(Color::Green)),
            ]));
        }
        for line in wrap_and_truncate_for_width(&busy_state.detail, card_text_width) {
            let padded = format!("{:<width$}", line, width = card_text_width);
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("│ ", Style::default().fg(Color::Green)),
                Span::styled(padded, Style::default().fg(Color::Gray)),
                Span::styled(" │", Style::default().fg(Color::Green)),
            ]));
        }
        if let Some(support) = &busy_state.support {
            for line in wrap_and_truncate_for_width(support, card_text_width) {
                let padded = format!("{:<width$}", line, width = card_text_width);
                conversation_lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("│ ", Style::default().fg(Color::Green)),
                    Span::styled(padded, Style::default().fg(Color::DarkGray)),
                    Span::styled(" │", Style::default().fg(Color::Green)),
                ]));
            }
        }
        for line in wrap_and_truncate_for_width(
            &busy_progress_text_for_width(busy_state.stage, spinner, card_text_width),
            card_text_width,
        ) {
            let padded = format!("{:<width$}", line, width = card_text_width);
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("│ ", Style::default().fg(Color::Green)),
                Span::styled(padded, Style::default().fg(Color::Cyan)),
                Span::styled(" │", Style::default().fg(Color::Green)),
            ]));
        }
        for line in wrap_and_truncate_for_width(&busy_cancel_hint(card_text_width), card_text_width)
        {
            let padded = format!("{:<width$}", line, width = card_text_width);
            conversation_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("│ ", Style::default().fg(Color::Green)),
                Span::styled(padded, Style::default().fg(Color::DarkGray)),
                Span::styled(" │", Style::default().fg(Color::Green)),
            ]));
        }
        conversation_lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                build_bottom_border(card_inner_width),
                Style::default().fg(Color::Green),
            ),
        ]));
        conversation_lines.push(Line::from(""));
    }

    if let Some(bar) = &app.quick_actions {
        if !app.busy {
            let mut action_spans: Vec<Span<'_>> = vec![Span::raw("  ")];
            for (i, action) in bar.actions.iter().enumerate() {
                let is_selected = i == bar.selected;
                if is_selected {
                    action_spans.push(Span::styled(
                        format!(" {} ", action.label),
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ));
                } else {
                    action_spans.push(Span::styled(
                        format!(" {} ", action.label),
                        Style::default().fg(Color::White),
                    ));
                }
                if i + 1 < bar.actions.len() {
                    action_spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
                }
            }
            conversation_lines.push(Line::from(action_spans));
            conversation_lines.push(Line::from(vec![Span::styled(
                "    ←→ select · Enter confirm · Esc dismiss",
                Style::default().fg(Color::DarkGray),
            )]));
            conversation_lines.push(Line::from(""));
        }
    }

    let convo_height = main_chunks[0].height.saturating_sub(2) as usize;
    let max_offset = conversation_lines.len().saturating_sub(convo_height);
    let offset = app.conversation_scroll.min(max_offset);
    let end = conversation_lines.len().saturating_sub(offset);
    let start = end.saturating_sub(convo_height);
    let visible_lines = conversation_lines[start..end].to_vec();

    let convo_title = if offset > 0 {
        format!(" Conversation  ↓ {} more lines ", offset)
    } else {
        " Conversation ".to_string()
    };
    let convo_border_color = if offset > 0 {
        Color::Yellow
    } else {
        Color::Gray
    };
    let convo = Paragraph::new(visible_lines)
        .block(
            Block::default()
                .title(convo_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(convo_border_color)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(Clear, main_chunks[0]);
    frame.render_widget(convo, main_chunks[0]);

    if app.show_context && root[1].width >= 80 {
        let all_inspector_lines = app.active_inspector_lines();
        let total_lines = all_inspector_lines.len();
        let inspector_height = main_chunks[1].height.saturating_sub(2) as usize;
        let max_scroll = total_lines.saturating_sub(inspector_height);
        let scroll_offset = app.inspector_scroll.min(max_scroll);

        let line_indicator = if total_lines > inspector_height {
            let current_top = scroll_offset + 1;
            let current_bottom = (scroll_offset + inspector_height).min(total_lines);
            format!(" {}-{}/{} ", current_top, current_bottom, total_lines)
        } else {
            format!(" {}/{} ", total_lines, total_lines)
        };

        let inspector_title = match app.inspector_tab {
            InspectorTab::Context => format!(
                " {} [{}]{} ",
                app.inspector_tab.title(),
                app.context_mode.title(),
                line_indicator
            ),
            _ => format!(" {}{} ", app.inspector_tab.title(), line_indicator),
        };
        let yaml_mode = matches!(app.inspector_tab, InspectorTab::Context)
            && matches!(app.context_mode, ContextMode::Yaml);
        let inspector_lines: Vec<Line<'_>> = all_inspector_lines
            .iter()
            .skip(scroll_offset)
            .take(inspector_height)
            .map(|line| {
                if yaml_mode {
                    style_yaml_inspector_line(line)
                } else {
                    style_inspector_line(line)
                }
            })
            .collect();
        let mut ctx = Paragraph::new(inspector_lines).block(
            Block::default()
                .title(inspector_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        );
        if !yaml_mode {
            ctx = ctx.wrap(Wrap { trim: true });
        }
        frame.render_widget(Clear, main_chunks[1]);
        frame.render_widget(ctx, main_chunks[1]);
    }

    let (input_title, input_border_color) = if app.busy {
        (" Input (busy, Ctrl+C to cancel) ", Color::DarkGray)
    } else {
        (" Input (Enter to send, /help) ", Color::White)
    };
    let input_text_width = root[2].width.saturating_sub(2) as usize;
    let mut input_lines = if app.busy {
        build_busy_input_lines(app, spinner, input_text_width)
    } else if app.input.is_empty() {
        vec![Line::from(Span::styled(
            "Type / for commands, or ask in plain English...",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        vec![Line::from(app.input.clone())]
    };
    if !app.autocomplete.is_empty() {
        input_lines.push(Line::from(Span::styled(
            " ",
            Style::default().bg(Color::Rgb(60, 66, 76)),
        )));
    }

    let input = Paragraph::new(input_lines)
        .style(Style::default().add_modifier(Modifier::BOLD))
        .block(
            Block::default()
                .title(input_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(input_border_color)),
        );
    frame.render_widget(Clear, root[2]);
    frame.render_widget(input, root[2]);

    if !app.autocomplete.is_empty() {
        let menu_height = (app.autocomplete.len().min(8) + 2) as u16;
        let menu_width = root[2].width.min(84);
        let menu_area = ratatui::layout::Rect {
            x: root[2].x,
            y: root[2].y.saturating_sub(menu_height),
            width: menu_width,
            height: menu_height,
        };

        let mut menu_lines = vec![Line::from(Span::styled(
            " suggestions ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))];
        for (idx, (cmd, desc)) in app.autocomplete.iter().take(8).enumerate() {
            let selected = idx == app.autocomplete_index.min(7);
            let marker = if selected { "› " } else { "  " };
            let bg = if selected {
                Color::Rgb(78, 84, 96)
            } else {
                Color::Rgb(38, 42, 51)
            };
            menu_lines.push(Line::from(vec![
                Span::styled(marker, Style::default().bg(bg).fg(Color::White)),
                Span::styled(
                    format!("{:<16}", cmd),
                    Style::default()
                        .bg(bg)
                        .fg(if selected { Color::Cyan } else { Color::White })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(desc.clone(), Style::default().bg(bg).fg(Color::Gray)),
            ]));
        }

        let popup = Paragraph::new(menu_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(90, 95, 110))),
        );
        frame.render_widget(popup, menu_area);
    }
}

fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn teardown_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn apply_stream_update(app: &mut ReplApp, update: &StreamUpdate) {
    match update {
        StreamUpdate::Token(token) => app.append_stream_token(token),
        StreamUpdate::ToolCallStart { name, args } => {
            let compact = compact_tool_start(name, args);
            app.tool_start_times.insert(name.clone(), Instant::now());
            app.push_log(format!("… {} started", name));
            app.push_tool_line(compact);
            app.inspector_tab = InspectorTab::Tools;
        }
        StreamUpdate::ToolCallResult { name, result } => {
            if name == "r8r_generate" {
                let duration_text = app
                    .tool_start_times
                    .remove(name)
                    .map(|started| format_duration(started.elapsed()));
                if let Some((wf_name, valid, nodes)) = parse_generate_result(result) {
                    app.turn_outcome.generated_name = Some(wf_name);
                    app.turn_outcome.generated_valid = Some(valid);
                    app.turn_outcome.generated_nodes = Some(nodes);
                }
                if let Some((compact, yaml_lines)) = compact_generate_tool_result(result) {
                    app.push_message(MessageKind::Tool, compact.clone());
                    let timed = if let Some(d) = duration_text {
                        app.turn_outcome
                            .tool_durations
                            .push(format!("r8r_generate {}", d));
                        format!("result r8r_generate {} ({})", compact, d)
                    } else {
                        format!("result r8r_generate {}", compact)
                    };
                    let duration_short = timed
                        .rsplit_once('(')
                        .and_then(|(_, t)| t.strip_suffix(')'))
                        .unwrap_or("n/a");
                    app.push_log(format!("✓ r8r_generate ({})", duration_short));
                    app.push_tool_line(timed);
                    app.yaml_lines = yaml_lines;
                    app.context_mode = ContextMode::Yaml;
                    app.inspector_tab = InspectorTab::Context;
                    app.show_context = true;
                    return;
                }
            }
            if name == "r8r_validate" {
                app.turn_outcome.validated = parse_validate_result(result);
            }
            if name == "r8r_create_workflow" {
                app.turn_outcome.saved_name = parse_create_result(result);
            }
            let duration_text = app
                .tool_start_times
                .remove(name)
                .map(|started| format_duration(started.elapsed()));
            let compact = compact_tool_result(name, result);
            let timed = if let Some(d) = duration_text {
                app.turn_outcome
                    .tool_durations
                    .push(format!("{} {}", name, d));
                format!("{} ({})", compact, d)
            } else {
                compact.clone()
            };
            let duration_short = timed
                .rsplit_once('(')
                .and_then(|(_, t)| t.strip_suffix(')'))
                .unwrap_or("n/a");
            app.push_log(format!("✓ {} ({})", name, duration_short));
            app.push_tool_line(timed.clone());
            if compact.contains(" error=") {
                app.push_log(format!("! {}", compact));
                app.push_message(MessageKind::Error, compact);
            }
            app.inspector_tab = InspectorTab::Tools;
        }
        StreamUpdate::Done => {
            app.push_log("Assistant turn complete.");
            app.inspector_tab = InspectorTab::Log;
        }
        StreamUpdate::Error(err) => {
            app.push_message(MessageKind::Error, err.clone());
            app.push_log(format!("Error: {}", err));
            app.inspector_tab = InspectorTab::Log;
        }
    }
}

fn show_help(app: &mut ReplApp) {
    app.push_message(MessageKind::System, "Available slash commands:");
    for (cmd, desc) in slash_commands() {
        app.push_message(MessageKind::System, format!("{} - {}", cmd, desc));
    }
}

fn load_session_into_app(app: &mut ReplApp, stored: &[ReplMessage]) {
    let mut values = Vec::new();
    for msg in stored {
        match msg.role.as_str() {
            "user" => {
                values.push(json!({"role": "user", "content": msg.content}));
                app.push_message(MessageKind::User, msg.content.clone());
            }
            "assistant" => {
                values.push(json!({"role": "assistant", "content": msg.content}));
                app.push_message(MessageKind::Assistant, msg.content.clone());
                if let Some(tokens) = msg.token_count {
                    if tokens > 0 {
                        app.session_usage.persisted_total_tokens += tokens as u64;
                        app.session_usage.turns_with_usage += 1;
                    } else {
                        app.session_usage.turns_without_usage += 1;
                    }
                } else {
                    app.session_usage.turns_without_usage += 1;
                }
            }
            "tool_call" | "tool_result" | "tool" => {
                app.push_message(MessageKind::Tool, msg.content.clone());
                app.push_tool_line(msg.content.clone());
            }
            _ => app.push_message(MessageKind::System, msg.content.clone()),
        }
    }
    app.conversation.load_from_stored(values);
}

fn workflow_to_dag_lines(yaml: &str) -> Vec<String> {
    match parse_workflow(yaml) {
        Ok(wf) => {
            let mut depth = std::collections::HashMap::<String, usize>::new();
            let mut changed = true;
            while changed {
                changed = false;
                for n in &wf.nodes {
                    let d = n
                        .depends_on
                        .iter()
                        .filter_map(|dep| depth.get(dep).copied())
                        .max()
                        .map(|v| v + 1)
                        .unwrap_or(0);
                    if depth.get(&n.id).copied() != Some(d) {
                        depth.insert(n.id.clone(), d);
                        changed = true;
                    }
                }
            }

            let mut nodes = wf.nodes.clone();
            nodes.sort_by_key(|n| (depth.get(&n.id).copied().unwrap_or(0), n.id.clone()));

            let mut lines = Vec::new();
            let mut last_depth = 0usize;
            for n in &nodes {
                let d = depth.get(&n.id).copied().unwrap_or(0);
                if !lines.is_empty() {
                    if d > last_depth {
                        lines.push(format!("{}│", "  ".repeat(last_depth + 2)));
                    } else {
                        lines.push(String::new());
                    }
                }
                let indent = "  ".repeat(d);
                lines.push(format!("{}┌{}┐", indent, "─".repeat(n.id.len() + 2)));
                lines.push(format!("{}│ {} │ ○ {}", indent, n.id, n.node_type));
                lines.push(format!("{}└{}┘", indent, "─".repeat(n.id.len() + 2)));
                if !n.depends_on.is_empty() {
                    lines.push(format!(
                        "{}  depends_on: {}",
                        indent,
                        n.depends_on.join(", ")
                    ));
                }
                last_depth = d;
            }

            if lines.is_empty() {
                vec!["Workflow has no nodes.".to_string()]
            } else {
                lines
            }
        }
        Err(e) => vec![format!("Failed to parse workflow DAG: {}", e)],
    }
}

fn execution_trace_lines(trace: &ExecutionTrace) -> Vec<String> {
    let mut lines = vec![
        format!("execution: {}", trace.execution.id),
        format!("workflow: {}", trace.execution.workflow_name),
        format!("status: {}", trace.execution.status),
        "nodes:".to_string(),
    ];
    for n in &trace.nodes {
        lines.push(format!(
            "- {} {} started={}",
            n.node_id,
            n.status,
            n.started_at.to_rfc3339()
        ));
    }
    lines
}

async fn switch_session(
    app: &mut ReplApp,
    storage: &dyn Storage,
    new_session_id: &str,
) -> anyhow::Result<()> {
    let session = storage.get_repl_session(new_session_id).await?;
    if session.is_none() {
        app.push_message(
            MessageKind::Error,
            format!("Session not found: {}", new_session_id),
        );
        return Ok(());
    }

    app.session_id = new_session_id.to_string();
    app.messages.clear();
    app.log_lines.clear();
    app.tool_lines = vec!["No tool calls yet.".to_string()];
    app.dag_lines = vec!["No DAG loaded.".to_string()];
    app.yaml_lines = vec!["No YAML loaded.".to_string()];
    app.trace_lines = vec!["No trace loaded.".to_string()];
    app.inspector_tab = InspectorTab::Log;
    app.jump_to_latest();
    app.conversation.clear();

    let loaded = storage.list_repl_messages(&app.session_id, 1000).await?;
    load_session_into_app(app, &loaded);
    app.push_log(format!(
        "Switched to session {} ({} messages)",
        app.session_id,
        loaded.len()
    ));

    Ok(())
}

async fn handle_slash_command(
    app: &mut ReplApp,
    cmd: InputCommand,
    storage: &dyn Storage,
    executor: &Arc<Executor>,
    llm_client: &mut reqwest::Client,
    llm_config: &mut Option<LlmConfig>,
) -> anyhow::Result<()> {
    match cmd {
        InputCommand::Run { workflow } => {
            app.push_message(
                MessageKind::System,
                format!("Running workflow {}", workflow),
            );
            let result = tools::execute_tool(
                "r8r_execute",
                &json!({"workflow": workflow}),
                storage,
                executor,
            )
            .await;
            app.push_message(MessageKind::Tool, result.clone());
            app.trace_lines = vec![result.clone()];
            app.context_mode = ContextMode::Trace;
            app.inspector_tab = InspectorTab::Context;
            let exec_id = serde_json::from_str::<Value>(&result)
                .ok()
                .and_then(|v| {
                    v.get("execution_id")
                        .and_then(|e| e.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_default();
            if !exec_id.is_empty() {
                app.quick_actions = Some(QuickActionBar::new(actions_execution_done(
                    &workflow, &exec_id,
                )));
            }
        }
        InputCommand::List => {
            let wfs = storage.list_workflows().await?;
            app.workflow_count = wfs.len();
            app.push_message(MessageKind::System, format!("{} workflows", wfs.len()));
            app.log_lines = if wfs.is_empty() {
                vec!["No workflows found.".to_string()]
            } else {
                wfs.iter()
                    .map(|w| {
                        format!(
                            "- {:<30}  {}",
                            w.name,
                            if w.enabled { "enabled" } else { "disabled" }
                        )
                    })
                    .collect()
            };
            app.inspector_tab = InspectorTab::Log;
        }
        InputCommand::Show { workflow } => match storage.get_workflow(&workflow).await? {
            Some(wf) => {
                app.yaml_lines = wf.definition.lines().map(|s| s.to_string()).collect();
                app.context_mode = ContextMode::Yaml;
                app.inspector_tab = InspectorTab::Context;
                app.push_message(MessageKind::System, format!("Loaded YAML for {}", workflow));
            }
            None => app.push_message(
                MessageKind::Error,
                format!("Workflow not found: {}", workflow),
            ),
        },
        InputCommand::Dag { workflow } => match storage.get_workflow(&workflow).await? {
            Some(wf) => {
                app.dag_lines = workflow_to_dag_lines(&wf.definition);
                app.context_mode = ContextMode::Dag;
                app.inspector_tab = InspectorTab::Context;
                app.push_message(MessageKind::System, format!("Loaded DAG for {}", workflow));
            }
            None => app.push_message(
                MessageKind::Error,
                format!("Workflow not found: {}", workflow),
            ),
        },
        InputCommand::Logs { workflow } => {
            let executions = storage.list_executions(&workflow, 20).await?;
            app.log_lines = if executions.is_empty() {
                vec![format!("No executions for {}", workflow)]
            } else {
                executions
                    .iter()
                    .map(|e| {
                        format!(
                            "- {} status={} started={}",
                            e.id,
                            e.status,
                            e.started_at.to_rfc3339()
                        )
                    })
                    .collect()
            };
            app.inspector_tab = InspectorTab::Log;
        }
        InputCommand::Trace { execution_id } => {
            match storage.get_execution_trace(&execution_id).await? {
                Some(trace) => {
                    app.trace_lines = execution_trace_lines(&trace);
                    app.context_mode = ContextMode::Trace;
                    app.inspector_tab = InspectorTab::Context;
                }
                None => app.push_message(
                    MessageKind::Error,
                    format!("Execution not found: {}", execution_id),
                ),
            }
        }
        InputCommand::Sessions => {
            let sessions = storage.list_repl_sessions(20).await?;
            if sessions.is_empty() {
                app.push_message(MessageKind::System, "No sessions available.");
            } else {
                app.push_message(MessageKind::System, "Recent sessions:");
                for s in sessions {
                    app.push_message(
                        MessageKind::System,
                        format!(
                            "- {} model={} summary={}",
                            s.id,
                            s.model,
                            s.summary.unwrap_or_default()
                        ),
                    );
                }
            }
        }
        InputCommand::Resume { session_id } => {
            if let Some(id) = session_id {
                switch_session(app, storage, &id).await?;
            } else {
                let sessions = storage.list_repl_sessions(20).await?;
                app.push_message(MessageKind::System, "Use /resume <id>. Available sessions:");
                for s in sessions {
                    app.push_message(MessageKind::System, format!("- {}", s.id));
                }
            }
        }
        InputCommand::Reconnect => {
            *llm_client = llm::create_llm_client();
            app.push_message(MessageKind::System, "Reconnected LLM client.");
            app.push_log("LLM client recreated.");
        }
        InputCommand::SetModel { provider, model } => {
            let mut cfg = llm_config.clone().unwrap_or_default();
            if let Some(provider_raw) = provider {
                match parse_provider(&provider_raw) {
                    Some(p) => cfg.provider = p,
                    None => {
                        app.push_message(
                            MessageKind::Error,
                            format!(
                                "Unknown provider '{}'. Use openai|anthropic|ollama|custom.",
                                provider_raw
                            ),
                        );
                        return Ok(());
                    }
                }
            }
            cfg.model = Some(model.clone());
            *llm_config = Some(cfg.clone());
            storage.save_repl_llm_config(&cfg).await?;
            app.model = model.clone();
            app.push_message(
                MessageKind::System,
                format!(
                    "LLM model set to {} ({:?}) and persisted",
                    model, cfg.provider
                ),
            );
        }
        InputCommand::SetApiKey { api_key } => {
            let mut cfg = llm_config.clone().unwrap_or_default();
            cfg.api_key = Some(api_key);
            storage.save_repl_llm_config(&cfg).await?;
            *llm_config = Some(cfg);
            app.push_message(
                MessageKind::System,
                "API key updated and persisted for future REPL sessions.",
            );
        }
        InputCommand::SetEndpoint { endpoint } => {
            let mut cfg = llm_config.clone().unwrap_or_default();
            cfg.endpoint = Some(endpoint.clone());
            storage.save_repl_llm_config(&cfg).await?;
            *llm_config = Some(cfg);
            app.push_message(
                MessageKind::System,
                format!("Endpoint set to {} and persisted", endpoint),
            );
        }
        InputCommand::ShowLlm => {
            if let Some(cfg) = llm_config {
                let key_state = if cfg.api_key.is_some() {
                    "configured"
                } else {
                    "missing"
                };
                app.push_message(
                    MessageKind::System,
                    format!(
                        "LLM provider={:?} model={} endpoint={} api_key={} timeout={}s",
                        cfg.provider,
                        cfg.model.clone().unwrap_or_else(|| "unset".to_string()),
                        cfg.endpoint
                            .clone()
                            .unwrap_or_else(|| "default".to_string()),
                        key_state,
                        cfg.timeout_seconds
                    ),
                );
            } else {
                app.push_message(
                    MessageKind::Error,
                    "LLM not configured. Use /model, /apikey, and /endpoint.",
                );
            }
        }
        InputCommand::View { target } => {
            match target.as_str() {
                "log" | "logs" => app.inspector_tab = InspectorTab::Log,
                "tools" | "tool" => app.inspector_tab = InspectorTab::Tools,
                "yaml" => {
                    let no_yaml_loaded = app.yaml_lines.is_empty()
                        || (app.yaml_lines.len() == 1
                            && app.yaml_lines[0].trim().starts_with("No YAML loaded"));
                    if no_yaml_loaded {
                        app.push_message(
                            MessageKind::Error,
                            "No YAML loaded yet. Generate a workflow or run /show <workflow> first.",
                        );
                        return Ok(());
                    }
                    app.context_mode = ContextMode::Yaml;
                    app.inspector_tab = InspectorTab::Context;
                }
                "dag" => {
                    app.context_mode = ContextMode::Dag;
                    app.inspector_tab = InspectorTab::Context;
                }
                "trace" => {
                    app.context_mode = ContextMode::Trace;
                    app.inspector_tab = InspectorTab::Context;
                }
                "context" => app.inspector_tab = InspectorTab::Context,
                _ => {
                    app.push_message(
                        MessageKind::Error,
                        "Usage: /view <log|tools|yaml|dag|trace|context>",
                    );
                    return Ok(());
                }
            }
            app.show_context = true;
            app.push_message(MessageKind::System, format!("Switched view: {}", target));
        }
        InputCommand::ExportYaml { path } => {
            let yaml = app.yaml_lines.join("\n").trim().to_string();
            if yaml.is_empty() || yaml.starts_with("No YAML ") {
                app.push_message(
                    MessageKind::Error,
                    "No YAML loaded. Generate/show a workflow first, then run /export yaml <path>.",
                );
                return Ok(());
            }

            let path_buf = std::path::PathBuf::from(&path);
            if let Some(parent) = path_buf.parent() {
                if !parent.as_os_str().is_empty() {
                    tokio::fs::create_dir_all(parent).await?;
                }
            }
            tokio::fs::write(&path_buf, format!("{}\n", yaml)).await?;
            app.push_message(
                MessageKind::System,
                format!("Exported YAML to {}", path_buf.display()),
            );
            app.push_log(format!("YAML exported: {}", path_buf.display()));
            app.context_mode = ContextMode::Yaml;
            app.inspector_tab = InspectorTab::Context;
            app.show_context = true;
        }
        InputCommand::Help => show_help(app),
        InputCommand::Clear => {
            app.conversation.clear();
            app.messages.clear();
            app.log_lines.clear();
            app.tool_lines = vec!["No tool calls yet.".to_string()];
            app.jump_to_latest();
            app.push_log("Cleared conversation.");
            app.inspector_tab = InspectorTab::Log;
        }
        InputCommand::Exit => app.should_quit = true,
        InputCommand::Unknown(cmd) => {
            let usage = match cmd.as_str() {
                "/show" => Some("Usage: /show <workflow>"),
                "/run" => Some("Usage: /run <workflow>"),
                "/dag" => Some("Usage: /dag <workflow>"),
                "/logs" => Some("Usage: /logs <workflow>"),
                "/trace" => Some("Usage: /trace <execution-id>"),
                "/model" => Some("Usage: /model <model> OR /model <provider> <model>"),
                "/apikey" => Some("Usage: /apikey <key>"),
                "/endpoint" => Some("Usage: /endpoint <url>"),
                "/view" => Some("Usage: /view <log|tools|yaml|dag|trace|context>"),
                "/export" => Some("Usage: /export yaml <path>"),
                _ => None,
            };
            if let Some(message) = usage {
                app.push_message(MessageKind::Error, message);
                if cmd == "/show" {
                    if let Ok(wfs) = storage.list_workflows().await {
                        if let Some(first) = wfs.first() {
                            app.push_message(
                                MessageKind::System,
                                format!("Try: /show {}", first.name),
                            );
                        }
                    }
                }
            } else {
                app.push_message(MessageKind::Error, format!("Unknown command: {}", cmd));
            }
        }
        InputCommand::Plan { description } => {
            let header = description
                .as_deref()
                .map(|d| format!("Plan: {}", d))
                .unwrap_or_else(|| {
                    "Plan (describe what you want to do before running)".to_string()
                });
            let plan_text = format!(
                "{}\n\nSteps:\n  1. Describe what nodes/services will be called\n  2. Review side effects below\n  3. Run with /run <workflow> or tell the AI what to do\n\nSide effects from write-capable tools:\n  • r8r_create_workflow — creates/overwrites a workflow\n  • r8r_execute / r8r_run_and_wait — triggers execution (may call HTTP, agent, etc.)\n  • r8r_approve — resolves a human-in-the-loop approval\n\nWrite gate: {}\nUse /arm to allow writes, /disarm to return to safe mode.",
                header,
                if app.writes_armed { "⚡ ARMED — writes allowed" } else { "🔒 SAFE — writes blocked (use /arm to allow)" }
            );
            app.log_lines = plan_text.lines().map(|s| s.to_string()).collect();
            app.inspector_tab = InspectorTab::Log;
            app.push_message(
                MessageKind::System,
                format!("Plan loaded in Log panel. {}", header),
            );
        }
        InputCommand::Usage => {
            let su = &app.session_usage;
            let current_total = su.total_prompt_tokens + su.total_completion_tokens;
            let total = current_total + su.persisted_total_tokens;
            let turns = su.turns_with_usage;
            let current_cost = estimate_cost(
                &app.model,
                Some(su.total_prompt_tokens),
                Some(su.total_completion_tokens),
            );

            let mut text = format!("Session usage ({} turns):\n\n", turns);
            text.push_str(&format!(
                "  Prompt tokens:      {:>10}\n",
                su.total_prompt_tokens
            ));
            text.push_str(&format!(
                "  Completion tokens:  {:>10}\n",
                su.total_completion_tokens
            ));
            if su.persisted_total_tokens > 0 {
                text.push_str(&format!(
                    "  Historical tokens:  {:>10}\n",
                    su.persisted_total_tokens
                ));
            }
            text.push_str(&format!("  Total tokens:       {:>10}\n", total));
            if su.persisted_total_tokens > 0 {
                if let Some(c) = current_cost {
                    text.push_str(&format!(
                        "  Estimated cost:     ~${:.3} (current process only)\n",
                        c
                    ));
                } else {
                    text.push_str("  Estimated cost:     unavailable for resumed history\n");
                }
            } else if let Some(c) = current_cost {
                text.push_str(&format!("  Estimated cost:     ~${:.3}\n", c));
            }
            text.push_str(&format!("\n  Model: {}", app.model));

            app.push_message(MessageKind::System, text);
        }
        InputCommand::ArmWrites => {
            app.writes_armed = true;
            app.push_message(
                MessageKind::System,
                "⚡ Writes ARMED — the AI can now create workflows, execute runs, and approve requests. Use /disarm to return to safe mode.",
            );
            if app.operator_mode {
                app.push_log("Write gate: ARMED");
            }
        }
        InputCommand::Disarm => {
            app.writes_armed = false;
            app.pending_write_confirm = None;
            app.push_message(
                MessageKind::System,
                "🔒 Writes DISARMED — safe mode active. The AI will ask before taking write actions.",
            );
        }
        InputCommand::Operator { enable } => {
            app.operator_mode = match enable {
                Some(v) => v,
                None => !app.operator_mode,
            };
            if app.operator_mode {
                app.push_message(
                    MessageKind::System,
                    format!(
                        "OPERATOR mode ON — status bar shows policy/gate state.\nWrite gate: {}\nPolicy file: ~/.config/r8r/policy.toml",
                        if app.writes_armed { "⚡ armed" } else { "🔒 safe" },
                    ),
                );
            } else {
                app.push_message(
                    MessageKind::System,
                    "Operator mode OFF — builder mode active.",
                );
            }
        }
        InputCommand::NaturalLanguage(_) | InputCommand::Empty => {}
    }

    Ok(())
}

/// Returns true if the user message likely intends to create, run, or approve something.
fn input_has_write_intent(text: &str) -> bool {
    let lower = text.to_lowercase();
    let write_keywords = [
        "create", "generate", "make", "build", "write", "add", "run", "execute", "trigger",
        "start", "launch", "deploy", "approve", "confirm", "accept", "delete", "remove", "drop",
        "save", "update", "modify", "change",
    ];
    write_keywords.iter().any(|kw| lower.contains(kw))
}

/// Builds a human-readable summary of what write operations might happen.
fn build_write_summary(text: &str) -> String {
    let lower = text.to_lowercase();
    let mut effects = Vec::new();
    if lower.contains("create")
        || lower.contains("generate")
        || lower.contains("build")
        || lower.contains("make")
    {
        effects.push("  • Create or overwrite a workflow definition");
    }
    if lower.contains("run")
        || lower.contains("execute")
        || lower.contains("trigger")
        || lower.contains("start")
    {
        effects.push("  • Execute a workflow (may call external APIs, send messages, etc.)");
    }
    if lower.contains("approve") || lower.contains("confirm") {
        effects.push("  • Approve a pending human-in-the-loop request");
    }
    if lower.contains("delete") || lower.contains("remove") {
        effects.push("  • Delete a workflow or resource");
    }
    if lower.contains("save") || lower.contains("update") || lower.contains("modify") {
        effects.push("  • Modify an existing workflow");
    }
    if effects.is_empty() {
        effects.push("  • Unknown write operation");
    }
    effects.join("\n")
}

fn parse_provider(raw: &str) -> Option<LlmProvider> {
    match raw.to_ascii_lowercase().as_str() {
        "openai" => Some(LlmProvider::Openai),
        "anthropic" => Some(LlmProvider::Anthropic),
        "ollama" => Some(LlmProvider::Ollama),
        "custom" => Some(LlmProvider::Custom),
        _ => None,
    }
}

/// Run the REPL TUI with real turn execution and streaming progress updates.
#[allow(clippy::too_many_arguments)]
pub async fn run_repl_tui(
    storage: Arc<dyn Storage>,
    executor: Arc<Executor>,
    llm_config: Option<LlmConfig>,
    llm_client: reqwest::Client,
    session_id: String,
    system_prompt: String,
    tool_defs: Vec<Value>,
) -> anyhow::Result<()> {
    let workflow_count = storage.list_workflows().await.unwrap_or_default().len();
    let mut active_llm_config = llm_config;
    let model = active_llm_config
        .as_ref()
        .and_then(|c| c.model.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let mut app = ReplApp::new(
        session_id.clone(),
        model,
        Conversation::new(MAX_CONTEXT_MESSAGES),
        workflow_count,
    );

    let existing = storage.list_repl_messages(&session_id, 1000).await?;
    if !existing.is_empty() {
        load_session_into_app(&mut app, &existing);
        app.push_log(format!("Loaded {} messages from session.", existing.len()));
    }
    app.refresh_autocomplete();

    let mut llm_client = llm_client;
    let mut terminal = setup_terminal()?;
    let (tx, mut rx) = mpsc::unbounded_channel::<ReplEvent>();
    let mut last_tick = Instant::now();
    let mut current_turn: Option<JoinHandle<()>> = None;

    loop {
        terminal.draw(|f| render(f, &app))?;

        while let Ok(event) = rx.try_recv() {
            match event {
                ReplEvent::Stream(update) => {
                    if let StreamUpdate::ToolCallStart { name, args } = &update {
                        let redacted_args = redact_credentials(args);
                        storage
                            .save_repl_message(
                                &app.session_id,
                                "tool_call",
                                &json!({"name": name, "args": redacted_args}).to_string(),
                                None,
                                None, // run_id not yet known at call-start time
                            )
                            .await?;
                    }
                    if let StreamUpdate::ToolCallResult { name, result } = &update {
                        // Extract execution_id from tool result if present, store for correlation.
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(result) {
                            if let Some(exec_id) =
                                parsed.get("execution_id").and_then(|v| v.as_str())
                            {
                                app.last_run_id = Some(exec_id.to_string());
                            }
                        }
                        storage
                            .save_repl_message(
                                &app.session_id,
                                "tool_result",
                                &json!({"name": name, "result": result}).to_string(),
                                None,
                                app.last_run_id.as_deref(),
                            )
                            .await?;
                    }
                    apply_stream_update(&mut app, &update);
                }
                ReplEvent::TurnComplete {
                    conversation,
                    result,
                } => {
                    app.busy = false;
                    app.busy_since = None;
                    app.assistant_streaming_index = None;
                    app.conversation = conversation;

                    // Track token usage
                    if let Some(ref usage) = result.usage {
                        app.turn_usage = Some(usage.clone());
                        app.session_usage.total_prompt_tokens += usage.prompt_tokens.unwrap_or(0);
                        app.session_usage.total_completion_tokens +=
                            usage.completion_tokens.unwrap_or(0);
                        app.session_usage.turns_with_usage += 1;
                    } else {
                        app.turn_usage = None;
                        app.session_usage.turns_without_usage += 1;
                    }

                    if !result.response.is_empty() {
                        let extracted = extract_fenced_yaml_blocks(&result.response);
                        if let Some(last_yaml) = extracted.last() {
                            app.yaml_lines = last_yaml.lines().map(|s| s.to_string()).collect();
                            app.context_mode = ContextMode::Yaml;
                            app.inspector_tab = InspectorTab::Context;
                        }
                        let compact = strip_fenced_yaml_from_text(&result.response);
                        if let Some(msg) = app
                            .messages
                            .iter_mut()
                            .rev()
                            .find(|m| matches!(m.kind, MessageKind::Assistant))
                        {
                            msg.text = compact.clone();
                        }
                        // Show inline usage after assistant message
                        if let Some(ref usage) = app.turn_usage {
                            if let Some(usage_line) = format_usage_line(usage, &app.model) {
                                app.push_message(MessageKind::System, usage_line);
                            }
                        }
                        let token_count = app.turn_usage.as_ref().map(|u| {
                            (u.prompt_tokens.unwrap_or(0) + u.completion_tokens.unwrap_or(0)) as i64
                        });
                        storage
                            .save_repl_message(
                                &app.session_id,
                                "assistant",
                                &compact,
                                token_count,
                                app.last_run_id.as_deref(),
                            )
                            .await?;
                    }
                    if let Some(card) = build_result_card(&app) {
                        app.push_message(MessageKind::System, card);
                    }
                    // Show contextual quick-actions based on turn outcome
                    app.quick_actions = build_quick_actions(&app.turn_outcome);
                    app.sticky_actions = app.quick_actions.clone();
                    app.turn_outcome = TurnOutcome::default();
                    app.last_run_id = None;
                    app.tool_start_times.clear();
                    current_turn = None;
                }
            }
        }

        if event::poll(Duration::from_millis(33))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        if app.busy {
                            if let Some(handle) = current_turn.take() {
                                handle.abort();
                            }
                            app.busy = false;
                            app.busy_since = None;
                            app.assistant_streaming_index = None;
                            app.push_message(MessageKind::System, "(cancelled)");
                            app.push_log("Cancelled current request.");
                            app.turn_outcome = TurnOutcome::default();
                            app.last_run_id = None;
                            app.tool_start_times.clear();
                        } else {
                            let now = Instant::now();
                            if let Some(prev) = app.last_ctrl_c {
                                if now.duration_since(prev) <= CTRL_C_EXIT_WINDOW {
                                    app.should_quit = true;
                                } else {
                                    app.last_ctrl_c = Some(now);
                                    app.push_log("Press Ctrl+C again to exit.");
                                }
                            } else {
                                app.last_ctrl_c = Some(now);
                                app.push_log("Press Ctrl+C again to exit.");
                            }
                        }
                    } else if key.code == KeyCode::Esc {
                        if app.quick_actions.is_some() {
                            app.quick_actions = None;
                            if app.pending_write_confirm.is_some() {
                                app.pending_write_confirm = None;
                                app.push_message(
                                    MessageKind::System,
                                    "\u{2717} Cancelled. No write operations performed.",
                                );
                            }
                        } else {
                            app.should_quit = true;
                        }
                    } else if key.code == KeyCode::Char('i')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        app.show_context = !app.show_context;
                        app.push_log(if app.show_context {
                            "Inspector panel enabled."
                        } else {
                            "Inspector panel hidden."
                        });
                    } else if matches_ctrl_shortcut(&key, 'y', 0x19) {
                        app.context_mode = ContextMode::Yaml;
                        app.inspector_tab = InspectorTab::Context;
                        app.show_context = true;
                        app.push_log("Switched to YAML context view.");
                    } else if matches_ctrl_shortcut(&key, 'l', 0x0c) {
                        app.conversation.clear();
                        app.messages.clear();
                        app.log_lines.clear();
                        app.tool_lines = vec!["No tool calls yet.".to_string()];
                        app.push_log("Cleared conversation.");
                    } else if key.code == KeyCode::Up
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        app.scroll_inspector_up(4);
                    } else if key.code == KeyCode::Down
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        app.scroll_inspector_down(4);
                    } else if key.code == KeyCode::PageUp
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        app.scroll_inspector_up(12);
                    } else if key.code == KeyCode::PageDown
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        app.scroll_inspector_down(12);
                    } else if key.code == KeyCode::Tab {
                        if !app.apply_autocomplete() {
                            app.inspector_tab = app.inspector_tab.next();
                            app.inspector_scroll = 0;
                            app.show_context = true;
                        }
                    } else if key.code == KeyCode::BackTab {
                        app.inspector_scroll = 0;
                        app.context_mode = app.context_mode.next();
                        app.inspector_tab = InspectorTab::Context;
                        app.show_context = true;
                    } else if key.code == KeyCode::Left {
                        if let Some(bar) = &mut app.quick_actions {
                            bar.move_left();
                        }
                    } else if key.code == KeyCode::Right {
                        if let Some(bar) = &mut app.quick_actions {
                            bar.move_right();
                        } else {
                            let _ = app.apply_autocomplete();
                        }
                    } else if key.code == KeyCode::Up {
                        if !app.move_autocomplete(-1) {
                            app.scroll_conversation_up(1);
                        }
                    } else if key.code == KeyCode::Down {
                        if !app.move_autocomplete(1) {
                            app.scroll_conversation_down(1);
                        }
                    } else if key.code == KeyCode::PageUp {
                        app.scroll_conversation_up(8);
                    } else if key.code == KeyCode::PageDown {
                        app.scroll_conversation_down(8);
                    } else if key.code == KeyCode::End {
                        app.jump_to_latest();
                    } else if key.code == KeyCode::Backspace {
                        app.input.pop();
                        app.refresh_autocomplete();
                    } else if key.code == KeyCode::Enter {
                        if app.busy {
                            continue;
                        }

                        // Quick-action bar: if active and input is empty, execute selected action
                        if app.input.trim().is_empty() {
                            if let Some(bar) = app.quick_actions.take() {
                                if let Some(action) = bar.selected_action().cloned() {
                                    match &action.command {
                                        QuickActionCommand::Slash(cmd) => {
                                            app.input = cmd.clone();
                                            // Fall through to normal submit
                                        }
                                        QuickActionCommand::Prompt(prompt) => {
                                            app.input = prompt.clone();
                                            // Fall through to normal submit
                                        }
                                    }
                                }
                            }
                        }

                        let submitted = app.input.trim().to_string();
                        app.input.clear();
                        app.refresh_autocomplete();
                        if submitted.is_empty() {
                            continue;
                        }
                        app.jump_to_latest();

                        // ── Pending write confirmation (via quick-action bar) ───
                        if submitted.starts_with("__write_confirm_") {
                            if let Some(confirm) = app.pending_write_confirm.take() {
                                match submitted.as_str() {
                                    "__write_confirm_yes" => {
                                        app.push_message(
                                            MessageKind::System,
                                            "\u{2713} Confirmed. Proceeding with write operations.",
                                        );
                                        let original = confirm.original_input.clone();
                                        app.turn_outcome = TurnOutcome::default();
                                        app.last_run_id = None;
                                        app.tool_start_times.clear();
                                        app.push_message(MessageKind::User, original.clone());
                                        let _ = storage
                                            .save_repl_message(
                                                &app.session_id,
                                                "user",
                                                &original,
                                                None,
                                                None,
                                            )
                                            .await;
                                        let Some(config) = active_llm_config.clone() else {
                                            app.push_message(MessageKind::Error, "LLM is not configured.");
                                            continue;
                                        };
                                        app.busy = true;
                                        app.busy_since = Some(Instant::now());
                                        app.assistant_streaming_index = None;
                                        app.quick_actions = None;
                                        app.push_log("Calling LLM (confirmed write)...");
                                        let mut conversation = app.conversation.clone();
                                        let tx_updates = tx.clone();
                                        let storage_cloned = storage.clone();
                                        let executor_cloned = executor.clone();
                                        let client_cloned = llm_client.clone();
                                        let system_prompt_cloned = system_prompt.clone();
                                        let tool_defs_cloned = tool_defs.clone();
                                        let text_cloned = original;
                                        current_turn = Some(tokio::spawn(async move {
                                            let tx_stream = tx_updates.clone();
                                            let callback: engine::StreamCallback =
                                                Box::new(move |update| {
                                                    let _ = tx_stream.send(ReplEvent::Stream(update));
                                                });
                                            let result = engine::run_turn(
                                                &mut conversation,
                                                &text_cloned,
                                                &system_prompt_cloned,
                                                &config,
                                                &client_cloned,
                                                &storage_cloned,
                                                &executor_cloned,
                                                &tool_defs_cloned,
                                                &callback,
                                            )
                                            .await;
                                            let _ = tx_updates.send(ReplEvent::TurnComplete {
                                                conversation,
                                                result,
                                            });
                                        }));
                                    }
                                    "__write_confirm_arm" => {
                                        app.writes_armed = true;
                                        app.push_message(
                                            MessageKind::System,
                                            "\u{2713} Writes armed for this session. Proceeding.",
                                        );
                                        let original = confirm.original_input.clone();
                                        app.turn_outcome = TurnOutcome::default();
                                        app.last_run_id = None;
                                        app.tool_start_times.clear();
                                        app.push_message(MessageKind::User, original.clone());
                                        let _ = storage
                                            .save_repl_message(
                                                &app.session_id,
                                                "user",
                                                &original,
                                                None,
                                                None,
                                            )
                                            .await;
                                        let Some(config) = active_llm_config.clone() else {
                                            app.push_message(MessageKind::Error, "LLM is not configured.");
                                            continue;
                                        };
                                        app.busy = true;
                                        app.busy_since = Some(Instant::now());
                                        app.assistant_streaming_index = None;
                                        app.quick_actions = None;
                                        app.push_log("Calling LLM (armed session)...");
                                        let mut conversation = app.conversation.clone();
                                        let tx_updates = tx.clone();
                                        let storage_cloned = storage.clone();
                                        let executor_cloned = executor.clone();
                                        let client_cloned = llm_client.clone();
                                        let system_prompt_cloned = system_prompt.clone();
                                        let tool_defs_cloned = tool_defs.clone();
                                        let text_cloned = original;
                                        current_turn = Some(tokio::spawn(async move {
                                            let tx_stream = tx_updates.clone();
                                            let callback: engine::StreamCallback =
                                                Box::new(move |update| {
                                                    let _ = tx_stream.send(ReplEvent::Stream(update));
                                                });
                                            let result = engine::run_turn(
                                                &mut conversation,
                                                &text_cloned,
                                                &system_prompt_cloned,
                                                &config,
                                                &client_cloned,
                                                &storage_cloned,
                                                &executor_cloned,
                                                &tool_defs_cloned,
                                                &callback,
                                            )
                                            .await;
                                            let _ = tx_updates.send(ReplEvent::TurnComplete {
                                                conversation,
                                                result,
                                            });
                                        }));
                                    }
                                    _ => {
                                        // __write_confirm_no or unknown
                                        app.push_message(
                                            MessageKind::System,
                                            "\u{2717} Cancelled. No write operations performed.",
                                        );
                                        app.quick_actions = None;
                                    }
                                }
                            }
                            continue;
                        }

                        match parse_input(&submitted) {
                            InputCommand::NaturalLanguage(text) => {
                                app.turn_outcome = TurnOutcome::default();
                                app.last_run_id = None;
                                app.tool_start_times.clear();
                                app.push_message(MessageKind::User, text.clone());
                                storage
                                    .save_repl_message(&app.session_id, "user", &text, None, None)
                                    .await?;

                                if app
                                    .messages
                                    .iter()
                                    .filter(|m| matches!(m.kind, MessageKind::User))
                                    .count()
                                    == 1
                                {
                                    let summary = text.chars().take(120).collect::<String>();
                                    let _ = storage
                                        .update_repl_session_summary(&app.session_id, &summary)
                                        .await;
                                }

                                let Some(config) = active_llm_config.clone() else {
                                    app.push_message(
                                        MessageKind::Error,
                                        "LLM is not configured. Use slash commands or configure LLM credentials.",
                                    );
                                    continue;
                                };

                                // ── Write-gate guardrail ──────────────────
                                // When writes are disarmed, detect write intent and
                                // show a side-effect summary with a pick-to-confirm bar.
                                if !app.writes_armed && input_has_write_intent(&text) {
                                    app.pending_write_confirm = Some(PendingWriteConfirm {
                                        summary: build_write_summary(&text),
                                        original_input: text.clone(),
                                    });
                                    app.push_message(
                                        MessageKind::System,
                                        format!(
                                            "\u{26a0} This request may trigger write operations:\n{}",
                                            build_write_summary(&text)
                                        ),
                                    );
                                    app.quick_actions = Some(QuickActionBar::new(vec![
                                        QuickAction {
                                            label: "Yes, proceed".to_string(),
                                            command: QuickActionCommand::Slash("__write_confirm_yes".to_string()),
                                        },
                                        QuickAction {
                                            label: "Cancel".to_string(),
                                            command: QuickActionCommand::Slash("__write_confirm_no".to_string()),
                                        },
                                        QuickAction {
                                            label: "Arm session".to_string(),
                                            command: QuickActionCommand::Slash("__write_confirm_arm".to_string()),
                                        },
                                    ]));
                                    continue;
                                }

                                app.busy = true;
                                app.busy_since = Some(Instant::now());
                                app.assistant_streaming_index = None;
                                app.quick_actions = None;
                                app.sticky_actions = None;
                                app.push_log("Calling LLM...");

                                let mut conversation = app.conversation.clone();
                                let tx_updates = tx.clone();
                                let storage_cloned = storage.clone();
                                let executor_cloned = executor.clone();
                                let client_cloned = llm_client.clone();
                                let system_prompt_cloned = system_prompt.clone();
                                let tool_defs_cloned = tool_defs.clone();
                                let text_cloned = text.clone();

                                current_turn = Some(tokio::spawn(async move {
                                    let tx_stream = tx_updates.clone();
                                    let callback: engine::StreamCallback =
                                        Box::new(move |update| {
                                            let _ = tx_stream.send(ReplEvent::Stream(update));
                                        });

                                    let result = engine::run_turn(
                                        &mut conversation,
                                        &text_cloned,
                                        &system_prompt_cloned,
                                        &config,
                                        &client_cloned,
                                        &storage_cloned,
                                        &executor_cloned,
                                        &tool_defs_cloned,
                                        &callback,
                                    )
                                    .await;

                                    let _ = tx_updates.send(ReplEvent::TurnComplete {
                                        conversation,
                                        result,
                                    });
                                }));
                            }
                            InputCommand::Empty => {}
                            other => {
                                handle_slash_command(
                                    &mut app,
                                    other,
                                    &storage,
                                    &executor,
                                    &mut llm_client,
                                    &mut active_llm_config,
                                )
                                .await?;
                                // Restore action bar after view-only slash commands so user
                                // can switch between views (e.g. View YAML → View DAG).
                                // If the command set its own bar (e.g. /run), keep that instead.
                                if app.quick_actions.is_none() {
                                    app.quick_actions = app.sticky_actions.clone();
                                }
                            }
                        }
                    } else if let KeyCode::Char(ch) = key.code {
                        if !app.busy {
                            app.quick_actions = None;
                            app.input.push(ch);
                            app.refresh_autocomplete();
                        }
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if last_tick.elapsed() >= Duration::from_millis(120) {
            app.spinner_index = app.spinner_index.wrapping_add(1);
            last_tick = Instant::now();
        }

        if app.should_quit {
            if let Some(handle) = current_turn.take() {
                handle.abort();
            }
            break;
        }
    }

    teardown_terminal(&mut terminal)?;
    Ok(())
}

/// Extra key handling placeholder for keys not explicitly consumed in loop.
pub fn handle_key_event(_app: &mut ReplApp, _key: KeyEvent) {}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn test_app() -> ReplApp {
        ReplApp::new(
            "session-1".to_string(),
            "gpt-test".to_string(),
            Conversation::new(MAX_CONTEXT_MESSAGES),
            0,
        )
    }

    fn render_lines(app: &ReplApp, width: u16, height: u16) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                let mut line = String::new();
                for x in 0..buffer.area.width {
                    line.push_str(buffer[(x, y)].symbol());
                }
                line.trim_end().to_string()
            })
            .collect()
    }

    #[test]
    fn load_session_restores_persisted_usage_totals() {
        let mut app = test_app();
        let stored = vec![
            ReplMessage {
                id: "msg-1".to_string(),
                session_id: "session-1".to_string(),
                role: "assistant".to_string(),
                content: "first".to_string(),
                token_count: Some(42),
                run_id: None,
                redacted: false,
                created_at: chrono::Utc::now(),
            },
            ReplMessage {
                id: "msg-2".to_string(),
                session_id: "session-1".to_string(),
                role: "assistant".to_string(),
                content: "second".to_string(),
                token_count: None,
                run_id: None,
                redacted: false,
                created_at: chrono::Utc::now(),
            },
        ];

        load_session_into_app(&mut app, &stored);

        assert_eq!(app.session_usage.persisted_total_tokens, 42);
        assert_eq!(app.session_usage.turns_with_usage, 1);
        assert_eq!(app.session_usage.turns_without_usage, 1);
    }

    #[test]
    fn busy_state_starts_in_planning() {
        let app = test_app();

        let state = build_busy_state_view(&app);

        assert_eq!(state.stage, BusyStage::Planning);
        assert_eq!(state.headline, "Planning next step");
        assert_eq!(
            state.detail,
            "Reading your prompt and choosing the first action."
        );
        assert_eq!(
            busy_progress_text(state.stage, "◐"),
            "plan ◐  ->  tools ○  ->  reply ○"
        );
    }

    #[test]
    fn busy_state_shows_active_tool_work() {
        let mut app = test_app();
        app.tool_start_times
            .insert("r8r_generate".to_string(), Instant::now());
        app.turn_outcome
            .tool_durations
            .push("r8r_list_workflows 120ms".to_string());

        let state = build_busy_state_view(&app);

        assert_eq!(state.stage, BusyStage::RunningTools);
        assert_eq!(state.headline, "Running r8r_generate");
        assert_eq!(state.detail, "r8r_generate is currently in flight.");
        assert_eq!(
            state.support.as_deref(),
            Some("Latest finished: r8r_list_workflows 120ms")
        );
    }

    #[test]
    fn busy_state_switches_to_drafting_when_streaming() {
        let mut app = test_app();
        app.turn_outcome
            .tool_durations
            .push("r8r_generate 1.4s".to_string());
        app.messages.push(DisplayMessage {
            kind: MessageKind::Assistant,
            text: "partial".to_string(),
        });
        app.assistant_streaming_index = Some(0);

        let state = build_busy_state_view(&app);

        assert_eq!(state.stage, BusyStage::Drafting);
        assert_eq!(state.headline, "Drafting reply");
        assert_eq!(
            state.detail,
            "Turning 1 tool result into the final response."
        );
        assert_eq!(
            state.support.as_deref(),
            Some("Latest finished: r8r_generate 1.4s")
        );
        assert_eq!(
            busy_progress_text(state.stage, "◓"),
            "plan ✓  ->  tools ✓  ->  reply ◓"
        );
    }

    #[test]
    fn busy_progress_text_compacts_for_narrow_width() {
        assert_eq!(
            busy_progress_text_for_width(BusyStage::Planning, "◐", 14),
            "◐ 1/3 planning"
        );
        assert_eq!(
            busy_progress_text_for_width(BusyStage::RunningTools, "◑", 12),
            "◑ 2/3 tools"
        );
    }

    #[test]
    fn busy_input_lines_wrap_long_tool_names() {
        let mut app = test_app();
        app.busy_since = Some(Instant::now());
        app.tool_start_times.insert(
            "r8r_generate_extremely_long_tool_name".to_string(),
            Instant::now(),
        );

        let lines = build_busy_input_lines(&app, "◐", 16);
        let rendered: Vec<String> = lines.into_iter().map(|line| line.to_string()).collect();

        assert!(rendered.iter().all(|line| line.chars().count() <= 16));
        assert!(rendered.iter().any(|line| line.ends_with('…')));
    }

    #[test]
    fn busy_render_uses_compact_copy_on_narrow_width() {
        let mut app = test_app();
        app.busy = true;
        app.busy_since = Some(Instant::now());
        app.spinner_index = 1;
        app.tool_start_times.insert(
            "r8r_generate_extremely_long_tool_name".to_string(),
            Instant::now(),
        );

        let rendered = render_lines(&app, 32, 16).join("\n");

        assert!(rendered.contains("2/3 tools"));
        assert!(rendered.contains("Ctrl+C cancels."));
        assert!(!rendered.contains("Ctrl+C cancels the current turn."));
    }
}
