//! Workflow templates system.
//!
//! Templates are reusable workflow patterns that can be instantiated
//! with custom variables.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// A workflow template with variable substitution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    /// Template name (e.g., "email-alert")
    pub name: String,

    /// Human-readable description
    pub description: String,

    /// Category for organization (e.g., "notifications", "data", "integrations")
    #[serde(default)]
    pub category: String,

    /// Template variables that must be provided
    #[serde(default)]
    pub variables: Vec<TemplateVariable>,

    /// The workflow YAML content (with {{ variable }} placeholders)
    pub content: String,
}

/// A variable that can be substituted in the template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateVariable {
    /// Variable name
    pub name: String,

    /// Description of what this variable is for
    #[serde(default)]
    pub description: String,

    /// Variable type: string, number, boolean
    #[serde(default = "default_var_type")]
    pub var_type: String,

    /// Whether this variable is required
    #[serde(default = "default_true")]
    pub required: bool,

    /// Default value if not provided
    #[serde(default)]
    pub default: Option<String>,

    /// Example value for documentation
    #[serde(default)]
    pub example: Option<String>,
}

fn default_var_type() -> String {
    "string".to_string()
}

fn default_true() -> bool {
    true
}

/// Template registry for managing available templates.
pub struct TemplateRegistry {
    templates: HashMap<String, Template>,
    custom_dir: Option<PathBuf>,
}

impl TemplateRegistry {
    /// Create a new registry with built-in templates.
    pub fn new() -> Self {
        let mut registry = Self {
            templates: HashMap::new(),
            custom_dir: None,
        };

        // Register built-in templates
        registry.register_builtin_templates();

        registry
    }

    /// Create registry with custom template directory.
    pub fn with_custom_dir(custom_dir: impl AsRef<Path>) -> Result<Self> {
        let mut registry = Self::new();
        registry.custom_dir = Some(custom_dir.as_ref().to_path_buf());
        registry.load_custom_templates()?;
        Ok(registry)
    }

