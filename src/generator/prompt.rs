/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
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

    // --- Part 2: Node catalog (name + description + per-node config schema) ---
    prompt.push_str("\nAVAILABLE NODE TYPES (with required config fields):\n");
    for (name, description) in &ctx.node_catalog {
        prompt.push_str(&format!("\n- {}: {}\n", name, description));
        if let Some(schema) = node_config_schema(name) {
            for line in schema.lines() {
                prompt.push_str(&format!("    {}\n", line));
            }
        }
    }

    // --- Part 2.5: Rhai cheatsheet (transform/switch/filter node expression DSL) ---
    prompt.push_str(RHAI_CHEATSHEET);

    // --- Part 3: Credentials (optional) ---
    if !ctx.credential_names.is_empty() {
        prompt.push_str("\nCONFIGURED CREDENTIALS (use these names in credential fields):\n");
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

/// Per-node config schema documentation rendered into the system prompt.
///
/// Returns `None` for node types that do not yet have a curated schema. When `None`,
/// the LLM still sees the node's one-line description from the catalog but no field
/// schema — increasing hallucination risk for that node type. Add coverage as needed.
fn node_config_schema(node_type: &str) -> Option<&'static str> {
    match node_type {
        "transform" => Some(
            "config:\n  \
             expression: <Rhai expression — see RHAI section below>\n  \
             # Input is a JSON STRING in Rhai scope. Use from_json(input) to parse.\n  \
             # Return value (last expression) becomes the node output.",
        ),
        "http" => Some(
            "config:\n  \
             url: <string, required, supports {{ env.X }} and {{ nodes.id.output.field }}>\n  \
             method: <GET|POST|PUT|PATCH|DELETE|HEAD, default GET>\n  \
             headers: <map of string -> string, optional>\n  \
             body: <JSON object, optional>\n  \
             timeout_seconds: <integer, optional, default 30>\n  \
             raw_response: <bool, optional, default false — return body as text not JSON>\n  \
             credential: <string, optional — name from credential store>\n  \
             auth_type: <bearer|basic|api_key|header:NAME, default bearer>\n  \
             # Output: { status: int, headers: map, body: parsed-JSON-or-string }",
        ),
        "agent" => Some(
            "config:\n  \
             prompt: <string, required, supports templates>\n  \
             provider: <openai|anthropic|ollama|custom|zeptoclaw — optional, falls back to global [llm].provider>\n  \
             endpoint: <string, optional — overrides provider default>\n  \
             model: <string, optional — falls back to global [llm].model>\n  \
             credential: <string, optional — defaults to looking up 'openai'/'anthropic'/'llm' in store>\n  \
             response_format: <text|json, default text>\n  \
             json_schema: <JSON Schema object, optional, used when response_format=json>\n  \
             system: <string, optional system prompt>\n  \
             temperature: <float, optional>\n  \
             max_tokens: <integer, optional>\n  \
             timeout_seconds: <integer, optional, default 30>",
        ),
        "limit" => Some(
            "config:\n  \
             field: <string, REQUIRED — template like {{ nodes.fetch.output.body }} pointing to an array>\n  \
             limit: <integer, required>\n  \
             offset: <integer, optional, default 0>\n  \
             # Output: array of up to `limit` items skipping `offset`.",
        ),
        "filter" => Some(
            "config:\n  \
             field: <string, REQUIRED — template pointing to an array>\n  \
             match_mode: <all|any, default all>\n  \
             conditions: <array of { field: <string>, operator: <equals|not_equals|gt|lt|gte|lte|contains|regex>, value: <any> }>\n  \
             # Output: filtered array.",
        ),
        "sort" => Some(
            "config:\n  \
             field: <string, REQUIRED — template pointing to an array>\n  \
             by: <array of { field: <string>, order: <asc|desc, default asc> }>\n  \
             # Output: sorted array.",
        ),
        "set" => Some(
            "config:\n  \
             values: <map of string -> any — values to set in the output>\n  \
             # Output: object with the specified key/value pairs.",
        ),
        "if" => Some(
            "config:\n  \
             conditions: <array of { field: <string>, operator: <equals|not_equals|gt|lt|gte|lte|contains|regex>, value: <any> }, required>\n  \
             true_branch: <node-id string, required>\n  \
             false_branch: <node-id string, required>\n  \
             match_mode: <all|any, default all>\n  \
             # Conditions reference input.* or node outputs via templates.",
        ),
        "switch" => Some(
            "config:\n  \
             expression: <string, required — usually a template like {{ input.type }}>\n  \
             cases: <array of { value: <any>, output: <any>, label: <string optional> }>\n  \
             default: <any, optional>\n  \
             mode: <first|all, default first>",
        ),
        "dedupe" => Some(
            "config:\n  \
             field: <string, REQUIRED — template pointing to an array>\n  \
             key: <string, optional — dedupe by this sub-field rather than full value>\n  \
             # Output: array with duplicates removed.",
        ),
        "aggregate" => Some(
            "config:\n  \
             mode: <array|object|first|last, default array>\n  \
             field: <string, optional — template pointing to an array>\n  \
             key_field: <string, required for mode=object>\n  \
             value_field: <string, required for mode=object>\n  \
             # Output: aggregated value.",
        ),
        "merge" => Some(
            "config:\n  \
             mode: <append|combine, default append>\n  \
             # Inputs come from multiple `depends_on` nodes; output is the merged result.",
        ),
        "split" => Some(
            "config:\n  \
             field: <string, REQUIRED — template pointing to an array>\n  \
             batch_size: <integer, required>\n  \
             # Output: array of batches.",
        ),
        "wait" => Some(
            "config:\n  \
             seconds: <integer, required — pause this many seconds>",
        ),
        "approval" => Some(
            "config:\n  \
             message: <string, required — shown to approver>\n  \
             timeout_seconds: <integer, optional>\n  \
             # Workflow pauses until approved/denied via r8r CLI or API.",
        ),
        "slack" => Some(
            "config:\n  \
             channel: <string, required — #channel or channel ID>\n  \
             text: <string, optional>\n  \
             blocks: <array of Slack Block Kit blocks, optional>\n  \
             token: <string, optional>\n  \
             webhook_url: <string, optional — use either token or webhook_url>\n  \
             credential: <string, optional — credential store name>",
        ),
        "email" => Some(
            "config:\n  \
             to: <string or array of strings, required>\n  \
             subject: <string, required>\n  \
             body: <string, required>\n  \
             credential: <string, required — SMTP credential>",
        ),
        "database" => Some(
            "config:\n  \
             query: <string, required — SQL>\n  \
             credential: <string, required — DB connection credential>\n  \
             params: <array of values, optional — for parameterized queries>",
        ),
        "s3" => Some(
            "config:\n  \
             bucket: <string, required>\n  \
             key: <string, required>\n  \
             operation: <get|put|list|delete, required>\n  \
             body: <string, required for put>\n  \
             credential: <string, required — AWS credential>",
        ),
        "subworkflow" => Some(
            "config:\n  \
             workflow: <string, required — name of another workflow>\n  \
             input: <any, optional — input to pass>",
        ),
        "integration" => Some(
            "config:\n  \
             service: <string, required — integration name e.g. github, slack, openai, stripe, notion>\n  \
             operation: <string, required — see r8r integrations show <service>>\n  \
             credential: <string, optional — credential alias>\n  \
             params: <map of params for the operation>",
        ),
        "crypto" => Some(
            "config:\n  \
             operation: <hash|encrypt|decrypt|sign|verify, required>\n  \
             algorithm: <string, required — e.g. sha256, hmac-sha256, aes-256-gcm>\n  \
             data: <string, required — input or template>\n  \
             key: <string, optional — for keyed ops>",
        ),
        "datetime" => Some(
            "config:\n  \
             operation: <now|parse|format|add|diff, required>\n  \
             format: <string, optional — strftime-like>\n  \
             input: <string, optional>\n  \
             timezone: <string, optional>",
        ),
        "debug" => Some(
            "config:\n  \
             message: <string, optional>\n  \
             log_input: <bool, optional>\n  \
             level: <info|debug|warn|trace, default info>",
        ),
        "summarize" => Some(
            "config:\n  \
             field: <string, REQUIRED — template pointing to text or array>\n  \
             max_length: <integer, optional>",
        ),
        "variables" => Some(
            "config:\n  \
             set: <map of name -> value, optional>\n  \
             get: <string, optional — variable name to read>",
        ),
        "sandbox" => None, // feature-gated, intentionally omitted here
        _ => None,
    }
}

