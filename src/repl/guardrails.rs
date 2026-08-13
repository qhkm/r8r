/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! REPL guardrails: tool permission matrix + budget enforcement.
//!
//! Two independent gates:
//! - **Tool matrix**: allowlist/denylist of tool names. A tool is executed
//!   only if it passes `check_tool`. Blocked tools return a JSON error to the
//!   LLM so it can adapt (try a different tool) instead of failing hard.
//! - **Cost budget**: `max_cost_per_turn` / `max_cost_per_session` in USD.
//!   Usage from every LLM call is accumulated; once the turn budget is
//!   exceeded the agentic loop stops and reports the limit.
//!
//! Config comes from `config.toml` (`repl.guardrails`) and/or CLI flags
//! (`r8r chat --max-cost 2 --deny-tool r8r_approve`).

use crate::llm::LlmUsage;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// File/CLI configuration for REPL guardrails.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuardrailsConfig {
    /// Allowlist of tool names. When non-empty, only these tools are allowed.
    #[serde(default)]
    pub allow_tools: Vec<String>,
    /// Denylist of tool names. These are always blocked, even if in allowlist.
    #[serde(default)]
    pub deny_tools: Vec<String>,
    /// Max LLM cost per turn in USD.
    #[serde(default)]
    pub max_cost_per_turn: Option<f64>,
    /// Max LLM cost per session in USD.
    #[serde(default)]
    pub max_cost_per_session: Option<f64>,
}

/// Runtime guardrail state shared between the TUI and the agentic loop.
#[derive(Debug, Clone)]
pub struct ReplGuardrails {
    allow: Option<HashSet<String>>,
    deny: HashSet<String>,
    max_cost_per_turn: Option<f64>,
    max_cost_per_session: Option<f64>,
    /// Accumulated cost (USD) for the in-flight turn.
    pub turn_cost: f64,
    /// Accumulated cost (USD) across the whole session.
    pub session_cost: f64,
}

