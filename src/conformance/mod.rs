//! Conformance test harness for r8r API contracts.
//!
//! This module provides tools for validating r8r's compliance with the v1 API contract.
//! It can be used for:
//! - Cross-repo integration testing (ZeptoClaw → r8r)
//! - CI/CD pipeline validation
//! - Contract drift detection

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Conformance test result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceResult {
    pub test_name: String,
    pub passed: bool,
    pub category: TestCategory,
    pub details: Option<String>,
    pub duration_ms: u64,
}

/// Test categories for conformance validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestCategory {
    /// Schema validation (request/response formats)
    Schema,
    /// Idempotency behavior
    Idempotency,
    /// Error handling and codes
    ErrorHandling,
    /// Correlation ID propagation
    Tracing,
    /// Timeout and retry behavior
    Resilience,
    /// Performance benchmarks
    Performance,
}

/// API contract compliance validator.
pub struct ConformanceHarness {
    #[allow(dead_code)]
    base_url: String,
    results: Vec<ConformanceResult>,
}

impl ConformanceHarness {
    /// Create a new conformance test harness.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            results: Vec::new(),
        }
    }

    /// Get all test results.
    pub fn results(&self) -> &[ConformanceResult] {
        &self.results
    }

    /// Check if all tests passed.
    pub fn all_passed(&self) -> bool {
        self.results.iter().all(|r| r.passed)
    }

    /// Get summary statistics.
    pub fn summary(&self) -> ConformanceSummary {
        let total = self.results.len();
        let passed = self.results.iter().filter(|r| r.passed).count();
        let failed = total - passed;

        let by_category: HashMap<String, (usize, usize)> =
            self.results.iter().fold(HashMap::new(), |mut acc, r| {
                let cat = format!("{:?}", r.category).to_lowercase();
                let entry = acc.entry(cat).or_insert((0, 0));
                if r.passed {
                    entry.0 += 1;
                } else {
                    entry.1 += 1;
                }
                acc
            });

        ConformanceSummary {
            total,
            passed,
            failed,
            by_category,
        }
    }

    /// Record a test result.
    #[allow(dead_code)]
    fn record(&mut self, result: ConformanceResult) {
        self.results.push(result);
    }
}

/// Conformance test summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub by_category: HashMap<String, (usize, usize)>,
}

/// Schema validation for API contracts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaValidation {
    pub endpoint: String,
    pub method: String,
    pub request_schema: Option<serde_json::Value>,
    pub response_schema: Option<serde_json::Value>,
}

/// Idempotency test cases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdempotencyTest {
    pub name: String,
    pub workflow_name: String,
    pub idempotency_key: String,
    pub expected_behavior: IdempotencyBehavior,
}

/// Expected idempotency behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdempotencyBehavior {
    /// Same execution ID returned for duplicate requests
    ReturnCached,
    /// Conflict error with existing execution ID
    ConflictError,
    /// Different key creates a new execution
    DistinctExecution,
}

/// Correlation propagation test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationTest {
    pub name: String,
    pub correlation_id: String,
    pub workflow_name: String,
    pub expected_in_response: bool,
    pub expected_in_logs: bool,
}

/// Error scenario test case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorScenario {
    pub name: String,
    pub description: String,
    pub trigger: ErrorTrigger,
    pub expected_code: String,
    pub expected_category: String,
    pub retryable: bool,
}

/// Ways to trigger errors for testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorTrigger {
    /// Invalid workflow name
    InvalidWorkflowName(String),
    /// Missing required field
    MissingField(String),
    /// Duplicate idempotency key
    DuplicateIdempotencyKey { workflow: String, key: String },
    /// Invalid input schema
    InvalidInput(serde_json::Value),
    /// Request while server shutting down
    ShutdownRequest,
}

/// Standard conformance test suite for r8r v1 API.
pub struct StandardTestSuite;

impl StandardTestSuite {
    /// Get all idempotency tests.
    pub fn idempotency_tests() -> Vec<IdempotencyTest> {
        vec![
            IdempotencyTest {
                name: "duplicate_request_returns_cached".to_string(),
                workflow_name: "test-workflow".to_string(),
                idempotency_key: "test-key-001".to_string(),
                expected_behavior: IdempotencyBehavior::ConflictError,
            },
            IdempotencyTest {
                name: "different_keys_different_executions".to_string(),
                workflow_name: "test-workflow".to_string(),
                idempotency_key: "test-key-002".to_string(),
                expected_behavior: IdempotencyBehavior::DistinctExecution,
            },
        ]
    }

    /// Get all error scenario tests.
    pub fn error_tests() -> Vec<ErrorScenario> {
        vec![
            ErrorScenario {
                name: "workflow_not_found".to_string(),
                description: "Request execution of non-existent workflow".to_string(),
                trigger: ErrorTrigger::InvalidWorkflowName("non-existent-workflow".to_string()),
                expected_code: "WORKFLOW_NOT_FOUND".to_string(),
                expected_category: "client_error".to_string(),
                retryable: false,
            },
            ErrorScenario {
                name: "idempotency_key_reuse".to_string(),
                description: "Duplicate request with same idempotency key".to_string(),
                trigger: ErrorTrigger::DuplicateIdempotencyKey {
                    workflow: "test-workflow".to_string(),
                    key: "duplicate-key".to_string(),
                },
                expected_code: "IDEMPOTENCY_KEY_REUSE".to_string(),
                expected_category: "conflict".to_string(),
                retryable: false,
            },
            ErrorScenario {
                name: "server_shutting_down".to_string(),
                description: "Request during graceful shutdown".to_string(),
                trigger: ErrorTrigger::ShutdownRequest,
                expected_code: "SERVICE_UNAVAILABLE".to_string(),
                expected_category: "transient".to_string(),
                retryable: true,
            },
        ]
    }

