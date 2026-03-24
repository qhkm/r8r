/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! JS-to-Rhai expression transpiler for n8n workflow migration.
//!
//! Converts n8n's JavaScript template expressions (e.g. `{{ $json.name }}`)
//! into equivalent Rhai expressions across three tiers:
//!
//! - **Tier 1**: Direct variable/accessor replacements ($json, $node, $env, etc.)
//! - **Tier 2**: Common method/pattern rewrites (.length, .includes, ternary, etc.)
//! - **Tier 3**: Fallback detection for unsupported JS constructs

use regex_lite::Regex;

/// Result of transpiling a single JS expression into Rhai.
#[derive(Debug, Clone)]
pub enum TranspileResult {
    /// Expression was converted with full fidelity.
    Exact(String),
    /// Expression was converted but semantics may differ slightly.
    Approximate(String, String),
    /// Expression could not be converted.
    Failed(String, String),
}

impl TranspileResult {
    /// Returns the Rhai expression (or original on failure).
    pub fn rhai(&self) -> &str {
        match self {
            TranspileResult::Exact(s) => s,
            TranspileResult::Approximate(s, _) => s,
            TranspileResult::Failed(s, _) => s,
        }
    }

    /// Returns `true` if the conversion was exact.
    pub fn is_exact(&self) -> bool {
        matches!(self, TranspileResult::Exact(_))
    }

    /// Returns `true` if the conversion failed.
    pub fn is_failed(&self) -> bool {
        matches!(self, TranspileResult::Failed(_, _))
    }

    /// Returns the warning message, if any.
    pub fn warning(&self) -> Option<&str> {
        match self {
            TranspileResult::Approximate(_, w) => Some(w),
            TranspileResult::Failed(_, r) => Some(r),
            _ => None,
        }
    }
}

