//! OpenAPI specification for the r8r API.

use serde_json::{json, Value};

/// Generate the OpenAPI 3.1 specification for the r8r API.
pub fn generate_openapi_spec() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "r8r Workflow Automation API",
            "description": "Agent-first Rust workflow automation engine API",
            "version": env!("CARGO_PKG_VERSION"),
            "license": {
                "name": "AGPL-3.0-or-later",
                "url": "https://spdx.org/licenses/AGPL-3.0-or-later.html"
            },
            "contact": {
                "name": "r8r",
                "url": "https://github.com/kitakod/r8r"
            }
        },
        "servers": [
            {
                "url": "http://localhost:3000",
                "description": "Local development server"
            }
        ],
        "paths": {
            "/api/health": {
                "get": {
                    "summary": "Health check",
                    "description": "Check the health status of the r8r server",
                    "operationId": "healthCheck",
                    "tags": ["System"],
                    "responses": {
                        "200": {
                            "description": "Server is healthy",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/HealthResponse"
                                    }
                                }
                            }
                        },
                        "500": {
                            "description": "Server is unhealthy",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/ErrorResponse"
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/api/metrics": {
                "get": {
                    "summary": "Prometheus metrics",
                    "description": "Get Prometheus-formatted metrics",
                    "operationId": "getMetrics",
                    "tags": ["System"],
                    "responses": {
                        "200": {
                            "description": "Prometheus metrics",
                            "content": {
                                "text/plain": {
                                    "schema": {
                                        "type": "string"
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/api/workflows": {
                "get": {
                    "summary": "List workflows",
                    "description": "List all registered workflows",
                    "operationId": "listWorkflows",
                    "tags": ["Workflows"],
                    "responses": {
                        "200": {
                            "description": "List of workflows",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "object",
                                        "properties": {
                                            "workflows": {
                                                "type": "array",
                                                "items": {
                                                    "$ref": "#/components/schemas/WorkflowSummary"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/api/workflows/{name}": {
                "get": {
                    "summary": "Get workflow",
                    "description": "Get a workflow by name",
                    "operationId": "getWorkflow",
                    "tags": ["Workflows"],
                    "parameters": [
                        {
                            "name": "name",
                            "in": "path",
                            "required": true,
                            "description": "Workflow name",
                            "schema": {
                                "type": "string"
                            }
                        }
                    ],
                    "responses": {
                        "200": {
                            "description": "Workflow details",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/WorkflowDetail"
                                    }
                                }
                            }
                        },
                        "404": {
                            "description": "Workflow not found",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/ErrorResponse"
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/api/workflows/{name}/execute": {
                "post": {
                    "summary": "Execute workflow",
                    "description": "Execute a workflow with the given input",
                    "operationId": "executeWorkflow",
                    "tags": ["Workflows"],
                    "parameters": [
                        {
                            "name": "name",
                            "in": "path",
                            "required": true,
                            "description": "Workflow name",
                            "schema": {
                                "type": "string"
                            }
                        }
                    ],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {
                                    "$ref": "#/components/schemas/ExecuteRequest"
                                }
                            }
                        }
                    },
                    "responses": {
                        "200": {
                            "description": "Execution result",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/ExecuteResponse"
                                    }
                                }
                            }
                        },
                        "400": {
                            "description": "Invalid request",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/ErrorResponse"
                                    }
                                }
                            }
                        },
                        "404": {
                            "description": "Workflow not found",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/ErrorResponse"
                                    }
                                }
                            }
                        },
                        "409": {
                            "description": "Idempotency key already used",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/ErrorResponse"
                                    }
                                }
                            }
                        },
                        "422": {
                            "description": "Invalid workflow definition",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/ErrorResponse"
                                    }
                                }
                            }
                        },
                        "429": {
                            "description": "Rate limit exceeded",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/ErrorResponse"
                                    }
                                }
                            }
                        },
                        "503": {
                            "description": "Service unavailable",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/ErrorResponse"
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/api/executions/{id}": {
                "get": {
                    "summary": "Get execution",
                    "description": "Get an execution by ID",
                    "operationId": "getExecution",
                    "tags": ["Executions"],
                    "parameters": [
                        {
                            "name": "id",
                            "in": "path",
                            "required": true,
                            "description": "Execution ID",
                            "schema": {
                                "type": "string",
                                "format": "uuid"
                            }
                        }
                    ],
                    "responses": {
                        "200": {
                            "description": "Execution details",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/Execution"
                                    }
                                }
                            }
                        },
                        "404": {
                            "description": "Execution not found",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/ErrorResponse"
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/api/executions/{id}/trace": {
                "get": {
                    "summary": "Get execution trace",
                    "description": "Get the execution trace with node details",
                    "operationId": "getExecutionTrace",
                    "tags": ["Executions"],
                    "parameters": [
                        {
                            "name": "id",
                            "in": "path",
                            "required": true,
                            "description": "Execution ID",
                            "schema": {
                                "type": "string",
                                "format": "uuid"
                            }
                        }
                    ],
                    "responses": {
                        "200": {
                            "description": "Execution trace",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/ExecutionTrace"
                                    }
                                }
                            }
                        },
                        "404": {
                            "description": "Execution not found",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "$ref": "#/components/schemas/ErrorResponse"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        "components": {
            "schemas": {
                "HealthResponse": {
                    "type": "object",
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["ok", "error"]
                        },
                        "foreign_keys_enabled": {
                            "type": "boolean"
                        },
                        "integrity_check": {
                            "type": "string"
                        },
                        "journal_mode": {
                            "type": "string"
                        }
                    },
                    "required": ["status"]
                },
                "ErrorResponse": {
                    "type": "object",
                    "properties": {
                        "error": {
                            "$ref": "#/components/schemas/ApiError"
                        }
                    },
                    "required": ["error"]
                },
                "ApiError": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "string" },
                        "message": { "type": "string" },
                        "category": {
                            "type": "string",
                            "enum": ["client_error", "transient", "permanent", "rate_limit", "conflict"]
                        },
                        "correlation_id": { "type": "string", "nullable": true },
                        "execution_id": { "type": "string", "nullable": true },
                        "request_id": { "type": "string", "nullable": true },
                        "retry_after_ms": { "type": "integer", "nullable": true },
                        "details": { "type": "object", "nullable": true, "additionalProperties": true }
                    },
                    "required": ["code", "message", "category"]
                },
                "WorkflowSummary": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Workflow ID"
                        },
                        "name": {
                            "type": "string",
                            "description": "Workflow name"
                        },
                        "enabled": {
                            "type": "boolean",
                            "description": "Whether the workflow is enabled"
                        },
                        "created_at": {
                            "type": "string",
                            "format": "date-time"
                        },
                        "updated_at": {
                            "type": "string",
                            "format": "date-time"
                        },
                        "node_count": {
                            "type": "integer",
                            "description": "Number of nodes in the workflow"
                        },
                        "trigger_count": {
                            "type": "integer",
                            "description": "Number of triggers"
                        }
                    },
                    "required": ["id", "name", "enabled", "created_at", "updated_at"]
                },
                "WorkflowDetail": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "enabled": { "type": "boolean" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" },
                        "node_count": { "type": "integer" },
                        "trigger_count": { "type": "integer" },
                        "definition": {
                            "type": "string",
                            "description": "YAML workflow definition"
                        }
                    },
                    "required": ["id", "name", "enabled", "definition"]
                },
                "ExecuteRequest": {
                    "type": "object",
                    "properties": {
                        "input": {
                            "type": "object",
                            "description": "Input data for the workflow",
                            "additionalProperties": true
                        },
                        "wait": {
                            "type": "boolean",
                            "default": true,
                            "description": "Wait for execution to complete"
                        },
                        "correlation_id": {
                            "type": "string",
                            "description": "End-to-end trace identifier"
                        },
                        "idempotency_key": {
                            "type": "string",
                            "description": "Key used to deduplicate repeated execution requests"
                        }
                    }
                },
                "ExecuteResponse": {
                    "type": "object",
                    "properties": {
                        "execution_id": {
                            "type": "string",
                            "format": "uuid"
                        },
                        "status": {
                            "type": "string",
                            "enum": ["running", "completed", "failed"]
                        },
                        "output": {
                            "type": "object",
                            "nullable": true,
                            "additionalProperties": true
                        },
                        "error": {
                            "type": "string",
                            "nullable": true
                        },
                        "duration_ms": {
                            "type": "integer",
                            "nullable": true
                        }
                    },
                    "required": ["execution_id", "status"]
                },
                "Execution": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "workflow_id": { "type": "string" },
                        "workflow_name": { "type": "string" },
                        "status": { "type": "string", "enum": ["running", "completed", "failed"] },
                        "trigger_type": { "type": "string" },
                        "input": { "type": "object", "additionalProperties": true },
                        "output": { "type": "object", "nullable": true, "additionalProperties": true },
                        "error": { "type": "string", "nullable": true },
                        "started_at": { "type": "string", "format": "date-time" },
                        "finished_at": { "type": "string", "format": "date-time", "nullable": true },
                        "duration_ms": { "type": "integer", "nullable": true },
                        "correlation_id": { "type": "string", "nullable": true },
                        "idempotency_key": { "type": "string", "nullable": true },
                        "origin": { "type": "string", "nullable": true }
                    },
                    "required": ["id", "workflow_id", "workflow_name", "status", "trigger_type", "started_at"]
                },
                "ExecutionTrace": {
                    "type": "object",
                    "properties": {
                        "execution_id": { "type": "string", "format": "uuid" },
                        "workflow_name": { "type": "string" },
                        "status": { "type": "string" },
                        "nodes": {
                            "type": "array",
                            "items": {
                                "$ref": "#/components/schemas/NodeExecution"
                            }
                        }
                    },
                    "required": ["execution_id", "workflow_name", "status", "nodes"]
                },
                "NodeExecution": {
                    "type": "object",
                    "properties": {
                        "node_id": { "type": "string" },
                        "status": { "type": "string", "enum": ["pending", "running", "completed", "failed", "skipped"] },
                        "started_at": { "type": "string", "format": "date-time" },
                        "finished_at": { "type": "string", "format": "date-time", "nullable": true },
                        "duration_ms": { "type": "integer", "nullable": true },
                        "error": { "type": "string", "nullable": true }
                    },
                    "required": ["node_id", "status", "started_at"]
                }
            }
        },
        "tags": [
            {
                "name": "System",
                "description": "System endpoints for health and metrics"
            },
            {
                "name": "Workflows",
                "description": "Workflow management and execution"
            },
            {
                "name": "Executions",
                "description": "Execution status and history"
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openapi_spec_is_valid_json() {
        let spec = generate_openapi_spec();
        assert!(spec.is_object());
        assert_eq!(spec["openapi"], "3.1.0");
        assert!(spec["paths"].is_object());
        assert!(spec["components"]["schemas"].is_object());
    }

    #[test]
    fn test_spec_has_required_paths() {
        let spec = generate_openapi_spec();
        let paths = spec["paths"].as_object().unwrap();

        assert!(paths.contains_key("/api/health"));
        assert!(paths.contains_key("/api/metrics"));
        assert!(paths.contains_key("/api/workflows"));
        assert!(paths.contains_key("/api/workflows/{name}"));
        assert!(paths.contains_key("/api/workflows/{name}/execute"));
        assert!(paths.contains_key("/api/executions/{id}"));
        assert!(paths.contains_key("/api/executions/{id}/trace"));
    }
}
