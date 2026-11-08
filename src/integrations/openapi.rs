/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Generate an [`IntegrationDefinition`] from an OpenAPI 3.x document.
//!
//! The converter maps each `paths.<path>.<method>` entry to one operation:
//! path, query and header parameters become typed `params`, and a JSON or
//! form request body is flattened into one param per top-level property.
//! Local `$ref`s (`#/components/...`) are resolved; remote refs are not.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Map, Value, json};

use super::definition::{AuthConfig, AuthMethod, IntegrationDefinition, OperationDef, ParamDef};

const METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "head", "options"];
const MAX_REF_DEPTH: usize = 32;

/// Options that steer the conversion.
#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// Integration name; derived from `info.title` when absent.
    pub name: Option<String>,
    /// Base URL; taken from `servers[0]` when absent.
    pub base_url: Option<String>,
    /// Only keep operations carrying at least one of these tags.
    pub tags: Vec<String>,
    /// Where the spec was fetched from; relative server URLs resolve against it.
    pub source_url: Option<String>,
}

/// Result of a conversion: the definition plus anything that was skipped.
#[derive(Debug, Clone)]
pub struct ImportResult {
    pub definition: IntegrationDefinition,
    pub warnings: Vec<String>,
}

/// Parse an OpenAPI document from JSON or YAML text.
pub fn parse_spec(text: &str) -> Result<Value, String> {
    // YAML is a superset of JSON, so one parser covers both.
    serde_yaml::from_str::<Value>(text).map_err(|e| format!("Could not parse spec: {}", e))
}

/// Convert a parsed OpenAPI 3.x document into an integration definition.
pub fn convert(spec: &Value, opts: &ImportOptions) -> Result<ImportResult, String> {
    match spec.get("openapi").and_then(Value::as_str) {
        Some(v) if v.starts_with('3') => {}
        Some(v) => {
            return Err(format!(
                "Unsupported OpenAPI version '{}'; only 3.x is supported",
                v
            ));
        }
        None if spec.get("swagger").is_some() => {
            return Err("Swagger 2.0 is not supported; convert the spec to OpenAPI 3 first".into());
        }
        None => return Err("Not an OpenAPI document: missing 'openapi' field".into()),
    }

    let mut warnings = Vec::new();
    let info = spec.get("info").cloned().unwrap_or(Value::Null);
    let title = info.get("title").and_then(Value::as_str);

    let name = match &opts.name {
        Some(n) => n.clone(),
        None => title
            .map(slugify)
            .filter(|s| !s.is_empty())
            .ok_or("Spec has no info.title; pass --name")?,
    };

    let base_url = match &opts.base_url {
        Some(u) => u.trim_end_matches('/').to_string(),
        None => {
            let url = server_url(spec).ok_or("Spec has no servers[0].url; pass --base-url")?;
            match &opts.source_url {
                // OpenAPI: relative server URLs are relative to the document.
                Some(src) if !url.starts_with("http://") && !url.starts_with("https://") => {
                    reqwest::Url::parse(src)
                        .and_then(|base| base.join(&url))
                        .map(|u| u.to_string().trim_end_matches('/').to_string())
                        .map_err(|e| format!("Cannot resolve server URL '{}': {}", url, e))?
                }
                _ => url,
            }
        }
    };
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return Err(format!(
            "Base URL '{}' is not absolute; pass --base-url",
            base_url
        ));
    }

    let (auth, query_credential) = convert_security(spec, &mut warnings);

    let mut operations: HashMap<String, OperationDef> = HashMap::new();
    let paths = spec.get("paths").and_then(Value::as_object);
    for (path, item) in paths.into_iter().flatten() {
        let item = resolve(spec, item);
        let shared_params = item.get("parameters").and_then(Value::as_array).cloned();
        for method in METHODS {
            let Some(op) = item.get(method) else { continue };
            if !opts.tags.is_empty() && !has_any_tag(op, &opts.tags) {
                continue;
            }
            if op.get("deprecated").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let op_name = unique_name(&operations, &operation_name(op, method, path));
            match convert_operation(
                spec,
                path,
                method,
                op,
                shared_params.as_deref(),
                query_credential.as_deref(),
                &mut warnings,
            ) {
                Ok(def) => {
                    operations.insert(op_name, def);
                }
                Err(e) => warnings.push(format!(
                    "{} {}: skipped ({})",
                    method.to_uppercase(),
                    path,
                    e
                )),
            }
        }
    }

    if operations.is_empty() {
        return Err("No operations found in spec (check --tag filters)".into());
    }

    let docs_url = spec
        .pointer("/externalDocs/url")
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok(ImportResult {
        definition: IntegrationDefinition {
            version: 1,
            name,
            display_name: title.map(str::to_string),
            description: info
                .get("description")
                .and_then(Value::as_str)
                .map(first_line),
            base_url,
            docs_url,
            auth,
            headers: None,
            operations,
        },
        warnings,
    })
}

