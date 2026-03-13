use serde::Serialize;

/// Output mode — human-readable text or machine-readable JSON.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputMode {
    Human,
    Json,
}

/// Abstraction for CLI command output.
///
/// Commands use this instead of raw `println!` to support both
/// human-readable tables and `--json` structured output.
pub struct Output {
    mode: OutputMode,
}

impl Output {
    pub fn new(mode: OutputMode) -> Self {
        Self { mode }
    }

    pub fn is_json(&self) -> bool {
        matches!(self.mode, OutputMode::Json)
    }

    /// Print a success message.
    /// Human: "✓ {msg}"    JSON: {"ok": true, "message": "..."}
    pub fn success(&self, msg: &str) {
        match self.mode {
            OutputMode::Human => println!("\u{2713} {}", msg),
            OutputMode::Json => {
                let obj = serde_json::json!({"ok": true, "message": msg});
                println!("{}", obj);
            }
        }
    }

    /// Print a list of items.
    /// Human: formatted table with headers.   JSON: array of objects.
    pub fn list<T: Serialize>(
        &self,
        headers: &[&str],
        rows: &[T],
        format_row: fn(&T) -> Vec<String>,
    ) {
        match self.mode {
            OutputMode::Human => {
                if rows.is_empty() {
                    return;
                }
                let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
                let formatted: Vec<Vec<String>> = rows.iter().map(format_row).collect();
                for row in &formatted {
                    for (i, cell) in row.iter().enumerate() {
                        if i < widths.len() {
                            widths[i] = widths[i].max(cell.len());
                        }
                    }
                }
                let header_line: String = headers
                    .iter()
                    .zip(&widths)
                    .map(|(h, w)| format!("{:<width$}", h, width = w))
                    .collect::<Vec<_>>()
                    .join("  ");
                println!("{}", header_line);
                println!("{}", "\u{2500}".repeat(header_line.len()));
                for row in &formatted {
                    let line: String = row
                        .iter()
                        .zip(&widths)
                        .map(|(cell, w)| format!("{:<width$}", cell, width = w))
                        .collect::<Vec<_>>()
                        .join("  ");
                    println!("{}", line);
                }
            }
            OutputMode::Json => {
                let json = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
                println!("{}", json);
            }
        }
    }

    /// Print a single item.
    /// Human: custom format.   JSON: serialized object.
    pub fn item<T: Serialize>(&self, item: &T, format_human: fn(&T) -> String) {
        match self.mode {
            OutputMode::Human => println!("{}", format_human(item)),
            OutputMode::Json => {
                let json = serde_json::to_string_pretty(item)
                    .unwrap_or_else(|_| "{}".into());
                println!("{}", json);
            }
        }
    }

    /// Print contextual next-step hints.
    /// Suppressed in JSON mode. Printed to stderr so stdout stays clean for piping.
    pub fn suggest(&self, hints: &[(&str, &str)]) {
        if self.is_json() {
            return;
        }
        eprintln!();
        eprintln!("  Next steps:");
        for (cmd, desc) in hints {
            eprintln!("    {}  \u{2014} {}", cmd, desc);
        }
    }
}
