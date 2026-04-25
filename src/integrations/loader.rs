/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{Instant, SystemTime};

use tracing::{debug, warn};

use super::definition::IntegrationDefinition;
use crate::config::Config;

/// Cache for user-supplied integration definitions loaded from the filesystem.
///
/// Rescans the user integrations directory at most once per second to avoid
/// unnecessary I/O on hot paths.
struct UserDefinitionCache {
    definitions: HashMap<String, IntegrationDefinition>,
    last_scan: Instant,
    mtimes: HashMap<PathBuf, SystemTime>,
}

impl UserDefinitionCache {
    fn new() -> Self {
        Self {
            definitions: HashMap::new(),
            last_scan: Instant::now() - std::time::Duration::from_secs(10),
            mtimes: HashMap::new(),
        }
    }
}

/// Loads integration definitions from built-in YAML files and user overrides.
///
/// Built-in definitions are compiled into the binary via `include_str!` and
/// parsed once at construction. User definitions live in the config directory
/// under `integrations/` and are scanned on demand with mtime-based caching.
pub struct IntegrationLoader {
    builtin: HashMap<String, IntegrationDefinition>,
    user_cache: RwLock<UserDefinitionCache>,
}

impl IntegrationLoader {
    /// Create a new loader, parsing all built-in YAML definitions.
    pub fn new() -> Self {
        let mut builtin = HashMap::new();

        const BUILTIN_GITHUB: &str = include_str!("builtin/github.yaml");
        const BUILTIN_SLACK: &str = include_str!("builtin/slack.yaml");
        const BUILTIN_OPENAI: &str = include_str!("builtin/openai.yaml");
        const BUILTIN_STRIPE: &str = include_str!("builtin/stripe.yaml");
        const BUILTIN_NOTION: &str = include_str!("builtin/notion.yaml");

        for (name, yaml) in [
            ("github", BUILTIN_GITHUB),
            ("slack", BUILTIN_SLACK),
            ("openai", BUILTIN_OPENAI),
            ("stripe", BUILTIN_STRIPE),
            ("notion", BUILTIN_NOTION),
        ] {
            match serde_yaml::from_str::<IntegrationDefinition>(yaml) {
                Ok(def) => {
                    debug!(name = %def.name, "loaded built-in integration");
                    builtin.insert(def.name.clone(), def);
                }
                Err(e) => {
                    warn!(builtin_name = name, error = %e, "failed to parse built-in YAML");
                }
            }
        }

        Self {
            builtin,
            user_cache: RwLock::new(UserDefinitionCache::new()),
        }
    }

    /// Get an integration definition by name.
    ///
    /// Returns the user override if one exists, otherwise falls back to the
    /// built-in definition.
    pub fn get(&self, name: &str) -> Option<IntegrationDefinition> {
        // Check user definitions first (override built-ins)
        self.refresh_user_cache();
        if let Ok(cache) = self.user_cache.read() {
            if let Some(def) = cache.definitions.get(name) {
                debug!(name, "returning user-defined integration");
                return Some(def.clone());
            }
        }

        // Fall back to built-in
        self.builtin.get(name).cloned()
    }

    /// Get a built-in integration definition by name (ignores user overrides).
    pub fn get_builtin(&self, name: &str) -> Option<IntegrationDefinition> {
        self.builtin.get(name).cloned()
    }

    /// List all available integration names (union of built-in and user), sorted.
    pub fn list_all(&self) -> Vec<String> {
        self.refresh_user_cache();
        let mut names: Vec<String> = self.builtin.keys().cloned().collect();

        if let Ok(cache) = self.user_cache.read() {
            for key in cache.definitions.keys() {
                if !names.contains(key) {
                    names.push(key.clone());
                }
            }
        }

        names.sort();
        names
    }

    /// Refresh the user definition cache if enough time has elapsed since the
    /// last scan (at most once per second).
    fn refresh_user_cache(&self) {
        let should_scan = {
            let cache = match self.user_cache.read() {
                Ok(c) => c,
                Err(_) => return,
            };
            cache.last_scan.elapsed() >= std::time::Duration::from_secs(1)
        };

        if !should_scan {
            return;
        }

        let mut cache = match self.user_cache.write() {
            Ok(c) => c,
            Err(_) => return,
        };

        // Double-check after acquiring write lock
        if cache.last_scan.elapsed() < std::time::Duration::from_secs(1) {
            return;
        }

        let integrations_dir = Self::user_integrations_dir();
        if !integrations_dir.is_dir() {
            cache.last_scan = Instant::now();
            return;
        }

        let entries = match std::fs::read_dir(&integrations_dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "failed to read user integrations directory");
                cache.last_scan = Instant::now();
                return;
            }
        };

        let mut new_mtimes: HashMap<PathBuf, SystemTime> = HashMap::new();
        let mut changed = false;

        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str());
            if !matches!(ext, Some("yaml" | "yml")) {
                continue;
            }

            let mtime = match entry.metadata().and_then(|m| m.modified()) {
                Ok(t) => t,
                Err(_) => continue,
            };

            new_mtimes.insert(path.clone(), mtime);

            // Check if file is new or modified
            if cache.mtimes.get(&path) != Some(&mtime) {
                changed = true;
            }
        }

        // Also detect deletions
        if new_mtimes.len() != cache.mtimes.len() {
            changed = true;
        }

        if changed {
            debug!("reloading user integration definitions");
            let mut definitions = HashMap::new();

            for path in new_mtimes.keys() {
                match std::fs::read_to_string(path) {
                    Ok(contents) => {
                        match serde_yaml::from_str::<IntegrationDefinition>(&contents) {
                            Ok(def) => {
                                debug!(name = %def.name, path = %path.display(), "loaded user integration");
                                definitions.insert(def.name.clone(), def);
                            }
                            Err(e) => {
                                warn!(path = %path.display(), error = %e, "failed to parse user integration YAML");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "failed to read user integration file");
                    }
                }
            }

            cache.definitions = definitions;
            cache.mtimes = new_mtimes;
        }

        cache.last_scan = Instant::now();
    }

    /// Path to the user integrations directory.
    fn user_integrations_dir() -> PathBuf {
        Config::config_dir().join("integrations")
    }
}