/// Render a definition as YAML with sorted keys and no null fields.
pub fn to_yaml(def: &IntegrationDefinition) -> Result<String, String> {
    let value = serde_json::to_value(def).map_err(|e| e.to_string())?;
    let mut value = sorted(prune(value).unwrap_or(Value::Null));
    // Keep the top-level keys in the order the built-in files use.
    if let Value::Object(map) = &mut value {
        let order = [
            "version",
            "name",
            "display_name",
            "description",
            "base_url",
            "docs_url",
            "auth",
            "headers",
            "operations",
        ];
        let mut ordered = serde_yaml::Mapping::new();
        for key in order {
            if let Some(v) = map.remove(key) {
                ordered.insert(
                    key.into(),
                    serde_yaml::to_value(v).map_err(|e| e.to_string())?,
                );
            }
        }
        return serde_yaml::to_string(&ordered).map_err(|e| e.to_string());
    }
    serde_yaml::to_string(&value).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

fn convert_operation(
    spec: &Value,
    path: &str,
    method: &str,
    op: &Value,
    shared_params: Option<&[Value]>,
    query_credential: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<OperationDef, String> {
    let label = format!("{} {}", method.to_uppercase(), path);
    let mut params: HashMap<String, ParamDef> = HashMap::new();
    let mut query: HashMap<String, String> = HashMap::new();
    let mut headers: HashMap<String, String> = HashMap::new();
    let mut path_tmpl = path.to_string();

    // Operation-level parameters override path-level ones with the same (in, name).
    let mut merged: Vec<Value> = Vec::new();
    let op_params: Vec<Value> = op
        .get("parameters")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|p| resolve(spec, p).clone()).collect())
        .unwrap_or_default();
    let key = |p: &Value| {
        (
            p.get("in")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            p.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        )
    };
    let overridden: HashSet<_> = op_params.iter().map(key).collect();
    for p in shared_params.unwrap_or_default() {
        let p = resolve(spec, p).clone();
        if !overridden.contains(&key(&p)) {
            merged.push(p);
        }
    }
    merged.extend(op_params);

    for p in &merged {
        let (location, raw) = key(p);
        if raw.is_empty() {
            continue;
        }
        let pname = param_ident(&raw);
        let schema = p
            .get("schema")
            .map(|s| resolve(spec, s))
            .unwrap_or(&Value::Null);
        let mut def = param_def(spec, schema, p.get("description"));
        def.required =
            location == "path" || p.get("required").and_then(Value::as_bool) == Some(true);
        let placeholder = format!("{{{{ params.{} }}}}", pname);
        match location.as_str() {
            "path" => path_tmpl = path_tmpl.replace(&format!("{{{}}}", raw), &placeholder),
            "query" => {
                query.insert(raw.clone(), placeholder);
            }
            "header" => {
                headers.insert(raw.clone(), placeholder);
            }
            other => {
                warnings.push(format!("{}: {} parameter '{}' ignored", label, other, raw));
                continue;
            }
        }
        params.insert(pname, def);
    }

    // Path variables the spec forgot to declare become required string params.
    for raw in undeclared_path_vars(&path_tmpl) {
        let pname = param_ident(&raw);
        path_tmpl = path_tmpl.replace(
            &format!("{{{}}}", raw),
            &format!("{{{{ params.{} }}}}", pname),
        );
        params.entry(pname).or_insert(ParamDef {
            required: true,
            param_type: Some("string".to_string()),
            description: None,
            default: None,
            enum_values: None,
        });
    }

    if let Some(cred) = query_credential {
        query.insert(cred.to_string(), "{{ credential }}".to_string());
    }

    let (body, body_type) = match op.get("requestBody") {
        Some(rb) => convert_body(spec, resolve(spec, rb), &mut params, &label, warnings)?,
        None => (None, None),
    };

    let description = op
        .get("summary")
        .or_else(|| op.get("description"))
        .and_then(Value::as_str)
        .map(first_line)
        .unwrap_or_else(|| label.clone());

    Ok(OperationDef {
        description,
        method: method.to_uppercase(),
        path: path_tmpl,
        params,
        headers: (!headers.is_empty()).then_some(headers),
        body,
        query: (!query.is_empty()).then_some(query),
        body_type,
    })
}

