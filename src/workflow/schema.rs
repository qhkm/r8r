//! Workflow JSON Schema for import/export validation.
//!
//! This module provides schema validation for workflow definitions,
//! ensuring they conform to the expected structure before import.

use jsonschema::Validator;
use serde_json::{json, Value};

use crate::error::{Error, Result};

/// JSON Schema for workflow definitions.
pub fn workflow_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["name", "nodes"],
        "properties": {
            "name": {
                "type": "string",
                "minLength": 1,
                "maxLength": 128,
                "pattern": "^[a-zA-Z][a-zA-Z0-9_-]*$",
                "description": "Workflow name (alphanumeric, hyphens, underscores)"
            },
            "description": {
                "type": "string",
                "maxLength": 1024,
                "description": "Optional workflow description"
            },
            "version": {
                "type": "string",
                "pattern": "^[0-9]+\\.[0-9]+\\.[0-9]+$",
                "description": "Semantic version (e.g., 1.0.0)"
            },
            "settings": {
                "$ref": "#/$defs/settings"
            },
            "input_schema": {
                "type": "object",
                "description": "JSON Schema for validating workflow input"
            },
            "triggers": {
                "type": "array",
                "items": {
                    "$ref": "#/$defs/trigger"
                },
                "description": "List of workflow triggers"
            },
            "nodes": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "$ref": "#/$defs/node"
                },
                "description": "List of workflow nodes"
            }
        },
        "additionalProperties": false,
        "$defs": {
            "settings": {
                "type": "object",
                "properties": {
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 86400,
                        "default": 3600
                    },
                    "max_concurrency": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 1000,
                        "default": 10
                    },
                    "max_for_each_items": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100000,
                        "default": 10000
                    },
                    "chunk_size": {
                        "type": "integer",
                        "minimum": 0
                    },
                    "store_traces": {
                        "type": "boolean",
                        "default": true
                    },
                    "debug": {
                        "type": "boolean",
                        "default": false
                    },
                    "timezone": {
                        "type": "string",
                        "description": "IANA timezone (e.g., America/New_York)"
                    }
                },
                "additionalProperties": false
            },
            "trigger": {
                "type": "object",
                "required": ["type"],
                "properties": {
                    "type": {
                        "type": "string",
                        "enum": ["cron", "webhook", "event", "manual"]
                    },
                    "schedule": {
                        "type": "string",
                        "description": "Cron expression for cron triggers"
                    },
                    "path": {
                        "type": "string",
                        "description": "Webhook path for webhook triggers"
                    },
                    "event": {
                        "type": "string",
                        "description": "Event name for event triggers"
                    },
                    "signature": {
                        "type": "object",
                        "properties": {
                            "scheme": {
                                "type": "string",
                                "enum": ["github", "stripe", "slack", "generic"]
                            },
                            "secret_env": {
                                "type": "string"
                            },
                            "secret_credential": {
                                "type": "string"
                            }
                        }
                    }
                },
                "additionalProperties": true
            },
            "node": {
                "type": "object",
                "required": ["id", "type"],
                "properties": {
                    "id": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 64,
                        "pattern": "^[a-zA-Z][a-zA-Z0-9_-]*$",
                        "description": "Unique node identifier"
                    },
                    "type": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Node type (e.g., http, agent, transform)"
                    },
                    "config": {
                        "type": "object",
                        "description": "Node-specific configuration"
                    },
                    "depends_on": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "description": "List of node IDs this node depends on"
                    },
                    "condition": {
                        "type": "string",
                        "description": "Rhai expression for conditional execution"
                    },
                    "for_each": {
                        "type": "string",
                        "description": "Expression returning array to iterate over"
                    },
                    "retry": {
                        "$ref": "#/$defs/retry"
                    },
                    "on_error": {
                        "$ref": "#/$defs/on_error"
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 3600
                    }
                },
                "additionalProperties": false
            },
            "retry": {
                "type": "object",
                "properties": {
                    "max_attempts": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 10,
                        "default": 3
                    },
                    "backoff": {
                        "type": "string",
                        "enum": ["fixed", "linear", "exponential"],
                        "default": "exponential"
                    },
                    "delay_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 300,
                        "default": 1
                    },
                    "max_delay_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 3600
                    }
                },
                "additionalProperties": false
            },
            "on_error": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["fail", "continue", "fallback"],
                        "default": "fail"
                    },
                    "fallback_value": {
                        "description": "Value to use when action is 'fallback'"
                    }
                },
                "additionalProperties": false
            }
        }
    })
}

