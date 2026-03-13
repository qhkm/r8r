/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! JSON Schema for r8r workflow YAML files.
//!
//! The schema enables IDE autocomplete and validation when paired with the
//! `yaml-language-server` or any JSON Schema-aware editor.
//!
//! # VSCode setup
//!
//! Add to `.vscode/settings.json`:
//! ```json
//! {
//!   "yaml.schemas": {
//!     "https://r8r.dev/schemas/workflow.json": "workflows/**/*.yaml"
//!   }
//! }
//! ```
//!
//! Or reference inline in a workflow file:
//! ```yaml
//! # yaml-language-server: $schema=https://r8r.dev/schemas/workflow.json
//! name: my-workflow
//! ```

/// The r8r workflow JSON Schema embedded at compile time.
pub const WORKFLOW_SCHEMA: &str = include_str!("../assets/r8r-workflow.schema.json");

/// Return the schema as a pretty-printed string (already pretty in the asset).
pub fn workflow_schema_json() -> &'static str {
    WORKFLOW_SCHEMA
}
