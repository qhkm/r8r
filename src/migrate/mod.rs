/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Workflow migration from other platforms.

pub mod n8n;

use crate::error::Result;
use crate::workflow::Workflow;

/// Result of a migration operation.
#[derive(Debug)]
pub struct MigrateResult {
    pub workflow: Workflow,
    pub warnings: Vec<MigrateWarning>,
}

/// A warning emitted during migration.
#[derive(Debug, Clone)]
pub struct MigrateWarning {
    pub node_name: Option<String>,
    pub category: WarningCategory,
    pub message: String,
}

/// Categories of migration warnings.
#[derive(Debug, Clone, PartialEq)]
pub enum WarningCategory {
    UnsupportedNode,
    ApproximateExpression,
    UnconvertedExpression,
    CredentialReference,
    FeatureGate,
}

impl std::fmt::Display for MigrateWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.node_name {
            Some(name) => write!(f, "Node \"{}\": {}", name, self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

/// Trait for migration sources (extensible for future platforms).
pub trait MigrateSource {
    fn name(&self) -> &str;
    fn convert(&self, input: &[u8]) -> Result<MigrateResult>;
}
