//! Prompt assembly for workflow generation.
//!
//! Provides functions that build LLM prompts for creating, refining, and fixing
//! r8r workflow YAML, as well as a utility to strip markdown fences from responses.

use super::context::GeneratorContext;

/// Build a system prompt that explains the r8r workflow format to the LLM.
///
/// The prompt has up to four parts:
/// 1. Role & hard rules (always present)
/// 2. Available node catalog (always present)
/// 3. Configured credentials (omitted when `ctx.credential_names` is empty)
/// 4. Example workflows (omitted when `ctx.examples` is empty)
pub fn build_system_prompt(ctx: &GeneratorContext) -> String {
    let mut prompt = String::with_capacity(8000);

    // --- Part 1: Role & rules ---
    prompt.push_str(
        "You are a workflow generator for r8r, a workflow automation engine.\n\
         Your job is to generate valid r8r workflow YAML from user descriptions.\n\
         \n\
         RULES:\n\
         - Output ONLY valid YAML. No markdown fences, no explanation, just YAML.\n\
         - Every workflow must have: name, description, triggers, nodes.\n\
         - Use {{ env.VAR_NAME }} for secrets and API keys. NEVER hardcode them.\n\
         - Use {{ nodes.<node-id>.output.<field> }} to reference previous node output.\n\
         - Use {{ input.<field> }} for workflow input parameters.\n\
         - Node IDs must be lowercase with hyphens (e.g., fetch-data, send-alert).\n\
         - Every node except the first must have depends_on.\n\
         - Add retry config for HTTP and external calls.\n",
    );

    // --- Part 2: Node catalog ---
    prompt.push_str("\nAVAILABLE NODE TYPES:\n\n");
    for (name, description) in &ctx.node_catalog {
        prompt.push_str(&format!("- {}: {}\n", name, description));
    }

    // --- Part 3: Credentials (optional) ---
    if !ctx.credential_names.is_empty() {
        prompt.push_str(
            "\nCONFIGURED CREDENTIALS (use these names in credential fields):\n",
        );
        for cred in &ctx.credential_names {
            prompt.push_str(&format!("- {}\n", cred));
        }
    }

    // --- Part 4: Examples (optional) ---
    if !ctx.examples.is_empty() {
        prompt.push_str("\nEXAMPLE WORKFLOWS:\n");
        for (i, example) in ctx.examples.iter().enumerate() {
            prompt.push_str(&format!("\n--- Example {} ---\n", i + 1));
            prompt.push_str(example);
            if !example.ends_with('\n') {
                prompt.push('\n');
            }
        }
    }

    prompt
}

/// Build a user-turn prompt for generating a brand-new workflow.
pub fn build_create_prompt(user_prompt: &str) -> String {
    format!(
        "Generate an r8r workflow YAML for the following:\n\
         \n\
         {}\n\
         \n\
         Output ONLY the YAML, nothing else.",
        user_prompt
    )
}

/// Build a user-turn prompt for refining an existing workflow.
pub fn build_refine_prompt(current_yaml: &str, user_prompt: &str) -> String {
    format!(
        "Here is an existing r8r workflow:\n\
         \n\
         {}\n\
         \n\
         The user wants to change it as follows:\n\
         \n\
         {}\n\
         \n\
         Output the complete updated YAML, nothing else.",
        current_yaml, user_prompt
    )
}

/// Build a user-turn prompt for fixing validation errors in a workflow.
pub fn build_fix_prompt(yaml: &str, errors: &str) -> String {
    format!(
        "The following r8r workflow YAML has validation errors:\n\
         \n\
         {}\n\
         \n\
         Errors:\n\
         {}\n\
         \n\
         Fix the errors and output the corrected YAML only.",
        yaml, errors
    )
}