/// Estimated USD cost for an LLM call. Returns None for unknown models
/// (e.g. local Ollama), which are treated as free for budgeting purposes.
pub fn estimate_cost(
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

impl ReplGuardrails {
    /// Build from config defaults.
    pub fn from_config(cfg: &GuardrailsConfig) -> Self {
        Self {
            allow: if cfg.allow_tools.is_empty() {
                None
            } else {
                Some(cfg.allow_tools.iter().cloned().collect())
            },
            deny: cfg.deny_tools.iter().cloned().collect(),
            max_cost_per_turn: cfg.max_cost_per_turn,
            max_cost_per_session: cfg.max_cost_per_session,
            turn_cost: 0.0,
            session_cost: 0.0,
        }
    }

    /// Whether the tool is permitted by the matrix.
    pub fn check_tool(&self, name: &str) -> Result<(), String> {
        if self.deny.contains(name) {
            return Err(format!(
                "Tool '{}' is blocked by the REPL guardrail denylist.",
                name
            ));
        }
        if let Some(allow) = &self.allow {
            if !allow.contains(name) {
                return Err(format!(
                    "Tool '{}' is not in the guardrail allowlist. Allowed: {}",
                    name,
                    allow.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
        }
        Ok(())
    }

    /// Record LLM usage, accumulating per-turn and per-session cost.
    /// Returns an error message when a budget is exceeded.
    pub fn record_usage(&mut self, model: &str, usage: &LlmUsage) -> Result<(), String> {
        let Some(cost) = estimate_cost(model, usage.prompt_tokens, usage.completion_tokens) else {
            return Ok(());
        };
        self.turn_cost += cost;
        self.session_cost += cost;
        if let Some(max) = self.max_cost_per_turn {
            if self.turn_cost > max {
                return Err(format!(
                    "Turn cost limit reached: ${:.3} used this turn (max ${:.2}).",
                    self.turn_cost, max
                ));
            }
        }
        if let Some(max) = self.max_cost_per_session {
            if self.session_cost > max {
                return Err(format!(
                    "Session cost limit reached: ${:.3} used total (max ${:.2}).",
                    self.session_cost, max
                ));
            }
        }
        Ok(())
    }

    /// Start a new turn (resets per-turn accumulation).
    pub fn begin_turn(&mut self) {
        self.turn_cost = 0.0;
    }

    /// Runtime allow/deny updates (from slash commands).
    pub fn allow_tool(&mut self, name: &str) {
        self.deny.remove(name);
        self.allow
            .get_or_insert_with(HashSet::new)
            .insert(name.to_string());
    }

    pub fn deny_tool(&mut self, name: &str) {
        self.deny.insert(name.to_string());
        self.allow.as_mut().map(|a| a.remove(name));
    }

    /// Render current guardrail state for /guardrails.
    pub fn status_summary(&self) -> String {
        let mut s = String::from("Guardrail state:\n");
        match &self.allow {
            Some(a) if !a.is_empty() => s.push_str(&format!(
                "  Allowlist: {}\n",
                a.iter().cloned().collect::<Vec<_>>().join(", ")
            )),
            Some(_) => s.push_str("  Allowlist: (configured, empty — all tools blocked)\n"),
            None => s.push_str("  Allowlist: none (all tools allowed)\n"),
        }
        s.push_str(&format!(
            "  Denylist: {}\n",
            if self.deny.is_empty() {
                "(empty)".to_string()
            } else {
                self.deny.iter().cloned().collect::<Vec<_>>().join(", ")
            }
        ));
        s.push_str(&format!(
            "  Max cost/turn:    {}\n",
            self.max_cost_per_turn
                .map(|v| format!("${v:.2}"))
                .unwrap_or_else(|| "unlimited".into())
        ));
        s.push_str(&format!(
            "  Max cost/session: {}\n",
            self.max_cost_per_session
                .map(|v| format!("${v:.2}"))
                .unwrap_or_else(|| "unlimited".into())
        ));
        s.push_str(&format!("  Spent this turn:   ${:.4}\n", self.turn_cost));
        s.push_str(&format!("  Spent total:       ${:.4}\n", self.session_cost));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmUsage;

    fn usage(prompt: u64, completion: u64) -> LlmUsage {
        LlmUsage {
            prompt_tokens: Some(prompt),
            completion_tokens: Some(completion),
        }
    }

    #[test]
    fn empty_config_allows_everything() {
        let g = ReplGuardrails::from_config(&GuardrailsConfig::default());
        assert!(g.check_tool("r8r_execute").is_ok());
        assert!(g.check_tool("anything").is_ok());
    }

    #[test]
    fn deny_list_blocks() {
        let cfg = GuardrailsConfig {
            deny_tools: vec!["r8r_approve".into()],
            ..Default::default()
        };
        let g = ReplGuardrails::from_config(&cfg);
        assert!(g.check_tool("r8r_approve").is_err());
        assert!(g.check_tool("r8r_execute").is_ok());
    }

    #[test]
    fn allow_list_restricts_and_deny_wins() {
        let cfg = GuardrailsConfig {
            allow_tools: vec!["r8r_list_workflows".into(), "r8r_execute".into()],
            deny_tools: vec!["r8r_execute".into()],
            ..Default::default()
        };
        let g = ReplGuardrails::from_config(&cfg);
        assert!(g.check_tool("r8r_list_workflows").is_ok());
        assert!(g.check_tool("r8r_execute").is_err()); // deny wins
        assert!(g.check_tool("r8r_validate").is_err()); // not allowed
    }

    #[test]
    fn turn_budget_blocks() {
        let cfg = GuardrailsConfig {
            max_cost_per_turn: Some(0.01),
            ..Default::default()
        };
        let mut g = ReplGuardrails::from_config(&cfg);
        g.begin_turn();
        // ~10k prompt tokens on gpt-4o-mini ≈ $0.0015; stay under budget.
        assert!(g.record_usage("gpt-4o-mini", &usage(10_000, 500)).is_ok());
        // ~200k prompt tokens ≈ $0.03 > $0.01 budget.
        assert!(g
            .record_usage("gpt-4o-mini", &usage(200_000, 10_000))
            .is_err());
    }

    #[test]
    fn session_budget_counts_across_turns() {
        let cfg = GuardrailsConfig {
            max_cost_per_session: Some(0.01),
            ..Default::default()
        };
        let mut g = ReplGuardrails::from_config(&cfg);
        g.begin_turn();
        assert!(g.record_usage("gpt-4o-mini", &usage(10_000, 500)).is_ok());
        g.begin_turn();
        assert!(g
            .record_usage("gpt-4o-mini", &usage(90_000, 5_000))
            .is_err());
    }

    #[test]
    fn runtime_allow_deny() {
        let mut g = ReplGuardrails::from_config(&GuardrailsConfig::default());
        g.deny_tool("r8r_test");
        assert!(g.check_tool("r8r_test").is_err());
        g.allow_tool("r8r_test");
        assert!(g.check_tool("r8r_test").is_ok());
    }

    #[test]
    fn unknown_model_is_free() {
        let mut g = ReplGuardrails::from_config(&GuardrailsConfig {
            max_cost_per_turn: Some(0.0001),
            ..Default::default()
        });
        g.begin_turn();
        assert!(g
            .record_usage("ollama/llama3", &usage(1_000_000, 1_000_000))
            .is_ok());
    }
}
