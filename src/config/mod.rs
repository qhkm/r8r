//! Configuration management.
//!
//! r8r configuration can come from:
//! - Environment variables (R8R_*)
//! - Config file (~/.config/r8r/config.toml)
//! - Shared Kitakod config (~/.config/kitakod/config.toml)

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// r8r configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Server configuration
    #[serde(default)]
    pub server: ServerConfig,

    /// Storage configuration
    #[serde(default)]
    pub storage: StorageConfig,

    /// ZeptoClaw integration
    #[serde(default)]
    pub agent: AgentConfig,
}

/// Server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// HTTP port
    #[serde(default = "default_port")]
    pub port: u16,

    /// Host to bind to
    #[serde(default = "default_host")]
    pub host: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
        }
    }
}

fn default_port() -> u16 {
    8080
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

/// Storage configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Path to SQLite database
    #[serde(default)]
    pub database_path: Option<PathBuf>,
}

/// ZeptoClaw agent integration configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// ZeptoClaw API endpoint
    #[serde(default = "default_agent_endpoint")]
    pub endpoint: String,

    /// Default timeout for agent calls (seconds)
    #[serde(default = "default_agent_timeout")]
    pub timeout_seconds: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            endpoint: default_agent_endpoint(),
            timeout_seconds: default_agent_timeout(),
        }
    }
}

fn default_agent_endpoint() -> String {
    std::env::var("R8R_AGENT_ENDPOINT")
        .or_else(|_| std::env::var("ZEPTOCLAW_API_URL"))
        .unwrap_or_else(|_| "http://localhost:3000/api/chat".to_string())
}

fn default_agent_timeout() -> u64 {
    30
}

impl Config {
    /// Load configuration from default locations.
    pub fn load() -> Self {
        let mut config = Self::default();

        // Primary config file: ~/.config/r8r/config.toml
        let primary_path = Self::config_dir().join("config.toml");
        if let Ok(partial) = Self::load_partial_from_path(&primary_path) {
            config.apply_partial(partial);
        }

        // Shared Kitakod config: ~/.config/kitakod/config.toml with [r8r] table
        if let Some(shared_path) = dirs::config_dir().map(|p| p.join("kitakod").join("config.toml"))
        {
            if let Ok(partial) = Self::load_nested_partial_from_path(&shared_path, "r8r") {
                config.apply_partial(partial);
            }
        }

        config.apply_env_overrides();
        config
    }

    /// Get the data directory.
    pub fn data_dir() -> PathBuf {
        dirs::data_dir()
            .map(|d| d.join("r8r"))
            .unwrap_or_else(|| PathBuf::from(".r8r"))
    }

    /// Get the config directory.
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .map(|d| d.join("r8r"))
            .unwrap_or_else(|| PathBuf::from(".r8r"))
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(port) = std::env::var("R8R_SERVER_PORT") {
            if let Ok(parsed) = port.parse::<u16>() {
                self.server.port = parsed;
            }
        }
        if let Ok(host) = std::env::var("R8R_SERVER_HOST") {
            self.server.host = host;
        }
        if let Ok(endpoint) = std::env::var("R8R_AGENT_ENDPOINT") {
            self.agent.endpoint = endpoint;
        }
        if let Ok(timeout) = std::env::var("R8R_AGENT_TIMEOUT_SECONDS") {
            if let Ok(parsed) = timeout.parse::<u64>() {
                self.agent.timeout_seconds = parsed;
            }
        }
        if let Ok(path) = std::env::var("R8R_DATABASE_PATH") {
            self.storage.database_path = Some(PathBuf::from(path));
        }
    }

    fn load_partial_from_path(path: &Path) -> std::result::Result<PartialConfig, ()> {
        let content = std::fs::read_to_string(path).map_err(|_| ())?;
        toml::from_str(&content).map_err(|_| ())
    }

    fn load_nested_partial_from_path(
        path: &Path,
        section: &str,
    ) -> std::result::Result<PartialConfig, ()> {
        let content = std::fs::read_to_string(path).map_err(|_| ())?;
        let value: toml::Value = toml::from_str(&content).map_err(|_| ())?;
        let section_value = value.get(section).cloned().ok_or(())?;
        section_value.try_into().map_err(|_| ())
    }

    fn apply_partial(&mut self, partial: PartialConfig) {
        if let Some(server) = partial.server {
            self.server = server;
        }
        if let Some(storage) = partial.storage {
            self.storage = storage;
        }
        if let Some(agent) = partial.agent {
            self.agent = agent;
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct PartialConfig {
    server: Option<ServerConfig>,
    storage: Option<StorageConfig>,
    agent: Option<AgentConfig>,
}