/// Strip markdown code fences from an LLM response.
///
/// Handles:
/// - ` ```yaml\n...\n``` ` → inner content (trimmed)
/// - ` ```\n...\n``` ` → inner content (trimmed)
/// - Plain text → trimmed input
pub fn extract_yaml(response: &str) -> &str {
    let trimmed = response.trim();

    // Try ```yaml fence first, then plain ``` fence.
    for prefix in &["```yaml\n", "```\n"] {
        if let Some(inner) = trimmed.strip_prefix(prefix) {
            if let Some(end) = inner.rfind("\n```") {
                return inner[..end].trim();
            }
            // Fence opened but never closed — return everything after the prefix.
            return inner.trim();
        }
    }

    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> GeneratorContext {
        GeneratorContext {
            node_catalog: vec![
                ("http".to_string(), "Make HTTP requests".to_string()),
                ("transform".to_string(), "Transform data with Rhai".to_string()),
                ("agent".to_string(), "Call LLM providers".to_string()),
            ],
            credential_names: vec!["openai".to_string(), "slack-webhook".to_string()],
            examples: vec![
                "name: hello\nnodes:\n  - id: greet\n    type: transform".to_string(),
            ],
        }
    }

    #[test]
    fn test_system_prompt_includes_all_nodes() {
        let ctx = test_context();
        let prompt = build_system_prompt(&ctx);

        assert!(prompt.contains("http"), "expected 'http' in prompt");
        assert!(prompt.contains("transform"), "expected 'transform' in prompt");
        assert!(prompt.contains("agent"), "expected 'agent' in prompt");

        assert!(prompt.contains("Make HTTP requests"));
        assert!(prompt.contains("Transform data with Rhai"));
        assert!(prompt.contains("Call LLM providers"));
    }

    #[test]
    fn test_system_prompt_includes_credentials() {
        let ctx = test_context();
        let prompt = build_system_prompt(&ctx);

        assert!(
            prompt.contains("CONFIGURED CREDENTIALS"),
            "expected credential section header"
        );
        assert!(prompt.contains("openai"), "expected 'openai' credential");
        assert!(
            prompt.contains("slack-webhook"),
            "expected 'slack-webhook' credential"
        );
    }

    #[test]
    fn test_system_prompt_includes_examples() {
        let ctx = test_context();
        let prompt = build_system_prompt(&ctx);

        assert!(
            prompt.contains("Example 1"),
            "expected 'Example 1' marker in prompt"
        );
        assert!(
            prompt.contains("name: hello"),
            "expected example YAML content in prompt"
        );
    }

    #[test]
    fn test_system_prompt_no_credentials() {
        let mut ctx = test_context();
        ctx.credential_names = vec![];
        let prompt = build_system_prompt(&ctx);

        assert!(
            !prompt.contains("CONFIGURED CREDENTIALS"),
            "credential section should be absent when no credentials"
        );
    }

    #[test]
    fn test_create_prompt() {
        let user_prompt = "Send a Slack message whenever a GitHub issue is opened";
        let prompt = build_create_prompt(user_prompt);

        assert!(
            prompt.contains(user_prompt),
            "user prompt text should appear in create prompt"
        );
        assert!(prompt.contains("Output ONLY the YAML"));
    }

    #[test]
    fn test_refine_prompt_includes_yaml_and_change() {
        let current_yaml = "name: my-workflow\nnodes: []";
        let change = "Add a step that sends an email after the last node";
        let prompt = build_refine_prompt(current_yaml, change);

        assert!(
            prompt.contains(current_yaml),
            "current YAML should appear in refine prompt"
        );
        assert!(
            prompt.contains(change),
            "change description should appear in refine prompt"
        );
        assert!(prompt.contains("Output the complete updated YAML"));
    }

    #[test]
    fn test_fix_prompt() {
        let yaml = "name: broken\nnodes: []";
        let errors = "triggers is required\nnode 'send' missing depends_on";
        let prompt = build_fix_prompt(yaml, errors);

        assert!(
            prompt.contains(yaml),
            "YAML should appear in fix prompt"
        );
        assert!(
            prompt.contains(errors),
            "errors should appear in fix prompt"
        );
        assert!(prompt.contains("Fix the errors"));
    }

    #[test]
    fn test_extract_yaml_strips_fences() {
        // ```yaml fence
        let with_yaml_fence = "```yaml\nname: hello\nnodes: []\n```";
        assert_eq!(extract_yaml(with_yaml_fence), "name: hello\nnodes: []");

        // plain ``` fence
        let with_plain_fence = "```\nname: hello\nnodes: []\n```";
        assert_eq!(extract_yaml(with_plain_fence), "name: hello\nnodes: []");

        // plain YAML — no fences
        let plain = "name: hello\nnodes: []";
        assert_eq!(extract_yaml(plain), "name: hello\nnodes: []");

        // surrounding whitespace
        let with_whitespace = "  name: hello\nnodes: []  ";
        assert_eq!(extract_yaml(with_whitespace), "name: hello\nnodes: []");
    }
}