/// Validator for workflow definitions.
pub struct WorkflowSchemaValidator {
    validator: Validator,
}

impl WorkflowSchemaValidator {
    /// Create a new workflow schema validator.
    pub fn new() -> Result<Self> {
        let schema = workflow_schema();
        let validator = Validator::new(&schema)
            .map_err(|e| Error::Validation(format!("Invalid workflow schema: {}", e)))?;
        Ok(Self { validator })
    }

    /// Validate a workflow definition.
    ///
    /// The input should be a parsed YAML/JSON workflow definition.
    pub fn validate(&self, workflow: &Value) -> Result<()> {
        let result = self.validator.validate(workflow);
        if let Err(error) = result {
            return Err(Error::Validation(format!(
                "Workflow validation failed: {} at {}",
                error,
                error.instance_path
            )));
        }
        Ok(())
    }

    /// Validate and return detailed errors.
    pub fn validate_detailed(&self, workflow: &Value) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        if let Err(error) = self.validator.validate(workflow) {
            errors.push(ValidationError {
                path: error.instance_path.to_string(),
                message: error.to_string(),
            });
        }

        errors
    }
}

impl Default for WorkflowSchemaValidator {
    fn default() -> Self {
        Self::new().expect("Failed to create workflow schema validator")
    }
}

/// A validation error with path information.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// JSON path to the invalid element.
    pub path: String,
    /// Error message.
    pub message: String,
}

/// Validate a workflow YAML string for import.
///
/// This performs schema validation on the parsed workflow definition.
pub fn validate_workflow_yaml(yaml: &str) -> Result<Value> {
    // Parse YAML to JSON Value
    let workflow: Value = serde_yaml::from_str(yaml)
        .map_err(|e| Error::Validation(format!("Invalid YAML: {}", e)))?;

    // Validate against schema
    let validator = WorkflowSchemaValidator::new()?;
    validator.validate(&workflow)?;

    Ok(workflow)
}