    /// Get all correlation propagation tests.
    pub fn correlation_tests() -> Vec<CorrelationTest> {
        vec![
            CorrelationTest {
                name: "correlation_id_propagated_to_execution".to_string(),
                correlation_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
                workflow_name: "test-workflow".to_string(),
                expected_in_response: true,
                expected_in_logs: true,
            },
            CorrelationTest {
                name: "auto_generated_correlation_id".to_string(),
                correlation_id: "".to_string(),
                workflow_name: "test-workflow".to_string(),
                expected_in_response: true,
                expected_in_logs: true,
            },
        ]
    }
}

/// Mock client for testing without network calls.
#[cfg(test)]
pub mod mock {
    use super::*;
    use crate::error::{ApiErrorCode, ApiErrorEnvelope, ErrorCategory};
    use crate::storage::{Execution, ExecutionStatus};

    /// Mock r8r client for conformance testing.
    pub struct MockR8rClient {
        executions: HashMap<String, Execution>,
        idempotency_keys: HashMap<String, String>, // key -> execution_id
    }

    impl Default for MockR8rClient {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockR8rClient {
        pub fn new() -> Self {
            Self {
                executions: HashMap::new(),
                idempotency_keys: HashMap::new(),
            }
        }

        /// Simulate workflow execution.
        pub fn execute_workflow(
            &mut self,
            workflow_name: &str,
            input: serde_json::Value,
            correlation_id: Option<String>,
            idempotency_key: Option<String>,
        ) -> Result<Execution, Box<ApiErrorEnvelope>> {
            // Check idempotency
            if let Some(key) = &idempotency_key {
                if let Some(exec_id) = self.idempotency_keys.get(key) {
                    return Err(Box::new(ApiErrorEnvelope::idempotency_conflict(exec_id)));
                }
            }

            // Create execution
            let execution = Execution {
                id: uuid::Uuid::new_v4().to_string(),
                workflow_id: format!("wf-{}", workflow_name),
                workflow_name: workflow_name.to_string(),
                workflow_version: Some(1),
                status: ExecutionStatus::Completed,
                trigger_type: "api".to_string(),
                input,
                output: Some(serde_json::json!({"success": true})),
                started_at: chrono::Utc::now(),
                finished_at: Some(chrono::Utc::now()),
                error: None,
                correlation_id,
                idempotency_key: idempotency_key.clone(),
                origin: Some("api".to_string()),
            };

            // Store idempotency key
            if let Some(key) = idempotency_key {
                self.idempotency_keys.insert(key, execution.id.clone());
            }

            self.executions
                .insert(execution.id.clone(), execution.clone());
            Ok(execution)
        }

        /// Get execution by ID.
        pub fn get_execution(&self, id: &str) -> Option<&Execution> {
            self.executions.get(id)
        }
    }

    #[test]
    fn test_idempotency_conflict() {
        let mut client = MockR8rClient::new();
        let key = "test-key-123";

        // First request succeeds
        let result1 = client.execute_workflow(
            "test-workflow",
            serde_json::json!({}),
            None,
            Some(key.to_string()),
        );
        assert!(result1.is_ok());
        let exec1 = result1.unwrap();

        // Second request with same key returns conflict
        let result2 = client.execute_workflow(
            "test-workflow",
            serde_json::json!({}),
            None,
            Some(key.to_string()),
        );
        assert!(result2.is_err());

        let error = result2.unwrap_err();
        assert_eq!(error.code, ApiErrorCode::IdempotencyKeyReuse);
        assert_eq!(error.category, ErrorCategory::Conflict);
        assert_eq!(error.execution_id, Some(exec1.id));
    }

    #[test]
    fn test_mock_client_default() {
        let client = MockR8rClient::default();
        assert!(client.get_execution("nonexistent").is_none());
    }

    #[test]
    fn test_correlation_id_propagation() {
        let mut client = MockR8rClient::new();
        let correlation_id = "550e8400-e29b-41d4-a716-446655440000";

        let result = client.execute_workflow(
            "test-workflow",
            serde_json::json!({}),
            Some(correlation_id.to_string()),
            None,
        );

        assert!(result.is_ok());
        let execution = result.unwrap();
        assert_eq!(execution.correlation_id, Some(correlation_id.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conformance_summary() {
        let mut harness = ConformanceHarness::new("http://localhost:8080");

        harness.record(ConformanceResult {
            test_name: "test1".to_string(),
            passed: true,
            category: TestCategory::Schema,
            details: None,
            duration_ms: 10,
        });

        harness.record(ConformanceResult {
            test_name: "test2".to_string(),
            passed: false,
            category: TestCategory::Idempotency,
            details: Some("Conflict expected".to_string()),
            duration_ms: 20,
        });

        let summary = harness.summary();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert!(!harness.all_passed());
    }

    #[test]
    fn test_standard_suite_structure() {
        let idempotency = StandardTestSuite::idempotency_tests();
        assert!(!idempotency.is_empty());

        let errors = StandardTestSuite::error_tests();
        assert!(!errors.is_empty());

        let correlation = StandardTestSuite::correlation_tests();
        assert!(!correlation.is_empty());
    }
}
