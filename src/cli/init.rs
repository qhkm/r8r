/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
// r8r init wizard — service detection and configuration.

use crate::cli::output::Output;
use crate::config;
use serde::Serialize;
use std::io::{self, BufRead, Write};

/// Result of detecting a service or capability.
#[derive(Debug, Clone, Serialize)]
pub struct DetectionResult {
    pub name: &'static str,
    pub detected: bool,
    pub detail: String,
}

#[derive(Serialize)]
struct InitSummary {
    ok: bool,
    config_path: String,
    overwritten: bool,
    provider: Option<String>,
    model: Option<String>,
    credential_saved: bool,
    detections: Vec<DetectionResult>,
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

fn print_detection(result: &DetectionResult, to_stderr: bool) {
    let icon = if result.detected {
        "\u{2713}"
    } else {
        "\u{2717}"
    };
    if to_stderr {
        eprintln!("  {} {:<14} {}", icon, result.name, result.detail);
    } else {
        println!("  {} {:<14} {}", icon, result.name, result.detail);
    }
}

fn print_line(line: &str, to_stderr: bool) {
    if to_stderr {
        eprintln!("{}", line);
    } else {
        println!("{}", line);
    }
}

fn prompt_line(prompt: &str, to_stderr: bool) -> String {
    if to_stderr {
        eprint!("{}", prompt);
        io::stderr().flush().ok();
    } else {
        print!("{}", prompt);
        io::stdout().flush().ok();
    }
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).ok();
    line.trim().to_string()
}

fn default_endpoint(provider: &str) -> Option<&'static str> {
    match provider {
        "openai" => Some("https://api.openai.com/v1/chat/completions"),
        "anthropic" => Some("https://api.anthropic.com/v1/messages"),
        "ollama" => Some("http://localhost:11434/api/chat"),
        _ => None,
    }
}

async fn persist_api_key(provider: &str, value: &str) -> anyhow::Result<()> {
    use crate::credentials::CredentialStore;

    let mut cred_store = CredentialStore::load().await?;
    cred_store.set(provider, None, value).await?;
    Ok(())
}