type BodyParts = (Option<Value>, Option<String>);

fn convert_body(
    spec: &Value,
    rb: &Value,
    params: &mut HashMap<String, ParamDef>,
    label: &str,
    warnings: &mut Vec<String>,
) -> Result<BodyParts, String> {
    let content = rb.get("content").and_then(Value::as_object);
    let pick = |want: &str| {
        content.and_then(|c| {
            c.iter()
                .find(|(k, _)| {
                    k.split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .eq_ignore_ascii_case(want)
                        || (want == "application/json" && k.ends_with("+json"))
                })
                .map(|(_, v)| v)
        })
    };
    let (media, body_type) = if let Some(m) = pick("application/json") {
        (m, None)
    } else if let Some(m) = pick("application/x-www-form-urlencoded") {
        (m, Some("form".to_string()))
    } else {
        let kinds: Vec<_> = content
            .map(|c| c.keys().cloned().collect())
            .unwrap_or_default();
        return Err(format!("unsupported request body type {:?}", kinds));
    };

    let body_required = rb.get("required").and_then(Value::as_bool) == Some(true);
    let schema = media
        .get("schema")
        .map(|s| resolve(spec, s))
        .unwrap_or(&Value::Null);
    let schema = merge_all_of(spec, schema);

    let props = schema.get("properties").and_then(Value::as_object);
    match props {
        Some(props) if !props.is_empty() => {
            let required: HashSet<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let mut body = Map::new();
            for (field, prop) in props {
                let prop = resolve(spec, prop);
                if prop.get("readOnly").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                let mut pname = param_ident(field);
                if params.contains_key(&pname) {
                    let renamed = format!("body_{}", pname);
                    warnings.push(format!(
                        "{}: body field '{}' clashes with a parameter; exposed as '{}'",
                        label, field, renamed
                    ));
                    pname = renamed;
                }
                let mut def = param_def(spec, prop, prop.get("description"));
                def.required = body_required && required.contains(field.as_str());
                body.insert(field.clone(), json!(format!("{{{{ params.{} }}}}", pname)));
                params.insert(pname, def);
            }
            Ok((Some(Value::Object(body)), body_type))
        }
        _ => {
            // Arrays, primitives and free-form objects pass through whole.
            let mut def = param_def(spec, &schema, rb.get("description"));
            def.required = body_required;
            if def.description.is_none() {
                def.description = Some("Request body".to_string());
            }
            params.insert("body".to_string(), def);
            Ok((Some(json!("{{ params.body }}")), body_type))
        }
    }
}

fn param_def(spec: &Value, schema: &Value, description: Option<&Value>) -> ParamDef {
    let schema = merge_all_of(spec, schema);
    let param_type = match schema.get("type") {
        Some(Value::String(t)) => Some(t.clone()),
        // OpenAPI 3.1 allows `type: [string, "null"]`.
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .map(str::to_string),
        _ if schema.get("properties").is_some() => Some("object".to_string()),
        _ if schema.get("items").is_some() => Some("array".to_string()),
        _ => None,
    }
    .map(|t| {
        if t == "number" || t == "integer" || t == "boolean" || t == "array" || t == "object" {
            t
        } else {
            "string".to_string()
        }
    });

    let enum_values = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|vals| {
            let strings: Vec<String> = vals
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            (!strings.is_empty() && strings.len() == vals.len()).then_some(strings)
        });

    ParamDef {
        required: false,
        param_type,
        description: description
            .or_else(|| schema.get("description"))
            .and_then(Value::as_str)
            .map(first_line),
        default: schema.get("default").cloned(),
        enum_values,
    }
}

// ---------------------------------------------------------------------------
// Security
// ---------------------------------------------------------------------------