impl Default for IntegrationLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_builtin_github() {
        let loader = IntegrationLoader::new();
        let github = loader
            .get_builtin("github")
            .expect("github built-in should exist");

        assert_eq!(github.name, "github");
        assert_eq!(github.base_url, "https://api.github.com");
        assert_eq!(github.display_name.as_deref(), Some("GitHub"));
        assert_eq!(
            github.docs_url.as_deref(),
            Some("https://docs.github.com/en/rest")
        );

        // Verify operations are present
        assert!(github.operations.contains_key("create_issue"));
        assert!(github.operations.contains_key("get_issue"));
        assert!(github.operations.contains_key("list_issues"));
        assert!(github.operations.contains_key("create_repo"));
        assert_eq!(github.operations.len(), 4);
    }

    #[test]
    fn test_list_builtin_services() {
        let loader = IntegrationLoader::new();
        let names = loader.list_all();

        for expected in &["github", "slack", "openai", "stripe", "notion"] {
            assert!(
                names.contains(&expected.to_string()),
                "list should contain {}, got: {:?}",
                expected,
                names
            );
        }
        assert_eq!(names.len(), 5, "expected 5 built-ins, got: {:?}", names);
    }

    #[test]
    fn test_load_builtin_slack() {
        let loader = IntegrationLoader::new();
        let slack = loader
            .get_builtin("slack")
            .expect("slack built-in should exist");

        assert_eq!(slack.name, "slack");
        assert_eq!(slack.base_url, "https://slack.com/api");
        assert_eq!(slack.display_name.as_deref(), Some("Slack"));
        assert!(slack.operations.contains_key("send_message"));
        assert!(slack.operations.contains_key("list_channels"));
        assert!(slack.operations.contains_key("get_user"));
        assert_eq!(slack.operations.len(), 3);
    }

    #[test]
    fn test_load_builtin_openai() {
        let loader = IntegrationLoader::new();
        let openai = loader
            .get_builtin("openai")
            .expect("openai built-in should exist");

        assert_eq!(openai.name, "openai");
        assert_eq!(openai.base_url, "https://api.openai.com/v1");
        assert_eq!(openai.display_name.as_deref(), Some("OpenAI"));
        assert!(openai.operations.contains_key("chat_completion"));
        assert!(openai.operations.contains_key("create_embedding"));
        assert!(openai.operations.contains_key("list_models"));
        assert_eq!(openai.operations.len(), 3);
    }

    #[test]
    fn test_load_builtin_stripe() {
        let loader = IntegrationLoader::new();
        let stripe = loader
            .get_builtin("stripe")
            .expect("stripe built-in should exist");

        assert_eq!(stripe.name, "stripe");
        assert_eq!(stripe.base_url, "https://api.stripe.com/v1");
        assert_eq!(stripe.display_name.as_deref(), Some("Stripe"));
        assert!(stripe.operations.contains_key("get_customer"));
        assert!(stripe.operations.contains_key("list_customers"));
        assert!(stripe.operations.contains_key("list_charges"));
        assert!(stripe.operations.contains_key("get_balance"));
        assert!(stripe.operations.contains_key("create_customer"));
        assert!(stripe.operations.contains_key("create_charge"));
        assert_eq!(stripe.operations.len(), 6);
    }

    #[test]
    fn test_load_builtin_notion() {
        let loader = IntegrationLoader::new();
        let notion = loader
            .get_builtin("notion")
            .expect("notion built-in should exist");

        assert_eq!(notion.name, "notion");
        assert_eq!(notion.base_url, "https://api.notion.com/v1");
        assert_eq!(notion.display_name.as_deref(), Some("Notion"));
        assert!(notion.operations.contains_key("query_database"));
        assert!(notion.operations.contains_key("create_page"));
        assert!(notion.operations.contains_key("get_page"));
        assert!(notion.operations.contains_key("search"));
        assert_eq!(notion.operations.len(), 4);
    }

    #[test]
    fn test_builtin_not_found() {
        let loader = IntegrationLoader::new();
        assert!(loader.get("nonexistent_service_xyz").is_none());
        assert!(loader.get_builtin("nonexistent_service_xyz").is_none());
    }
}