/// Run the init wizard.
pub async fn run_init(output: &Output, yes: bool, force: bool) -> anyhow::Result<()> {
    let config_dir = config::Config::config_dir();
    let config_path = config_dir.join("config.toml");
    let to_stderr = output.is_json();
    let mut overwritten = false;

    // Guard: existing config
    if config_path.exists() && !force {
        if yes {
            if output.is_json() {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "skipped": true,
                        "message": format!(
                            "Config already exists at {}. Use --force to overwrite.",
                            config_path.display()
                        ),
                        "config_path": config_path.display().to_string(),
                    })
                );
            } else {
                println!(
                    "Config already exists at {}. Use --force to overwrite.",
                    config_path.display()
                );
            }
            return Ok(());
        }
        print_line(
            &format!("Config already exists at {}.", config_path.display()),
            to_stderr,
        );
        let answer = prompt_line("Overwrite? [y/N] ", to_stderr);
        if answer.to_lowercase() != "y" {
            if output.is_json() {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "aborted": true,
                        "config_path": config_path.display().to_string(),
                    })
                );
            } else {
                println!("Aborted.");
            }
            return Ok(());
        }
        overwritten = true;
    } else if config_path.exists() && force {
        overwritten = true;
    }

    print_line("", to_stderr);
    print_line(
        "  Welcome to r8r \u{2014} agent-native workflow engine",
        to_stderr,
    );
    print_line("", to_stderr);
    print_line("  Detecting your environment...", to_stderr);
    print_line("", to_stderr);

    // Detect services
    let ollama = detect_ollama().await;
    let openai = detect_env_key("OPENAI_API_KEY");
    let anthropic = detect_env_key("ANTHROPIC_API_KEY");
    let docker = detect_docker().await;
    let shell = detect_shell();
    let detections = vec![
        ollama.clone(),
        DetectionResult {
            name: "OpenAI key",
            detected: openai.detected,
            detail: openai.detail.clone(),
        },
        DetectionResult {
            name: "Anthropic key",
            detected: anthropic.detected,
            detail: anthropic.detail.clone(),
        },
        docker.clone(),
        shell.clone(),
    ];

    print_detection(&ollama, to_stderr);
    print_detection(&detections[1], to_stderr);
    print_detection(&detections[2], to_stderr);
    print_detection(&docker, to_stderr);
    print_detection(&shell, to_stderr);

    // Choose LLM provider
    print_line("", to_stderr);
    print_line(
        "  \u{2500}\u{2500}\u{2500} LLM Provider \u{2500}\u{2500}\u{2500}",
        to_stderr,
    );
    print_line("", to_stderr);

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
            print_line(
                "  No LLM provider detected. You can configure one later.",
                to_stderr,
            );
        }
    } else {
        // Interactive selection
        let mut options: Vec<(&str, &str)> = Vec::new();
        if ollama.detected {
            options.push(("ollama", "Ollama (detected, free, local)"));
        }
        options.push(("openai", "OpenAI (requires API key)"));
        options.push(("anthropic", "Anthropic (requires API key)"));

        print_line("  Which provider do you want to use?", to_stderr);
        for (i, (_, desc)) in options.iter().enumerate() {
            print_line(&format!("    [{}] {}", i + 1, desc), to_stderr);
        }
        print_line("", to_stderr);

        let choice = prompt_line("  > ", to_stderr);
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
            print_line(
                &format!("  \u{2713} Using {} with {}", provider, model),
                to_stderr,
            );
        }
    }

    // Write config
    print_line("", to_stderr);
    print_line(
        "  \u{2500}\u{2500}\u{2500} Config \u{2500}\u{2500}\u{2500}",
        to_stderr,
    );
    print_line("", to_stderr);

    std::fs::create_dir_all(&config_dir)?;
    let llm_section = if provider.is_empty() {
        "# Configure [llm] later with `r8r init --force` or by editing this file.\n".to_string()
    } else {
        format!(
            r#"
[llm]
provider = "{provider}"
model = "{model}"
endpoint = "{endpoint}"
timeout_seconds = 60
"#,
            endpoint = default_endpoint(&provider).unwrap_or_default(),
        )
    };
    let config_content = format!(
        r#"# r8r configuration (generated by r8r init)

[server]
port = 8080
host = "127.0.0.1"
{llm_section}"#,
    );
    std::fs::write(&config_path, &config_content)?;
    print_line(
        &format!("  \u{2713} Wrote {}", config_path.display()),
        to_stderr,
    );

    let mut credential_saved = false;

    if !provider.is_empty() && provider != "ollama" {
        let env_key = match provider.as_str() {
            "openai" => "OPENAI_API_KEY",
            "anthropic" => "ANTHROPIC_API_KEY",
            _ => "",
        };
        if !env_key.is_empty() {
            if let Ok(key_value) = std::env::var(env_key) {
                if !key_value.is_empty() {
                    persist_api_key(&provider, &key_value).await?;
                    credential_saved = true;
                    print_line(
                        &format!("  \u{2713} Stored {} securely via r8r credentials", env_key),
                        to_stderr,
                    );
                }
            } else if !yes {
                print_line("", to_stderr);
                let key_value = prompt_line(
                    &format!("  Enter your {} (or press Enter to skip): ", env_key),
                    to_stderr,
                );
                if !key_value.is_empty() {
                    persist_api_key(&provider, &key_value).await?;
                    credential_saved = true;
                    print_line(
                        "  \u{2713} API key stored securely via r8r credentials",
                        to_stderr,
                    );
                } else {
                    print_line(
                        &format!(
                            "  Skipped. Set {} later or use: r8r credentials set {}",
                            env_key, provider
                        ),
                        to_stderr,
                    );
                }
            }
        }
    }

    let summary = InitSummary {
        ok: true,
        config_path: config_path.display().to_string(),
        overwritten,
        provider: (!provider.is_empty()).then_some(provider.clone()),
        model: (!model.is_empty()).then_some(model.clone()),
        credential_saved,
        detections,
    };

    if output.is_json() {
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
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
