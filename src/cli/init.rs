// r8r init wizard — service detection and configuration.

use crate::cli::output::Output;
use crate::config;
use std::io::{self, BufRead, Write};

/// Result of detecting a service or capability.
pub struct DetectionResult {
    pub name: &'static str,
    pub detected: bool,
    pub detail: String,
}

/// Detect the user's shell from $SHELL.
pub fn detect_shell() -> DetectionResult {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let name = if shell.contains("zsh") {
        "zsh"
    } else if shell.contains("bash") {
        "bash"
    } else if shell.contains("fish") {
        "fish"
    } else {
        "unknown"
    };
    DetectionResult {
        name: "Shell",
        detected: !shell.is_empty(),
        detail: format!("{} detected", name),
    }
}

/// Detect an environment variable key.
pub fn detect_env_key(key: &str) -> DetectionResult {
    let present = std::env::var(key).is_ok();
    DetectionResult {
        name: "EnvKey",
        detected: present,
        detail: if present {
            "configured".into()
        } else {
            "not found".into()
        },
    }
}

/// Detect Ollama by calling its API.
pub async fn detect_ollama() -> DetectionResult {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    match client.get("http://localhost:11434/api/tags").send().await {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let models: Vec<String> = body["models"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|m| m["name"].as_str().map(String::from))
                .collect();
            let detail = if models.is_empty() {
                "running (no models pulled)".into()
            } else {
                format!("running ({})", models.first().unwrap())
            };
            DetectionResult {
                name: "Ollama",
                detected: true,
                detail,
            }
        }
        _ => DetectionResult {
            name: "Ollama",
            detected: false,
            detail: "not running".into(),
        },
    }
}

/// Detect Docker by running `docker info`.
pub async fn detect_docker() -> DetectionResult {
    match tokio::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
    {
        Ok(status) if status.success() => DetectionResult {
            name: "Docker",
            detected: true,
            detail: "running (for sandboxed execution)".into(),
        },
        _ => DetectionResult {
            name: "Docker",
            detected: false,
            detail: "not available".into(),
        },
    }
}

fn print_detection(result: &DetectionResult) {
    let icon = if result.detected { "\u{2713}" } else { "\u{2717}" };
    println!("  {} {:<14} {}", icon, result.name, result.detail);
}

fn prompt_line(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).ok();
    line.trim().to_string()
}

