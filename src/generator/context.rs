//! Generator context — collects node catalog, credential names, and example workflows.

use crate::credentials::CredentialStore;
use crate::nodes::NodeRegistry;

/// Context collected at runtime to guide LLM prompt generation.
#[derive(Debug, Clone)]
pub struct GeneratorContext {
    /// Node type names paired with their descriptions, sorted alphabetically.
    pub node_catalog: Vec<(String, String)>,
    /// Credential service names only — values are never included.
    pub credential_names: Vec<String>,
    /// Example workflow YAML strings loaded from the `examples/` directory.
    pub examples: Vec<String>,
}

impl GeneratorContext {
    /// Build a `GeneratorContext` by querying the node registry, credential store,
    /// and loading curated example workflows from disk.
    pub async fn build() -> Self {
        // 1. Build node catalog from the registry.
        let registry = NodeRegistry::new();
        let mut descs: Vec<(String, String)> = registry
            .descriptions()
            .into_iter()
            .map(|(name, desc)| (name.to_string(), desc.to_string()))
            .collect();
        descs.sort_by(|a, b| a.0.cmp(&b.0));

        // 2. Load credential names (no values).
        let credential_names = match CredentialStore::load().await {
            Ok(store) => store
                .list()
                .into_iter()
                .map(|c| c.service.clone())
                .collect(),
            Err(_) => Vec::new(),
        };

        // 3. Load example workflows.
        let examples = Self::load_examples();

        Self {
            node_catalog: descs,
            credential_names,
            examples,
        }
    }

    /// Load a curated subset of example workflows from the `examples/` directory.
    ///
    /// Files that do not exist on disk are silently skipped.
    pub fn load_examples() -> Vec<String> {
        let curated = [
            "hello-world.yaml",
            "order-notification.yaml",
            "smart-classifier.yaml",
        ];

        // Locate the `examples/` directory relative to the crate root (CARGO_MANIFEST_DIR).
        let examples_dir = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()),
        )
        .join("examples");

        curated
            .iter()
            .filter_map(|filename| {
                let path = examples_dir.join(filename);
                std::fs::read_to_string(&path).ok()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_catalog_not_empty() {
        let registry = NodeRegistry::new();
        let descs = registry.descriptions();
        assert!(!descs.is_empty());
        let names: Vec<&str> = descs.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"http"));
        assert!(names.contains(&"agent"));
        assert!(names.contains(&"transform"));
    }

    #[test]
    fn test_load_examples() {
        let examples = GeneratorContext::load_examples();
        for ex in &examples {
            assert!(ex.contains("name:"));
        }
    }
}
