/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! REPL system prompt builder.
//!
//! Auto-generates the system prompt from r8r internals.

use crate::llm::LlmProvider;

/// Build the REPL system prompt from runtime context.
pub fn build_repl_system_prompt(
    tool_names: &[&str],
    node_types: &[&str],
    credential_names: &[&str],
    workflow_names: &[&str],
    provider: LlmProvider,
) -> String {
    let mut prompt = String::new();

    prompt.push_str(
        "You are the r8r workflow assistant. You help users create, manage, run, and debug \
         workflows using the tools provided.\n\n\
         RULES:\n\
         - Only use the tools provided. Never suggest manual file edits or CLI commands.\n\
         - Only reference node types from the NODE CATALOG below.\n\
         - Only reference credentials from the CONFIGURED CREDENTIALS below.\n\
         - When creating workflows, always validate before saving.\n\
         - If unsure about a workflow's structure, use r8r_discover first.\n\
         - Be concise. Show results clearly. Summarize tool outputs, don't dump raw JSON.\n\
         - When generating workflow YAML, use {{ env.VAR }} for secrets, never hardcode them.\n\
         - Use {{ nodes.id.output.field }} for inter-node references.\n\n",
    );

    if !tool_names.is_empty() {
        prompt.push_str("AVAILABLE TOOLS:\n");
        for name in tool_names {
            prompt.push_str(&format!("- {}\n", name));
        }
        prompt.push('\n');
    }

    if !node_types.is_empty() {
        prompt.push_str("NODE CATALOG (available node types for workflows):\n");
        for node_type in node_types {
            prompt.push_str(&format!("- {}\n", node_type));
        }
        prompt.push('\n');
    }

    if !credential_names.is_empty() {
        prompt.push_str("CONFIGURED CREDENTIALS:\n");
        for name in credential_names {
            prompt.push_str(&format!("- {}\n", name));
        }
        prompt.push('\n');
    }

    if !workflow_names.is_empty() {
        prompt.push_str("EXISTING WORKFLOWS:\n");
        for name in workflow_names {
            prompt.push_str(&format!("- {}\n", name));
        }
        prompt.push('\n');
    }

    if provider == LlmProvider::Ollama {
        prompt.push_str(
            "TOOL CALLING FORMAT:\n\
             When you need to call a tool, output EXACTLY:\n\
             <tool_call>{\"name\": \"tool_name\", \"args\": {\"param\": \"value\"}}</tool_call>\n\
             Do not add any text before or after the tag. Wait for the result before continuing.\n\
             You may call one tool at a time.\n\n",
        );
    }

    prompt
}
