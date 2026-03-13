// Elm-style error diagnostics with fuzzy matching and recovery hints.

use crate::cli::output::OutputMode;
use crate::error::Error;

/// Classifies the specific kind of error for targeted hints.
#[derive(Debug)]
pub enum DiagnosticKind {
    WorkflowNotFound { name: String },
    NoLlmConfigured,
    YamlParseError { line: Option<usize>, column: Option<usize> },
    CredentialMissing { service: String },
    ConnectionRefused { url: String },
    DbNotInitialized,
    Generic,
}

/// Rich diagnostic with hints and suggestions for the user.
#[derive(Debug)]
pub struct Diagnostic {
    pub code: &'static str,
    pub kind: DiagnosticKind,
    pub message: String,
    /// Short "try this command" lines.
    pub hints: Vec<String>,
    /// Longer explanatory suggestions.
    pub suggestions: Vec<String>,
}

impl Diagnostic {
    /// Build a `Diagnostic` from an `Error`, enriching it with fuzzy-matched
    /// workflow candidates when applicable.
    pub fn from_error(err: &Error, candidates: &[&str]) -> Self {
        let code = err.code();
        let message = err.to_string();
        let kind = classify_error(code, &message);
        let (hints, suggestions) = enrich(&kind, candidates);
        Self { code, kind, message, hints, suggestions }
    }

    /// Render according to the requested output mode.
    pub fn render(&self, mode: OutputMode) {
        match mode {
            OutputMode::Human => self.print(),
            OutputMode::Json => self.print_json(),
        }
    }

    /// Rustc-style human output to stderr (no box-drawing characters).
    pub fn print(&self) {
        eprintln!("error[{}]: {}", self.code, self.message);
        if !self.hints.is_empty() {
            eprintln!();
            eprintln!("  hints:");
            for hint in &self.hints {
                eprintln!("    {}", hint);
            }
        }
        if !self.suggestions.is_empty() {
            eprintln!();
            for suggestion in &self.suggestions {
                eprintln!("  note: {}", suggestion);
            }
        }
    }

    /// JSON output to stderr.
    pub fn print_json(&self) {
        let obj = serde_json::json!({
            "error": {
                "code": self.code,
                "message": self.message,
                "hints": self.hints,
                "suggestions": self.suggestions,
            }
        });
        eprintln!("{}", obj);
    }
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Classify an error into a `DiagnosticKind` based on the error code and
/// message content.
pub fn classify_error(code: &str, message: &str) -> DiagnosticKind {
    let msg_lower = message.to_lowercase();

    match code {
        "WORKFLOW_ERROR" if msg_lower.contains("not found") => {
            let name = extract_quoted_name(message)
                .unwrap_or_else(|| "unknown".to_string());
            DiagnosticKind::WorkflowNotFound { name }
        }

        "CONFIG_ERROR" if msg_lower.contains("llm")
            || msg_lower.contains("provider")
            || msg_lower.contains("no llm") =>
        {
            DiagnosticKind::NoLlmConfigured
        }

        "CONFIG_ERROR" if msg_lower.contains("credential") => {
            let service = extract_quoted_name(message)
                .unwrap_or_else(|| "unknown".to_string());
            DiagnosticKind::CredentialMissing { service }
        }

        "YAML_ERROR" => {
            let (line, column) = extract_yaml_position(message);
            DiagnosticKind::YamlParseError { line, column }
        }

        "HTTP_ERROR"
            if msg_lower.contains("connection refused") || msg_lower.contains("connect") =>
        {
            let url = extract_quoted_name(message)
                .unwrap_or_else(|| "unknown".to_string());
            DiagnosticKind::ConnectionRefused { url }
        }

        "DATABASE_ERROR" | "STORAGE_ERROR"
            if msg_lower.contains("no such table") || msg_lower.contains("not initialized") =>
        {
            DiagnosticKind::DbNotInitialized
        }

        _ => DiagnosticKind::Generic,
    }
}

/// Fuzzy-match `input` against `candidates` using Jaro-Winkler similarity.
/// Returns candidates whose similarity score is at or above `threshold`.
pub fn fuzzy_match(input: &str, candidates: &[&str], threshold: f64) -> Vec<String> {
    candidates
        .iter()
        .filter(|&&c| strsim::jaro_winkler(input, c) >= threshold)
        .map(|&c| c.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Extract the first single-quoted name from `message`, e.g. `'my-wf'` → `"my-wf"`.
fn extract_quoted_name(message: &str) -> Option<String> {
    let start = message.find('\'')?;
    let rest = &message[start + 1..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

/// Extract `line N column M` from a YAML error message.
/// Returns `(line, column)` — either may be `None` if not found.
fn extract_yaml_position(message: &str) -> (Option<usize>, Option<usize>) {
    // Match "line N column M" (case-insensitive)
    let re_line = regex_lite::Regex::new(r"line (\d+)").unwrap();
    let re_col = regex_lite::Regex::new(r"column (\d+)").unwrap();

    let line = re_line
        .captures(message)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<usize>().ok());

    let column = re_col
        .captures(message)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<usize>().ok());

    (line, column)
}

/// Build hints and suggestions for a given `DiagnosticKind`.
fn enrich(kind: &DiagnosticKind, candidates: &[&str]) -> (Vec<String>, Vec<String>) {
    match kind {
        DiagnosticKind::WorkflowNotFound { name } => {
            let mut hints = Vec::new();
            let similar = fuzzy_match(name, candidates, 0.6);
            if !similar.is_empty() {
                hints.push(format!("Did you mean: {}", similar.join(", ")));
            }
            hints.push("r8r workflows list".to_string());
            hints.push("r8r create".to_string());
            (hints, Vec::new())
        }

        DiagnosticKind::NoLlmConfigured => (
            vec![
                "r8r credentials set openai --key <YOUR_KEY>".to_string(),
                "ollama serve".to_string(),
                "r8r init".to_string(),
            ],
            Vec::new(),
        ),

        DiagnosticKind::YamlParseError { line, column } => {
            let location = match (line, column) {
                (Some(l), Some(c)) => format!(" at line {}, column {}", l, c),
                (Some(l), None) => format!(" at line {}", l),
                _ => String::new(),
            };
            (
                vec![format!("r8r lint <file>  -- check YAML syntax{}", location)],
                Vec::new(),
            )
        }

        DiagnosticKind::CredentialMissing { service } => (
            vec![format!("r8r credentials set {}", service)],
            Vec::new(),
        ),

        DiagnosticKind::ConnectionRefused { url } => (
            vec![
                "r8r server".to_string(),
                format!("Check port — could not connect to {}", url),
            ],
            Vec::new(),
        ),

        DiagnosticKind::DbNotInitialized => {
            (vec!["r8r doctor".to_string()], Vec::new())
        }

        DiagnosticKind::Generic => (Vec::new(), Vec::new()),
    }
}