/// Returns the auth config plus, for `apiKey in: query` schemes, the query
/// parameter name that should carry `{{ credential }}`.
fn convert_security(
    spec: &Value,
    warnings: &mut Vec<String>,
) -> (Option<AuthConfig>, Option<String>) {
    let Some(schemes) = spec
        .pointer("/components/securitySchemes")
        .and_then(Value::as_object)
    else {
        return (None, None);
    };

    // Prefer schemes the spec actually applies globally, then the rest.
    let mut names: Vec<&String> = schemes.keys().collect();
    let global: Vec<String> = spec
        .get("security")
        .and_then(Value::as_array)
        .map(|reqs| {
            reqs.iter()
                .filter_map(Value::as_object)
                .flat_map(|o| o.keys().cloned())
                .collect()
        })
        .unwrap_or_default();
    names.sort_by_key(|n| (!global.contains(n), n.to_string()));

    let mut methods = Vec::new();
    let mut query_credential = None;
    for name in names {
        let scheme = resolve(spec, &schemes[name]);
        let kind = scheme.get("type").and_then(Value::as_str).unwrap_or("");
        let description = scheme
            .get("description")
            .and_then(Value::as_str)
            .map(first_line);
        match kind {
            "http" => match scheme
                .get("scheme")
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("bearer") => methods.push(AuthMethod::Bearer { description }),
                Some("basic") => methods.push(AuthMethod::Basic { description }),
                other => warnings.push(format!(
                    "security scheme '{}': http {:?} not supported",
                    name, other
                )),
            },
            "apiKey" => {
                let key_name = scheme
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                match scheme.get("in").and_then(Value::as_str) {
                    Some("header") => methods.push(AuthMethod::ApiKey {
                        header_name: key_name,
                        description,
                    }),
                    Some("query") if query_credential.is_none() => query_credential = key_name,
                    other => warnings.push(format!(
                        "security scheme '{}': apiKey in {:?} not supported",
                        name, other
                    )),
                }
            }
            "oauth2" => {
                let flows = scheme.get("flows").and_then(Value::as_object);
                if let Some(code) = flows.and_then(|f| f.get("authorizationCode")) {
                    let scopes = code
                        .get("scopes")
                        .and_then(Value::as_object)
                        .map(|s| s.keys().cloned().collect())
                        .unwrap_or_default();
                    methods.push(AuthMethod::OAuth2 {
                        authorization_url: str_field(code, "authorizationUrl"),
                        token_url: str_field(code, "tokenUrl"),
                        scopes,
                        flows: vec!["authorization_code".to_string()],
                    });
                } else {
                    warnings.push(format!(
                        "security scheme '{}': only the oauth2 authorizationCode flow is imported",
                        name
                    ));
                }
            }
            other => warnings.push(format!(
                "security scheme '{}': type '{}' not supported",
                name, other
            )),
        }
    }

    let auth = (!methods.is_empty()).then_some(AuthConfig { methods });
    (auth, query_credential)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Follow local `$ref`s until a concrete node is reached.
fn resolve<'a>(spec: &'a Value, mut node: &'a Value) -> &'a Value {
    for _ in 0..MAX_REF_DEPTH {
        let Some(r) = node.get("$ref").and_then(Value::as_str) else {
            return node;
        };
        let Some(pointer) = r.strip_prefix('#') else {
            return node;
        };
        let pointer = pointer.replace("~1", "/").replace("~0", "~");
        match spec.pointer(&pointer) {
            Some(target) => node = target,
            None => return node,
        }
    }
    node
}

/// Flatten `allOf` composition (properties + required) into one schema.
fn merge_all_of(spec: &Value, schema: &Value) -> Value {
    let schema = resolve(spec, schema);
    let Some(parts) = schema.get("allOf").and_then(Value::as_array) else {
        return schema.clone();
    };
    let mut out = schema.as_object().cloned().unwrap_or_default();
    out.remove("allOf");
    let mut props = out
        .remove("properties")
        .and_then(|p| p.as_object().cloned())
        .unwrap_or_default();
    let mut required: Vec<Value> = out
        .remove("required")
        .and_then(|r| r.as_array().cloned())
        .unwrap_or_default();
    for part in parts {
        let part = merge_all_of(spec, part);
        if let Some(p) = part.get("properties").and_then(Value::as_object) {
            props.extend(p.clone());
        }
        if let Some(r) = part.get("required").and_then(Value::as_array) {
            required.extend(r.iter().cloned());
        }
        for k in ["type", "description"] {
            if !out.contains_key(k)
                && let Some(v) = part.get(k)
            {
                out.insert(k.to_string(), v.clone());
            }
        }
    }
    if !props.is_empty() {
        out.insert("properties".into(), Value::Object(props));
    }
    if !required.is_empty() {
        out.insert("required".into(), Value::Array(required));
    }
    Value::Object(out)
}