/// Sanitize an n8n node name into a valid Rhai identifier.
///
/// Lowercases, replaces spaces and hyphens with underscores, strips
/// everything that isn't alphanumeric or underscore.
fn sanitize_node_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c == ' ' || c == '-' { '_' } else { c })
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// Transpile a single n8n JS expression into Rhai.
///
/// If the expression is wrapped in `{{ }}` delimiters they are stripped first.
pub fn transpile_expression(js: &str) -> TranspileResult {
    // Strip {{ }} delimiters if present.
    let expr = strip_delimiters(js);
    let mut out = expr.to_string();
    let mut approximate = false;
    let mut warnings: Vec<String> = Vec::new();

    // ── Tier 1: Direct replacements ────────────────────────────────

    // $input.item.json.X → input.X
    let re_input_item = Regex::new(r"\$input\.item\.json\.(\w[\w.]*)").unwrap();
    out = re_input_item.replace_all(&out, "input.$1").to_string();

    // $json["field"] → input.field
    let re_json_bracket = Regex::new(r#"\$json\["(\w+)"\]"#).unwrap();
    out = re_json_bracket.replace_all(&out, "input.$1").to_string();

    // $json.X → input.X
    let re_json_dot = Regex::new(r"\$json\.(\w[\w.]*)").unwrap();
    out = re_json_dot.replace_all(&out, "input.$1").to_string();

    // Bare $json (without dot or bracket) → input
    let re_json_bare = Regex::new(r"\$json\b").unwrap();
    out = re_json_bare.replace_all(&out, "input").to_string();

    // $node["Name"].json.field → nodes.sanitized_name.output.field
    let re_node_field = Regex::new(r#"\$node\["([^"]+)"\]\.json\.(\w[\w.]*)"#).unwrap();
    out = re_node_field
        .replace_all(&out, |caps: &regex_lite::Captures| {
            let name = sanitize_node_name(&caps[1]);
            let field = &caps[2];
            format!("nodes.{}.output.{}", name, field)
        })
        .to_string();

    // $node["Name"].json → nodes.sanitized_name.output
    let re_node_json = Regex::new(r#"\$node\["([^"]+)"\]\.json"#).unwrap();
    out = re_node_json
        .replace_all(&out, |caps: &regex_lite::Captures| {
            let name = sanitize_node_name(&caps[1]);
            format!("nodes.{}.output", name)
        })
        .to_string();

    // $env.X → env.X
    let re_env = Regex::new(r"\$env\.(\w[\w.]*)").unwrap();
    out = re_env.replace_all(&out, "env.$1").to_string();

    // $now → now()
    out = out.replace("$now", "now()");

    // $execution.id → execution_id
    out = out.replace("$execution.id", "execution_id");

    // === → ==, !== → !=
    out = out.replace("!==", "!=");
    out = out.replace("===", "==");

    // ── Tier 2: Common patterns ────────────────────────────────────

    // .length → len()
    let re_length = Regex::new(r"(\w[\w.]*)\.length\b").unwrap();
    out = re_length
        .replace_all(&out, |caps: &regex_lite::Captures| {
            format!("len({})", &caps[1])
        })
        .to_string();

    // .includes( → .contains(
    out = out.replace(".includes(", ".contains(");

    // .toLowerCase() → .to_lower()
    out = out.replace(".toLowerCase()", ".to_lower()");

    // .toUpperCase() → .to_upper()
    out = out.replace(".toUpperCase()", ".to_upper()");

    // parseInt( → parse_int(
    out = out.replace("parseInt(", "parse_int(");

    // JSON.parse( → from_json(
    out = out.replace("JSON.parse(", "from_json(");

    // JSON.stringify( → to_json(
    out = out.replace("JSON.stringify(", "to_json(");

    // Math.round/floor/ceil( → round/floor/ceil(
    out = out.replace("Math.round(", "round(");
    out = out.replace("Math.floor(", "floor(");
    out = out.replace("Math.ceil(", "ceil(");

    // Object.keys(x) → x.keys()
    let re_keys = Regex::new(r"Object\.keys\((\w[\w.]*)\)").unwrap();
    out = re_keys
        .replace_all(&out, |caps: &regex_lite::Captures| {
            format!("{}.keys()", &caps[1])
        })
        .to_string();

    // Ternary: a ? b : c → if a { b } else { c }  (simple, non-nested)
    let re_ternary = Regex::new(r"(.+?)\s*\?\s*(.+?)\s*:\s*(.+)").unwrap();
    if re_ternary.is_match(&out) && !out.contains("if ") {
        out = re_ternary
            .replace(&out, |caps: &regex_lite::Captures| {
                let cond = caps[1].trim();
                let then = caps[2].trim();
                let els = caps[3].trim();
                format!("if {} {{ {} }} else {{ {} }}", cond, then, els)
            })
            .to_string();
        approximate = true;
        warnings.push("ternary converted to if/else block".into());
    }

    // ── Tier 3: Fallback — detect unconvertible JS ─────────────────

    let unsupported: &[(&str, &str)] = &[
        (r"\.reduce\(", "Array.reduce() not supported in Rhai"),
        (r"\.forEach\(", "Array.forEach() not supported in Rhai"),
        (r"\.indexOf\(", "Array.indexOf() not supported in Rhai"),
        (r"\bnew Date\(", "new Date() not supported in Rhai"),
        (r"\btypeof\s", "typeof operator not supported in Rhai"),
        (r"\binstanceof\s", "instanceof operator not supported in Rhai"),
    ];

    for (pat, reason) in unsupported {
        let re = Regex::new(pat).unwrap();
        if re.is_match(&out) {
            return TranspileResult::Failed(expr.to_string(), reason.to_string());
        }
    }

    // Complex arrow functions with .map( or .filter( containing =>
    let re_arrow = Regex::new(r"\.(map|filter)\([^)]*=>").unwrap();
    if re_arrow.is_match(&out) {
        return TranspileResult::Failed(
            expr.to_string(),
            "arrow function with .map()/.filter() not supported in Rhai".to_string(),
        );
    }

    // ── Return ─────────────────────────────────────────────────────

    if out == expr {
        // Nothing changed — pass through as exact (simple literal / already Rhai).
        TranspileResult::Exact(out)
    } else if approximate {
        TranspileResult::Approximate(out, warnings.join("; "))
    } else {
        TranspileResult::Exact(out)
    }
}

/// Find all `{{ expr }}` occurrences in `input`, transpile each, and return
/// the modified string plus a vec of `(TranspileResult, original_match)` entries.
pub fn transpile_all_expressions(input: &str) -> (String, Vec<(TranspileResult, String)>) {
    let re = Regex::new(r"\{\{\s*(.+?)\s*\}\}").unwrap();
    let mut results: Vec<(TranspileResult, String)> = Vec::new();
    let output = re
        .replace_all(input, |caps: &regex_lite::Captures| {
            let full_match = caps[0].to_string();
            let inner = caps[1].to_string();
            let result = transpile_expression(&inner);
            let rhai_expr = result.rhai().to_string();
            results.push((result, full_match));
            format!("{{{{ {} }}}}", rhai_expr)
        })
        .to_string();
    (output, results)
}

/// Strip `{{ }}` delimiters from an expression, if present.
fn strip_delimiters(s: &str) -> &str {
    let trimmed = s.trim();
    if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
        trimmed[2..trimmed.len() - 2].trim()
    } else {
        trimmed
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_field_access() {
        let r = transpile_expression("$json.name");
        assert!(r.is_exact());
        assert_eq!(r.rhai(), "input.name");

        let r2 = transpile_expression("$json.nested.path");
        assert!(r2.is_exact());
        assert_eq!(r2.rhai(), "input.nested.path");
    }

    #[test]
    fn test_json_bracket_access() {
        let r = transpile_expression("$json[\"field\"]");
        assert!(r.is_exact());
        assert_eq!(r.rhai(), "input.field");
    }

    #[test]
    fn test_node_reference() {
        let r = transpile_expression("$node[\"HTTP Request\"].json.data");
        assert!(r.is_exact());
        assert_eq!(r.rhai(), "nodes.http_request.output.data");
    }

    #[test]
    fn test_builtins() {
        assert_eq!(transpile_expression("$now").rhai(), "now()");
        assert_eq!(transpile_expression("$execution.id").rhai(), "execution_id");
        assert_eq!(transpile_expression("$env.API_KEY").rhai(), "env.API_KEY");
    }

    #[test]
    fn test_strict_equality() {
        let r = transpile_expression("$json.status === 'active'");
        assert_eq!(r.rhai(), "input.status == 'active'");

        let r2 = transpile_expression("$json.x !== 0");
        assert_eq!(r2.rhai(), "input.x != 0");
    }

    #[test]
    fn test_input_item() {
        let r = transpile_expression("$input.item.json.field");
        assert!(r.is_exact());
        assert_eq!(r.rhai(), "input.field");
    }

    #[test]
    fn test_ternary() {
        let r = transpile_expression("$json.active ? \"yes\" : \"no\"");
        assert!(!r.is_exact());
        assert!(!r.is_failed());
        assert!(r.warning().is_some());
        assert_eq!(
            r.rhai(),
            "if input.active { \"yes\" } else { \"no\" }"
        );
    }

    #[test]
    fn test_array_length() {
        let r = transpile_expression("$json.items.length");
        assert!(r.is_exact());
        assert_eq!(r.rhai(), "len(input.items)");
    }

    #[test]
    fn test_string_methods() {
        let r = transpile_expression("$json.tags.includes(\"vip\")");
        assert!(r.is_exact());
        assert_eq!(r.rhai(), "input.tags.contains(\"vip\")");

        let r2 = transpile_expression("$json.name.toLowerCase()");
        assert_eq!(r2.rhai(), "input.name.to_lower()");

        let r3 = transpile_expression("$json.code.toUpperCase()");
        assert_eq!(r3.rhai(), "input.code.to_upper()");
    }

    #[test]
    fn test_json_parse_stringify() {
        let r = transpile_expression("JSON.parse($json.raw)");
        assert!(r.is_exact());
        assert_eq!(r.rhai(), "from_json(input.raw)");

        let r2 = transpile_expression("JSON.stringify($json.data)");
        assert!(r2.is_exact());
        assert_eq!(r2.rhai(), "to_json(input.data)");
    }

    #[test]
    fn test_parseint() {
        let r = transpile_expression("parseInt($json.count)");
        assert!(r.is_exact());
        assert_eq!(r.rhai(), "parse_int(input.count)");
    }

    #[test]
    fn test_unrecognized_fallback() {
        let r = transpile_expression("$json.items.reduce((a, b) => a + b, 0)");
        assert!(r.is_failed());
        assert!(r.warning().unwrap().contains("reduce"));
    }
}