    /// Register built-in templates.
    fn register_builtin_templates(&mut self) {
        // Email Alert Template
        self.templates.insert(
            "email-alert".to_string(),
            Template {
                name: "email-alert".to_string(),
                description: "Send email alerts based on webhook triggers".to_string(),
                category: "notifications".to_string(),
                variables: vec![
                    TemplateVariable {
                        name: "workflow_name".to_string(),
                        description: "Name for this workflow".to_string(),
                        var_type: "string".to_string(),
                        required: true,
                        default: None,
                        example: Some("order-alerts".to_string()),
                    },
                    TemplateVariable {
                        name: "recipient_email".to_string(),
                        description: "Email address to send alerts to".to_string(),
                        var_type: "string".to_string(),
                        required: true,
                        default: None,
                        example: Some("alerts@example.com".to_string()),
                    },
                    TemplateVariable {
                        name: "sender_email".to_string(),
                        description: "Email address to send from".to_string(),
                        var_type: "string".to_string(),
                        required: true,
                        default: None,
                        example: Some("noreply@example.com".to_string()),
                    },
                    TemplateVariable {
                        name: "email_provider".to_string(),
                        description: "Email provider (resend, sendgrid, smtp)".to_string(),
                        var_type: "string".to_string(),
                        required: false,
                        default: Some("resend".to_string()),
                        example: None,
                    },
                ],
                content: r#"name: {{ workflow_name }}
description: Email alert workflow

triggers:
  - type: webhook
    path: /{{ workflow_name }}

nodes:
  - id: send-alert
    type: email
    config:
      provider: {{ email_provider }}
      to: "{{ recipient_email }}"
      from: "{{ sender_email }}"
      subject: "Alert: {{ input.title }}"
      body: |
        Alert received at {{ $now }}

        {{ input.message }}
      credential: email_api_key
"#.to_string(),
            },
        );

        // Slack Notification Template
        self.templates.insert(
            "slack-notify".to_string(),
            Template {
                name: "slack-notify".to_string(),
                description: "Send Slack notifications on events".to_string(),
                category: "notifications".to_string(),
                variables: vec![
                    TemplateVariable {
                        name: "workflow_name".to_string(),
                        description: "Name for this workflow".to_string(),
                        var_type: "string".to_string(),
                        required: true,
                        default: None,
                        example: Some("deploy-notify".to_string()),
                    },
                    TemplateVariable {
                        name: "slack_channel".to_string(),
                        description: "Slack channel to post to".to_string(),
                        var_type: "string".to_string(),
                        required: true,
                        default: None,
                        example: Some("#deployments".to_string()),
                    },
                ],
                content: r#"name: {{ workflow_name }}
description: Slack notification workflow

triggers:
  - type: webhook
    path: /{{ workflow_name }}

nodes:
  - id: notify-slack
    type: slack
    config:
      channel: "{{ slack_channel }}"
      text: "{{ input.message }}"
      blocks:
        - type: section
          text:
            type: mrkdwn
            text: "*{{ input.title }}*\n{{ input.message }}"
      credential: slack_token
"#.to_string(),
            },
        );

        // HTTP Webhook Processor Template
        self.templates.insert(
            "webhook-processor".to_string(),
            Template {
                name: "webhook-processor".to_string(),
                description: "Process incoming webhooks and forward to an API".to_string(),
                category: "integrations".to_string(),
                variables: vec![
                    TemplateVariable {
                        name: "workflow_name".to_string(),
                        description: "Name for this workflow".to_string(),
                        var_type: "string".to_string(),
                        required: true,
                        default: None,
                        example: Some("github-webhook".to_string()),
                    },
                    TemplateVariable {
                        name: "target_url".to_string(),
                        description: "URL to forward processed data to".to_string(),
                        var_type: "string".to_string(),
                        required: true,
                        default: None,
                        example: Some("https://api.example.com/events".to_string()),
                    },
                ],
                content: r#"name: {{ workflow_name }}
description: Webhook processor workflow

triggers:
  - type: webhook
    path: /{{ workflow_name }}

nodes:
  - id: transform-data
    type: transform
    config:
      expression: |
        #{
          "event": input.event,
          "timestamp": input.timestamp ?? now(),
          "data": input
        }

  - id: forward-request
    type: http
    config:
      url: "{{ target_url }}"
      method: POST
      headers:
        Content-Type: application/json
      body: "{{ nodes.transform-data }}"
    depends_on: [transform-data]
    retry:
      max_attempts: 3
      backoff: exponential
"#.to_string(),
            },
        );