fn server_url(spec: &Value) -> Option<String> {
    let server = spec.get("servers")?.as_array()?.first()?;
    let mut url = server.get("url")?.as_str()?.to_string();
    if let Some(vars) = server.get("variables").and_then(Value::as_object) {
        for (k, v) in vars {
            if let Some(d) = v.get("default").and_then(Value::as_str) {
                url = url.replace(&format!("{{{}}}", k), d);
            }
        }
    }
    Some(url.trim_end_matches('/').to_string())
}

/// `{name}` segments left in a path after `{{ params.* }}` substitution.
fn undeclared_path_vars(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        if rest[start..].starts_with("{{") {
            match rest[start..].find("}}") {
                Some(end) => rest = &rest[start + end + 2..],
                None => break,
            }
            continue;
        }
        match rest[start + 1..].find('}') {
            Some(end) => {
                out.push(rest[start + 1..start + 1 + end].to_string());
                rest = &rest[start + 1 + end + 1..];
            }
            None => break,
        }
    }
    out
}

fn has_any_tag(op: &Value, wanted: &[String]) -> bool {
    op.get("tags")
        .and_then(Value::as_array)
        .is_some_and(|tags| {
            tags.iter()
                .filter_map(Value::as_str)
                .any(|t| wanted.iter().any(|w| w.eq_ignore_ascii_case(t)))
        })
}

fn operation_name(op: &Value, method: &str, path: &str) -> String {
    if let Some(id) = op.get("operationId").and_then(Value::as_str) {
        let n = snake_case(id);
        if !n.is_empty() {
            return n;
        }
    }
    let mut parts = vec![method.to_string()];
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        if let Some(inner) = seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            parts.push(format!("by_{}", snake_case(inner)));
        } else {
            parts.push(snake_case(seg));
        }
    }
    parts.join("_")
}

fn unique_name(existing: &HashMap<String, OperationDef>, base: &str) -> String {
    if !existing.contains_key(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{}_{}", base, i))
        .find(|n| !existing.contains_key(n))
        .unwrap()
}

/// `listRepos` / `list-repos` / `List Repos` → `list_repos`.
fn snake_case(s: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && prev_lower {
                out.push('_');
            }
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
            out.push(c.to_ascii_lowercase());
        } else {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            prev_lower = false;
        }
    }
    out.trim_matches('_').to_string()
}

/// Parameter names must survive `{{ params.X }}` templating: keep them to
/// `[a-z0-9_]`.
fn param_ident(raw: &str) -> String {
    let s = snake_case(raw);
    if s.is_empty() { "param".to_string() } else { s }
}

fn slugify(s: &str) -> String {
    snake_case(s).replace('_', "-")
}

fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Drop nulls and empty maps so the YAML stays tidy.
fn prune(v: Value) -> Option<Value> {
    match v {
        Value::Null => None,
        Value::Object(map) => {
            let out: Map<String, Value> = map
                .into_iter()
                .filter_map(|(k, v)| prune(v).map(|v| (k, v)))
                .collect();
            Some(Value::Object(out))
        }
        Value::Array(a) => Some(Value::Array(a.into_iter().filter_map(prune).collect())),
        other => Some(other),
    }
}

