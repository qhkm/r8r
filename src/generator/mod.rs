//! Prompt-to-workflow generation.
//!
//! Generate r8r workflow YAML from natural language prompts using LLMs.

pub mod context;
pub mod prompt;

pub use context::GeneratorContext;

use crate::error::{Error, Result};
use crate::llm::{self, LlmConfig, LlmProvider};
use crate::workflow::{parse_workflow, validate_workflow};
use reqwest::Client;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of a workflow generation or refinement call.
pub struct GenerateResult {
    /// The generated (or refined) workflow YAML.
    pub yaml: String,
    /// Human-readable summary of the workflow.
    pub summary: String,
    /// Whether the YAML passed validation.
    pub valid: bool,
    /// Validation errors, if any (empty when `valid` is true).
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// LLM config resolution
// ---------------------------------------------------------------------------

/// Resolve LLM configuration from environment variables.
///
/// Priority order for each field:
/// - `R8R_GENERATOR_*` → `R8R_AGENT_*` → built-in default
///
/// Returns an error if the resolved provider is not Ollama and no API key is
/// available.
pub async fn resolve_llm_config() -> Result<LlmConfig> {
    // --- Provider ---
    let provider_str = std::env::var("R8R_GENERATOR_PROVIDER")
        .or_else(|_| std::env::var("R8R_AGENT_PROVIDER"))
        .unwrap_or_else(|_| "openai".to_string());

    let provider = match provider_str.to_lowercase().as_str() {
        "anthropic" => LlmProvider::Anthropic,
        "ollama" => LlmProvider::Ollama,
        "custom" => LlmProvider::Custom,
        _ => LlmProvider::Openai,
    };

    // --- Model (optional) ---
    let model = std::env::var("R8R_GENERATOR_MODEL")
        .or_else(|_| std::env::var("R8R_AGENT_MODEL"))
        .ok();

    // --- API key ---
    let api_key = std::env::var("R8R_GENERATOR_API_KEY")
        .or_else(|_| std::env::var("R8R_AGENT_API_KEY"))
        .ok();

    // Require API key for all providers except Ollama.
    if provider != LlmProvider::Ollama && api_key.is_none() {
        return Err(Error::Config(
            "Generator LLM API key is required. \
             Set R8R_GENERATOR_API_KEY or R8R_AGENT_API_KEY."
                .to_string(),
        ));
    }

    // --- Endpoint (optional override) ---
    let endpoint = std::env::var("R8R_GENERATOR_ENDPOINT")
        .or_else(|_| std::env::var("R8R_AGENT_ENDPOINT"))
        .ok();

    Ok(LlmConfig {
        provider,
        model,
        api_key,
        endpoint,
        temperature: Some(0.3),
        max_tokens: Some(4096),
        timeout_seconds: 60,
    })
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Generate a new workflow from a natural language prompt.
///
/// 1. Resolves LLM configuration from the environment.
/// 2. Builds a generator context (node catalog, credentials, examples).
/// 3. Calls the LLM to produce YAML.
/// 4. Validates the YAML and, if invalid, attempts one auto-fix pass.
pub async fn generate(user_prompt: &str) -> Result<GenerateResult> {
    let llm_config = resolve_llm_config().await?;
    let ctx = GeneratorContext::build().await;
    let client = llm::create_llm_client();

    let system = prompt::build_system_prompt(&ctx);
    let user_msg = prompt::build_create_prompt(user_prompt);

    info!(provider = ?llm_config.provider, "Calling LLM for workflow generation");

    let response = llm::call_llm(&client, &llm_config, Some(&system), &user_msg).await?;
    let yaml = prompt::extract_yaml(&response.content).to_string();

    validate_or_fix(&client, &llm_config, &system, &yaml).await
}

/// Refine an existing workflow given a change description.
///
/// Identical to [`generate`] but uses the refine prompt which includes the
/// current YAML so the LLM can make targeted edits.
pub async fn refine(current_yaml: &str, user_prompt: &str) -> Result<GenerateResult> {
    let llm_config = resolve_llm_config().await?;
    let ctx = GeneratorContext::build().await;
    let client = llm::create_llm_client();

    let system = prompt::build_system_prompt(&ctx);
    let user_msg = prompt::build_refine_prompt(current_yaml, user_prompt);

    info!(provider = ?llm_config.provider, "Calling LLM for workflow refinement");

    let response = llm::call_llm(&client, &llm_config, Some(&system), &user_msg).await?;
    let yaml = prompt::extract_yaml(&response.content).to_string();

    validate_or_fix(&client, &llm_config, &system, &yaml).await
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Validate the YAML and, if it fails, attempt one LLM-assisted fix pass.
///
/// Returns a [`GenerateResult`] with `valid=true` when validation succeeds
/// (either on first attempt or after the fix), or `valid=false` with the
/// collected errors when the fix also fails.
async fn validate_or_fix(
    client: &Client,
    llm_config: &LlmConfig,
    system: &str,
    yaml: &str,
) -> Result<GenerateResult> {
    match try_validate(yaml) {
        Ok(summary) => {
            info!("Generated workflow passed validation: {}", summary);
            Ok(GenerateResult {
                yaml: yaml.to_string(),
                summary,
                valid: true,
                errors: vec![],
            })
        }
        Err(errors) => {
            warn!(
                "Generated workflow failed validation ({} error(s)), attempting auto-fix",
                errors.len()
            );

            let errors_text = errors.join("\n");
            let fix_msg = prompt::build_fix_prompt(yaml, &errors_text);

            match llm::call_llm(client, llm_config, Some(system), &fix_msg).await {
                Ok(fix_response) => {
                    let fixed_yaml = prompt::extract_yaml(&fix_response.content).to_string();

                    match try_validate(&fixed_yaml) {
                        Ok(summary) => {
                            info!("Auto-fixed workflow passed validation: {}", summary);
                            Ok(GenerateResult {
                                yaml: fixed_yaml,
                                summary,
                                valid: true,
                                errors: vec![],
                            })
                        }
                        Err(fix_errors) => {
                            warn!("Auto-fix also failed validation");
                            Ok(GenerateResult {
                                yaml: fixed_yaml,
                                summary: String::new(),
                                valid: false,
                                errors: fix_errors,
                            })
                        }
                    }
                }
                Err(e) => {
                    warn!("Auto-fix LLM call failed: {}", e);
                    Ok(GenerateResult {
                        yaml: yaml.to_string(),
                        summary: String::new(),
                        valid: false,
                        errors,
                    })
                }
            }
        }
    }
}

/// Attempt to parse and validate a workflow YAML string.
///
/// Returns `Ok(summary)` on success where `summary` describes the workflow,
/// or `Err(errors)` with one or more human-readable error messages.
fn try_validate(yaml: &str) -> std::result::Result<String, Vec<String>> {
    // Parse the YAML into a Workflow struct.
    let workflow = match parse_workflow(yaml) {
        Ok(w) => w,
        Err(e) => return Err(vec![format!("Parse error: {}", e)]),
    };

    // Structurally validate the workflow.
    if let Err(e) = validate_workflow(&workflow) {
        return Err(vec![e.to_string()]);
    }

    // Build a human-readable summary.
    let node_types: Vec<String> = workflow
        .nodes
        .iter()
        .map(|n| n.node_type.clone())
        .collect();

    let summary = format!(
        "\"{}\" with {} node{}: {}",
        workflow.name,
        node_types.len(),
        if node_types.len() == 1 { "" } else { "s" },
        node_types.join(" → ")
    );

    Ok(summary)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_validate_valid_yaml() {
        let yaml = "name: test-workflow\nnodes:\n  - id: step1\n    type: transform\n    config:\n      expression: '\"hello\"'\n";
        let result = try_validate(yaml);
        assert!(result.is_ok());
        let summary = result.unwrap();
        assert!(summary.contains("test-workflow"));
    }

    #[test]
    fn test_try_validate_invalid_yaml() {
        let yaml = "this is not yaml: [[[";
        let result = try_validate(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_try_validate_empty_name() {
        let yaml = "name: \nnodes: []";
        let result = try_validate(yaml);
        assert!(result.is_err());
    }
}