/// Cheatsheet block appended to the system prompt to fight common Rhai hallucinations.
const RHAI_CHEATSHEET: &str = "\n\
RHAI EXPRESSION CHEATSHEET (for transform / switch / filter / condition):\n\
\n\
The expression language is Rhai, NOT JavaScript. Common pitfalls:\n\
\n\
INPUT:\n\
  - In `transform` nodes, `input` is a JSON STRING in scope. Parse it first:\n\
      let data = from_json(input);\n\
  - Previous node outputs are exposed as `<node_id_with_underscores>` (hyphens → underscores)\n\
    and are ALSO JSON strings:\n\
      let prev = from_json(my_fetch_node);\n\
  - In `if` / `filter` conditions, `field` uses template syntax like\n\
    \"input.foo\" or \"{{ nodes.fetch.output.body }}\" — NOT a Rhai expression.\n\
\n\
LOOPS:\n\
  - Use `for x in arr` (NOT `for (let i = 0; i < n; i++)`).\n\
  - Index manually with a counter:\n\
      let i = 0;\n\
      for s in stories { i += 1; ... }\n\
\n\
STRING INTERPOLATION:\n\
  - Backtick templates work: `Hello ${name}` is valid.\n\
  - But concatenation is also fine: \"Hello \" + name.\n\
\n\
ARRAY METHODS:\n\
  - LENGTH: use len(arr) — NOT arr.length.\n\
  - JOIN: there is NO built-in join. Fold manually:\n\
      let result = \"\";\n\
      for line in lines {\n\
        if result != \"\" { result += \"\\n\\n\"; }\n\
        result += line;\n\
      }\n\
      result\n\
  - PUSH: arr.push(x) works.\n\
  - MAP / FILTER: arr.map(|x| ...) and arr.filter(|x| ...) work.\n\
\n\
NULL / OPTIONAL:\n\
  - Missing object fields are `()` (unit), NOT null/undefined.\n\
  - Check with type_of(value): returns \"()\", \"string\", \"i64\", \"map\", \"array\", etc.\n\
\n\
JSON OUTPUT:\n\
  - to_json(value) serializes back to a JSON string.\n\
  - The LAST expression in the script becomes the node output (no `return`).\n\
\n\
WHAT DOES NOT WORK:\n\
  - `for (let i = 0; i < n; i++)`  ← C-style for loops (Rhai uses `for x in y`)\n\
  - `arr.length`                    ← use len(arr)\n\
  - `arr.join(sep)`                  ← fold manually\n\
  - `JSON.parse` / `JSON.stringify`  ← use from_json / to_json\n\
  - `if (cond)`                      ← parens not needed (and OK), use `if cond { ... }`\n\
  - `typeof x === \"string\"`         ← use type_of(x) == \"string\"\n\
\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> GeneratorContext {
        GeneratorContext {
            node_catalog: vec![
                ("http".to_string(), "Make HTTP requests".to_string()),
                (
                    "transform".to_string(),
                    "Transform data with Rhai".to_string(),
                ),
                ("agent".to_string(), "Call LLM providers".to_string()),
            ],
            credential_names: vec!["openai".to_string(), "slack-webhook".to_string()],
            examples: vec!["name: hello\nnodes:\n  - id: greet\n    type: transform".to_string()],
        }
    }

    #[test]
    fn test_system_prompt_includes_all_nodes() {
        let ctx = test_context();
        let prompt = build_system_prompt(&ctx);

        assert!(prompt.contains("http"), "expected 'http' in prompt");
        assert!(
            prompt.contains("transform"),
            "expected 'transform' in prompt"
        );
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

        assert!(prompt.contains(yaml), "YAML should appear in fix prompt");
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

    #[test]
    fn test_system_prompt_includes_config_schema_for_critical_nodes() {
        let ctx = GeneratorContext {
            node_catalog: vec![
                ("limit".into(), "Limit array".into()),
                ("transform".into(), "Transform data".into()),
                ("http".into(), "HTTP request".into()),
                ("filter".into(), "Filter array".into()),
            ],
            credential_names: vec![],
            examples: vec![],
        };
        let prompt = build_system_prompt(&ctx);
        // limit must surface the REQUIRED `field` config so the model stops omitting it.
        assert!(prompt.contains("field: <string, REQUIRED"));
        // transform schema must warn that input is a JSON string
        assert!(prompt.contains("Input is a JSON STRING"));
        // http must list the body field
        assert!(prompt.contains("body: <JSON object"));
        // filter must list operators
        assert!(prompt.contains("equals|not_equals|gt|lt|gte|lte|contains|regex"));
    }

    #[test]
    fn test_system_prompt_includes_rhai_cheatsheet() {
        let ctx = GeneratorContext {
            node_catalog: vec![("transform".into(), "Transform data".into())],
            credential_names: vec![],
            examples: vec![],
        };
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("RHAI EXPRESSION CHEATSHEET"));
        // The cheatsheet must call out the most common hallucinations
        assert!(prompt.contains("from_json(input)"));
        assert!(prompt.contains("use len(arr)"));
        assert!(prompt.contains("there is NO built-in join"));
        assert!(prompt.contains("for (let i"));
    }

    #[test]
    fn test_node_config_schema_returns_none_for_unknown() {
        assert!(node_config_schema("bogus").is_none());
    }

    #[test]
    fn test_node_config_schema_covers_priority_nodes() {
        for n in [
            "transform",
            "http",
            "agent",
            "limit",
            "filter",
            "sort",
            "set",
            "if",
            "switch",
            "integration",
        ] {
            assert!(
                node_config_schema(n).is_some(),
                "missing config_schema coverage for '{n}'"
            );
        }
    }
}