fn sorted(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let ordered: BTreeMap<String, Value> =
                map.into_iter().map(|(k, v)| (k, sorted(v))).collect();
            Value::Object(ordered.into_iter().collect())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(sorted).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::node::build_http_config;
    use crate::integrations::validator::validate_and_resolve_params;

    const PETSTORE: &str = r##"
openapi: 3.0.3
info:
  title: Pet Store
  description: |
    A sample pet store.
    Second line.
servers:
  - url: https://{region}.petstore.example.com/v1/
    variables:
      region: { default: eu }
security:
  - bearerAuth: []
components:
  securitySchemes:
    bearerAuth: { type: http, scheme: bearer }
  parameters:
    Limit:
      name: limit
      in: query
      schema: { type: integer, default: 20 }
  schemas:
    NewPet:
      type: object
      required: [name]
      properties:
        name: { type: string, description: Pet name }
        status: { type: string, enum: [available, sold] }
        id: { type: string, readOnly: true }
paths:
  /pets:
    get:
      operationId: listPets
      summary: List pets
      tags: [pets]
      parameters:
        - $ref: '#/components/parameters/Limit'
    post:
      operationId: createPet
      tags: [pets]
      requestBody:
        required: true
        content:
          application/json:
            schema: { $ref: '#/components/schemas/NewPet' }
  /pets/{petId}:
    parameters:
      - { name: petId, in: path, required: true, schema: { type: string } }
    get:
      summary: Get a pet
      tags: [pets]
      parameters:
        - { name: X-Request-Id, in: header, schema: { type: string } }
    delete:
      deprecated: true
  /stores/bulk:
    post:
      operationId: bulkStores
      tags: [stores]
      requestBody:
        content:
          application/json:
            schema: { type: array, items: { type: object } }
"##;

    fn petstore() -> ImportResult {
        convert(&parse_spec(PETSTORE).unwrap(), &ImportOptions::default()).unwrap()
    }

    #[test]
    fn maps_top_level_fields() {
        let def = petstore().definition;
        assert_eq!(def.name, "pet-store");
        assert_eq!(def.display_name.as_deref(), Some("Pet Store"));
        assert_eq!(def.description.as_deref(), Some("A sample pet store."));
        assert_eq!(def.base_url, "https://eu.petstore.example.com/v1");
        assert!(matches!(
            def.auth.unwrap().methods[0],
            AuthMethod::Bearer { .. }
        ));
    }

    #[test]
    fn maps_operations_and_skips_deprecated() {
        let def = petstore().definition;
        let mut names: Vec<_> = def.operations.keys().cloned().collect();
        names.sort();
        assert_eq!(
            names,
            [
                "bulk_stores",
                "create_pet",
                "get_pets_by_pet_id",
                "list_pets"
            ]
        );
    }

    #[test]
    fn query_ref_param_with_default() {
        let def = petstore().definition;
        let op = &def.operations["list_pets"];
        assert_eq!(op.query.as_ref().unwrap()["limit"], "{{ params.limit }}");
        assert_eq!(op.params["limit"].default, Some(json!(20)));
        assert_eq!(op.params["limit"].param_type.as_deref(), Some("integer"));
    }

    #[test]
    fn path_level_and_header_params() {
        let def = petstore().definition;
        let op = &def.operations["get_pets_by_pet_id"];
        assert_eq!(op.path, "/pets/{{ params.pet_id }}");
        assert!(op.params["pet_id"].required);
        assert_eq!(
            op.headers.as_ref().unwrap()["X-Request-Id"],
            "{{ params.x_request_id }}"
        );
    }

    #[test]
    fn json_body_flattened_without_read_only() {
        let def = petstore().definition;
        let op = &def.operations["create_pet"];
        assert_eq!(
            op.body,
            Some(json!({"name": "{{ params.name }}", "status": "{{ params.status }}"}))
        );
        assert!(op.params["name"].required);
        assert!(!op.params["status"].required);
        assert_eq!(
            op.params["status"].enum_values.as_deref(),
            Some(&["available".to_string(), "sold".to_string()][..])
        );
        assert!(!op.params.contains_key("id"));
    }

    #[test]
    fn array_body_passes_through() {
        let def = petstore().definition;
        let op = &def.operations["bulk_stores"];
        assert_eq!(op.body, Some(json!("{{ params.body }}")));
        assert_eq!(op.params["body"].param_type.as_deref(), Some("array"));
    }

    #[test]
    fn tag_filter() {
        let opts = ImportOptions {
            tags: vec!["stores".into()],
            ..Default::default()
        };
        let def = convert(&parse_spec(PETSTORE).unwrap(), &opts)
            .unwrap()
            .definition;
        assert_eq!(def.operations.len(), 1);
    }

    #[test]
    fn yaml_round_trips_and_builds_request() {
        let yaml = to_yaml(&petstore().definition).unwrap();
        assert!(
            yaml.starts_with("version: 1\nname: pet-store\n"),
            "{}",
            yaml
        );
        assert!(!yaml.contains("null"));
        let def: IntegrationDefinition = serde_yaml::from_str(&yaml).unwrap();

        let op = &def.operations["create_pet"];
        let resolved = validate_and_resolve_params(&op.params, &json!({"name": "Rex"})).unwrap();
        let config = build_http_config(&def, op, &resolved, Some("tok"));
        assert_eq!(config["url"], "https://eu.petstore.example.com/v1/pets");
        assert_eq!(config["body"], json!({"name": "Rex"}));
        assert_eq!(config["auth_type"], "bearer");
    }

    #[test]
    fn api_key_in_query_uses_credential() {
        let spec = parse_spec(
            r#"
openapi: 3.1.0
info: { title: Weather }
servers: [{ url: "https://api.weather.test" }]
components:
  securitySchemes:
    key: { type: apiKey, in: query, name: appid }
paths:
  /now:
    get: { operationId: now }
"#,
        )
        .unwrap();
        let def = convert(&spec, &ImportOptions::default())
            .unwrap()
            .definition;
        assert!(def.auth.is_none());
        assert_eq!(
            def.operations["now"].query.as_ref().unwrap()["appid"],
            "{{ credential }}"
        );
    }

    #[test]
    fn body_field_clashing_with_path_param_is_renamed() {
        let spec = parse_spec(
            r#"
openapi: 3.0.0
info: { title: T }
servers: [{ url: "https://t.test" }]
paths:
  /items/{id}:
    put:
      operationId: update
      parameters: [{ name: id, in: path, required: true, schema: { type: string } }]
      requestBody:
        content:
          application/json:
            schema: { type: object, properties: { id: { type: string } } }
"#,
        )
        .unwrap();
        let res = convert(&spec, &ImportOptions::default()).unwrap();
        let op = &res.definition.operations["update"];
        assert_eq!(op.body, Some(json!({"id": "{{ params.body_id }}"})));
        assert_eq!(res.warnings.len(), 1);
    }

    #[test]
    fn rejects_swagger_2_and_relative_servers() {
        let swagger = parse_spec("swagger: '2.0'\ninfo: { title: x }\npaths: {}").unwrap();
        assert!(
            convert(&swagger, &ImportOptions::default())
                .unwrap_err()
                .contains("Swagger 2.0")
        );
        let rel =
            parse_spec("openapi: 3.0.0\ninfo: { title: x }\nservers: [{ url: /api }]\npaths: {}")
                .unwrap();
        assert!(
            convert(&rel, &ImportOptions::default())
                .unwrap_err()
                .contains("--base-url")
        );
    }

    #[test]
    fn relative_server_resolves_against_source_url() {
        let spec = parse_spec(
            "openapi: 3.0.0\ninfo: { title: x }\nservers: [{ url: /api/v3 }]\npaths:\n  /p:\n    get: { operationId: p }\n",
        )
        .unwrap();
        let opts = ImportOptions {
            source_url: Some("https://example.com/specs/openapi.json".into()),
            ..Default::default()
        };
        let def = convert(&spec, &opts).unwrap().definition;
        assert_eq!(def.base_url, "https://example.com/api/v3");
    }

    #[test]
    fn undeclared_path_var_becomes_required_param() {
        let spec = parse_spec(
            "openapi: 3.0.0\ninfo: { title: T }\nservers: [{ url: 'https://t.test' }]\npaths:\n  /a/{orgId}/b:\n    get: { operationId: g }\n",
        )
        .unwrap();
        let def = convert(&spec, &ImportOptions::default())
            .unwrap()
            .definition;
        let op = &def.operations["g"];
        assert_eq!(op.path, "/a/{{ params.org_id }}/b");
        assert!(op.params["org_id"].required);
    }

    #[test]
    fn snake_case_variants() {
        assert_eq!(snake_case("listRepos"), "list_repos");
        assert_eq!(snake_case("list-repos"), "list_repos");
        assert_eq!(snake_case("filter[status]"), "filter_status");
        assert_eq!(snake_case("getHTTPStatus"), "get_httpstatus");
    }
}