        // Data Pipeline Template
        self.templates.insert(
            "data-pipeline".to_string(),
            Template {
                name: "data-pipeline".to_string(),
                description: "Fetch, transform, and store data on a schedule".to_string(),
                category: "data".to_string(),
                variables: vec![
                    TemplateVariable {
                        name: "workflow_name".to_string(),
                        description: "Name for this workflow".to_string(),
                        var_type: "string".to_string(),
                        required: true,
                        default: None,
                        example: Some("daily-sync".to_string()),
                    },
                    TemplateVariable {
                        name: "source_url".to_string(),
                        description: "API URL to fetch data from".to_string(),
                        var_type: "string".to_string(),
                        required: true,
                        default: None,
                        example: Some("https://api.example.com/data".to_string()),
                    },
                    TemplateVariable {
                        name: "cron_schedule".to_string(),
                        description: "Cron expression for scheduling".to_string(),
                        var_type: "string".to_string(),
                        required: false,
                        default: Some("0 0 * * *".to_string()),
                        example: Some("0 */6 * * *".to_string()),
                    },
                ],
                content: r#"name: {{ workflow_name }}
description: Scheduled data pipeline

triggers:
  - type: cron
    schedule: "{{ cron_schedule }}"

nodes:
  - id: fetch-data
    type: http
    config:
      url: "{{ source_url }}"
      method: GET
    retry:
      max_attempts: 3

  - id: transform
    type: transform
    config:
      expression: |
        input.body.items.map(|item| #{
          "id": item.id,
          "processed_at": now(),
          "data": item
        })
    depends_on: [fetch-data]

  - id: log-result
    type: debug
    config:
      label: "Pipeline result"
      level: info
    depends_on: [transform]
"#.to_string(),
            },
        );

        // Scheduled Report Template
        self.templates.insert(
            "scheduled-report".to_string(),
            Template {
                name: "scheduled-report".to_string(),
                description: "Generate and send reports on a schedule".to_string(),
                category: "reports".to_string(),
                variables: vec![
                    TemplateVariable {
                        name: "workflow_name".to_string(),
                        description: "Name for this workflow".to_string(),
                        var_type: "string".to_string(),
                        required: true,
                        default: None,
                        example: Some("weekly-report".to_string()),
                    },
                    TemplateVariable {
                        name: "report_email".to_string(),
                        description: "Email to send report to".to_string(),
                        var_type: "string".to_string(),
                        required: true,
                        default: None,
                        example: Some("reports@example.com".to_string()),
                    },
                    TemplateVariable {
                        name: "data_source_url".to_string(),
                        description: "API to fetch report data from".to_string(),
                        var_type: "string".to_string(),
                        required: true,
                        default: None,
                        example: Some("https://api.example.com/metrics".to_string()),
                    },
                    TemplateVariable {
                        name: "cron_schedule".to_string(),
                        description: "When to run the report".to_string(),
                        var_type: "string".to_string(),
                        required: false,
                        default: Some("0 9 * * 1".to_string()),
                        example: None,
                    },
                ],
                content: r#"name: {{ workflow_name }}
description: Scheduled report generator

triggers:
  - type: cron
    schedule: "{{ cron_schedule }}"

nodes:
  - id: fetch-metrics
    type: http
    config:
      url: "{{ data_source_url }}"
      method: GET

  - id: generate-report
    type: transform
    config:
      expression: |
        let data = input.body;
        #{
          "title": "Report - " + now(),
          "summary": data.summary ?? "No summary",
          "metrics": data.metrics ?? []
        }
    depends_on: [fetch-metrics]

  - id: send-report
    type: email
    config:
      provider: resend
      to: "{{ report_email }}"
      from: "reports@r8r.dev"
      subject: "{{ workflow_name }} - {{ $now }}"
      html: |
        <h1>{{ nodes.generate-report.title }}</h1>
        <p>{{ nodes.generate-report.summary }}</p>
      credential: email_api_key
    depends_on: [generate-report]
"#.to_string(),
            },
        );
    }

    /// Load templates from custom directory.
    fn load_custom_templates(&mut self) -> Result<()> {
        let dir = match &self.custom_dir {
            Some(d) => d,
            None => return Ok(()),
        };

        if !dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)
            .map_err(|e| Error::Config(format!("Failed to read templates dir: {}", e)))?
        {
            let entry = entry.map_err(|e| Error::Config(format!("Failed to read entry: {}", e)))?;
            let path = entry.path();

            if path.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
                if let Ok(template) = self.load_template_file(&path) {
                    self.templates.insert(template.name.clone(), template);
                }
            }
        }

        Ok(())
    }

    /// Load a template from a YAML file.
    fn load_template_file(&self, path: &Path) -> Result<Template> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("Failed to read template file: {}", e)))?;

        serde_yaml::from_str(&content)
            .map_err(|e| Error::Config(format!("Failed to parse template: {}", e)))
    }

    /// List all available templates.
    pub fn list(&self) -> Vec<&Template> {
        let mut templates: Vec<_> = self.templates.values().collect();
        templates.sort_by(|a, b| a.name.cmp(&b.name));
        templates
    }

    /// List templates by category.
    pub fn list_by_category(&self) -> HashMap<String, Vec<&Template>> {
        let mut by_category: HashMap<String, Vec<&Template>> = HashMap::new();

        for template in self.templates.values() {
            let category = if template.category.is_empty() {
                "other".to_string()
            } else {
                template.category.clone()
            };
            by_category.entry(category).or_default().push(template);
        }

        // Sort templates within each category
        for templates in by_category.values_mut() {
            templates.sort_by(|a, b| a.name.cmp(&b.name));
        }

        by_category
    }

    /// Get a template by name.
    pub fn get(&self, name: &str) -> Option<&Template> {
        self.templates.get(name)
    }

    /// Instantiate a template with given variables.
    pub fn instantiate(
        &self,
        name: &str,
        variables: &HashMap<String, String>,
    ) -> Result<String> {
        let template = self
            .get(name)
            .ok_or_else(|| Error::Config(format!("Template not found: {}", name)))?;

        // Check required variables
        for var in &template.variables {
            if var.required && var.default.is_none() && !variables.contains_key(&var.name) {
                return Err(Error::Config(format!(
                    "Missing required variable: {} - {}",
                    var.name, var.description
                )));
            }
        }

        // Build substitution map with defaults
        let mut subs: HashMap<String, String> = HashMap::new();
        for var in &template.variables {
            if let Some(value) = variables.get(&var.name) {
                subs.insert(var.name.clone(), value.clone());
            } else if let Some(default) = &var.default {
                subs.insert(var.name.clone(), default.clone());
            }
        }

        // Perform substitution
        let mut result = template.content.clone();
        for (key, value) in &subs {
            let placeholder = format!("{{{{ {} }}}}", key);
            result = result.replace(&placeholder, value);
        }

        Ok(result)
    }

    /// Validate that all required variables are provided.
    pub fn validate_variables(
        &self,
        name: &str,
        variables: &HashMap<String, String>,
    ) -> Result<Vec<String>> {
        let template = self
            .get(name)
            .ok_or_else(|| Error::Config(format!("Template not found: {}", name)))?;

        let mut missing = Vec::new();
        for var in &template.variables {
            if var.required && var.default.is_none() && !variables.contains_key(&var.name) {
                missing.push(var.name.clone());
            }
        }

        Ok(missing)
    }
}