/// Run the init wizard.
pub async fn run_init(output: &Output, yes: bool, force: bool) -> anyhow::Result<()> {
    let config_dir = config::Config::config_dir();
    let config_path = config_dir.join("config.toml");

    // Guard: existing config
    if config_path.exists() && !force {
        if yes {
            println!(
                "Config already exists at {}. Use --force to overwrite.",
                config_path.display()
            );
            return Ok(());
        }
        println!("Config already exists at {}.", config_path.display());
        let answer = prompt_line("Overwrite? [y/N] ");
        if answer.to_lowercase() != "y" {
            println!("Aborted.");
            return Ok(());
        }
    }

    println!();
    println!("  Welcome to r8r \u{2014} agent-native workflow engine");
    println!();
    println!("  Detecting your environment...");
    println!();

    // Detect services
    let ollama = detect_ollama().await;
    let openai = detect_env_key("OPENAI_API_KEY");
    let anthropic = detect_env_key("ANTHROPIC_API_KEY");
    let docker = detect_docker().await;
    let shell = detect_shell();

    print_detection(&ollama);
    println!(
        "  {} {:<14} {}",
        if openai.detected { "\u{2713}" } else { "\u{2717}" },
        "OpenAI key",
        openai.detail
    );
    println!(
        "  {} {:<14} {}",
        if anthropic.detected { "\u{2713}" } else { "\u{2717}" },
        "Anthropic key",
        anthropic.detail
    );
    print_detection(&docker);
    print_detection(&shell);

    // Choose LLM provider
    println!();
    println!("  \u{2500}\u{2500}\u{2500} LLM Provider \u{2500}\u{2500}\u{2500}");
    println!();

    let provider: String;
    let model: String;

    if yes {
        // Auto-detect: env key > Ollama > none
        if openai.detected {
            provider = "openai".into();
            model = "gpt-4o".into();
        } else if anthropic.detected {
            provider = "anthropic".into();
            model = "claude-sonnet-4-20250514".into();
        } else if ollama.detected {
            provider = "ollama".into();
            model = "llama3.2".into();
        } else {
            provider = String::new();
            model = String::new();
            println!("  No LLM provider detected. You can configure one later.");
        }
    } else {
        // Interactive selection
        let mut options: Vec<(&str, &str)> = Vec::new();
        if ollama.detected {
            options.push(("ollama", "Ollama (detected, free, local)"));
        }
        options.push(("openai", "OpenAI (requires API key)"));
        options.push(("anthropic", "Anthropic (requires API key)"));

        println!("  Which provider do you want to use?");
        for (i, (_, desc)) in options.iter().enumerate() {
            println!("    [{}] {}", i + 1, desc);
        }
        println!();

        let choice = prompt_line("  > ");
        let idx: usize = choice.parse::<usize>().unwrap_or(1).saturating_sub(1);
        let chosen = options.get(idx).unwrap_or(&options[0]);

        provider = chosen.0.to_string();
        model = match provider.as_str() {
            "ollama" => "llama3.2".into(),
            "openai" => "gpt-4o".into(),
            "anthropic" => "claude-sonnet-4-20250514".into(),
            _ => String::new(),
        };

        if !provider.is_empty() {
            println!("  \u{2713} Using {} with {}", provider, model);
        }
    }

    // Write config
    println!();
    println!("  \u{2500}\u{2500}\u{2500} Config \u{2500}\u{2500}\u{2500}");
    println!();

    std::fs::create_dir_all(&config_dir)?;

    let llm_env_hint = match provider.as_str() {
        "openai" => {
            "# export OPENAI_API_KEY=<your-key>\n# export R8R_LLM_BASE_URL=https://api.openai.com/v1\n"
        }
        "anthropic" => "# export ANTHROPIC_API_KEY=<your-key>\n",
        "ollama" => "# Ollama detected at localhost:11434 — no key needed\n",
        _ => "# Set OPENAI_API_KEY or ANTHROPIC_API_KEY, or start Ollama\n",
    };
    let config_content = format!(
        r#"# r8r configuration (generated by r8r init)

[server]
port = 8080
host = "127.0.0.1"

# LLM provider: {provider} / {model}
# Configure via environment variables:
{llm_env_hint}
"#,
    );
    std::fs::write(&config_path, &config_content)?;
    println!("  \u{2713} Wrote {}", config_path.display());

    // If the chosen provider needs an API key and it's not set, prompt
    if !yes && !provider.is_empty() && provider != "ollama" {
        let env_key = match provider.as_str() {
            "openai" => "OPENAI_API_KEY",
            "anthropic" => "ANTHROPIC_API_KEY",
            _ => "",
        };
        if !env_key.is_empty() && std::env::var(env_key).is_err() {
            println!();
            let key_value = prompt_line(&format!(
                "  Enter your {} (or press Enter to skip): ",
                env_key
            ));
            if !key_value.is_empty() {
                // Store via credential system
                use crate::credentials::CredentialStore;
                let mut cred_store = CredentialStore::load().await?;
                cred_store.set(&provider, None, &key_value).await?;
                println!("  \u{2713} API key stored securely via r8r credentials");
            } else {
                println!(
                    "  Skipped. Set {} later or use: r8r credentials set {}",
                    env_key, provider
                );
            }
        }
    }

    // Final suggestions
    println!();
    output.suggest(&[
        ("r8r chat", "interactive AI assistant"),
        ("r8r create \"...\"", "generate a workflow"),
        ("r8r templates list", "browse ready-made templates"),
    ]);

    Ok(())
}