/// Export a workflow as validated YAML.
///
/// This ensures the workflow conforms to the schema before export.
pub fn export_workflow_yaml(workflow: &Value) -> Result<String> {
    // Validate before export
    let validator = WorkflowSchemaValidator::new()?;
    validator.validate(workflow)?;

    // Convert to YAML
    let yaml = serde_yaml::to_string(workflow)
        .map_err(|e| Error::Validation(format!("Failed to serialize workflow: {}", e)))?;

    Ok(yaml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_minimal_workflow() {
        let workflow = json!({
            "name": "test-workflow",
            "nodes": [
                {
                    "id": "step1",
                    "type": "http",
                    "config": {
                        "url": "https://api.example.com"
                    }
                }
            ]
        });

        let validator = WorkflowSchemaValidator::new().unwrap();
        assert!(validator.validate(&workflow).is_ok());
    }

    #[test]
    fn test_valid_full_workflow() {
        let workflow = json!({
            "name": "full-workflow",
            "description": "A complete workflow example",
            "version": "1.0.0",
            "settings": {
                "timeout_seconds": 300,
                "max_concurrency": 5,
                "debug": true
            },
            "triggers": [
                {
                    "type": "cron",
                    "schedule": "*/5 * * * *"
                },
                {
                    "type": "webhook",
                    "path": "/hook/test"
                }
            ],
            "nodes": [
                {
                    "id": "fetch",
                    "type": "http",
                    "config": {
                        "url": "https://api.example.com/data",
                        "method": "GET"
                    },
                    "retry": {
                        "max_attempts": 3,
                        "backoff": "exponential",
                        "delay_seconds": 2
                    }
                },
                {
                    "id": "process",
                    "type": "transform",
                    "config": {
                        "expression": "input.data"
                    },
                    "depends_on": ["fetch"],
                    "condition": "input.status == 200",
                    "on_error": {
                        "action": "fallback",
                        "fallback_value": []
                    }
                }
            ]
        });

        let validator = WorkflowSchemaValidator::new().unwrap();
        assert!(validator.validate(&workflow).is_ok());
    }

    #[test]
    fn test_missing_name() {
        let workflow = json!({
            "nodes": [
                {"id": "step1", "type": "http"}
            ]
        });

        let validator = WorkflowSchemaValidator::new().unwrap();
        assert!(validator.validate(&workflow).is_err());
    }

    #[test]
    fn test_missing_nodes() {
        let workflow = json!({
            "name": "test"
        });

        let validator = WorkflowSchemaValidator::new().unwrap();
        assert!(validator.validate(&workflow).is_err());
    }

    #[test]
    fn test_empty_nodes() {
        let workflow = json!({
            "name": "test",
            "nodes": []
        });

        let validator = WorkflowSchemaValidator::new().unwrap();
        assert!(validator.validate(&workflow).is_err());
    }

    #[test]
    fn test_invalid_node_id() {
        let workflow = json!({
            "name": "test",
            "nodes": [
                {"id": "123invalid", "type": "http"}
            ]
        });

        let validator = WorkflowSchemaValidator::new().unwrap();
        assert!(validator.validate(&workflow).is_err());
    }

    #[test]
    fn test_invalid_workflow_name() {
        let workflow = json!({
            "name": "123-invalid",
            "nodes": [
                {"id": "step1", "type": "http"}
            ]
        });

        let validator = WorkflowSchemaValidator::new().unwrap();
        assert!(validator.validate(&workflow).is_err());
    }

    #[test]
    fn test_invalid_retry_config() {
        let workflow = json!({
            "name": "test",
            "nodes": [
                {
                    "id": "step1",
                    "type": "http",
                    "retry": {
                        "max_attempts": 100,  // Too high
                        "backoff": "invalid"  // Invalid backoff type
                    }
                }
            ]
        });

        let validator = WorkflowSchemaValidator::new().unwrap();
        assert!(validator.validate(&workflow).is_err());
    }

    #[test]
    fn test_invalid_trigger_type() {
        let workflow = json!({
            "name": "test",
            "triggers": [
                {"type": "invalid"}
            ],
            "nodes": [
                {"id": "step1", "type": "http"}
            ]
        });

        let validator = WorkflowSchemaValidator::new().unwrap();
        assert!(validator.validate(&workflow).is_err());
    }

    #[test]
    fn test_validate_yaml_string() {
        let yaml = r#"
name: yaml-workflow
nodes:
  - id: step1
    type: http
    config:
      url: https://example.com
"#;

        let result = validate_workflow_yaml(yaml);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_invalid_yaml() {
        let yaml = r#"
name: invalid
nodes: "not an array"
"#;

        let result = validate_workflow_yaml(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_export_workflow() {
        let workflow = json!({
            "name": "export-test",
            "nodes": [
                {"id": "step1", "type": "http"}
            ]
        });

        let result = export_workflow_yaml(&workflow);
        assert!(result.is_ok());
        let yaml = result.unwrap();
        assert!(yaml.contains("export-test"));
    }

    #[test]
    fn test_additional_properties_rejected() {
        let workflow = json!({
            "name": "test",
            "unknown_field": "value",
            "nodes": [
                {"id": "step1", "type": "http"}
            ]
        });

        let validator = WorkflowSchemaValidator::new().unwrap();
        assert!(validator.validate(&workflow).is_err());
    }

    #[test]
    fn test_settings_validation() {
        let workflow = json!({
            "name": "test",
            "settings": {
                "timeout_seconds": 100000  // Exceeds maximum
            },
            "nodes": [
                {"id": "step1", "type": "http"}
            ]
        });

        let validator = WorkflowSchemaValidator::new().unwrap();
        assert!(validator.validate(&workflow).is_err());
    }
}