impl Default for TemplateRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_has_builtin_templates() {
        let registry = TemplateRegistry::new();
        let templates = registry.list();

        assert!(!templates.is_empty());
        assert!(registry.get("email-alert").is_some());
        assert!(registry.get("slack-notify").is_some());
        assert!(registry.get("webhook-processor").is_some());
        assert!(registry.get("data-pipeline").is_some());
    }

    #[test]
    fn test_instantiate_template() {
        let registry = TemplateRegistry::new();

        let mut vars = HashMap::new();
        vars.insert("workflow_name".to_string(), "my-alerts".to_string());
        vars.insert("recipient_email".to_string(), "test@example.com".to_string());
        vars.insert("sender_email".to_string(), "noreply@example.com".to_string());

        let result = registry.instantiate("email-alert", &vars).unwrap();

        assert!(result.contains("name: my-alerts"));
        assert!(result.contains("to: \"test@example.com\""));
        assert!(result.contains("from: \"noreply@example.com\""));
    }

    #[test]
    fn test_missing_required_variable() {
        let registry = TemplateRegistry::new();

        let vars = HashMap::new(); // Empty - missing required vars

        let result = registry.instantiate("email-alert", &vars);
        assert!(result.is_err());
    }

    #[test]
    fn test_default_values() {
        let registry = TemplateRegistry::new();

        let mut vars = HashMap::new();
        vars.insert("workflow_name".to_string(), "my-alerts".to_string());
        vars.insert("recipient_email".to_string(), "test@example.com".to_string());
        vars.insert("sender_email".to_string(), "noreply@example.com".to_string());
        // email_provider has default "resend"

        let result = registry.instantiate("email-alert", &vars).unwrap();
        assert!(result.contains("provider: resend"));
    }

    #[test]
    fn test_list_by_category() {
        let registry = TemplateRegistry::new();
        let by_category = registry.list_by_category();

        assert!(by_category.contains_key("notifications"));
        assert!(by_category.contains_key("integrations"));
        assert!(by_category.contains_key("data"));
    }
}
