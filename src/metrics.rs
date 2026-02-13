//! Prometheus metrics for r8r.
//!
//! This module provides application metrics exposed via the /metrics endpoint.
//!
//! ## Metrics
//!
//! ### Counters
//! - `r8r_workflows_executed_total` - Total workflow executions by status and trigger_type
//! - `r8r_nodes_executed_total` - Total node executions by node_type and status
//! - `r8r_http_requests_total` - Total HTTP node requests by method and status
//!
//! ### Histograms
//! - `r8r_workflow_duration_seconds` - Workflow execution duration
//! - `r8r_node_duration_seconds` - Node execution duration by node_type
//!
//! ### Gauges
//! - `r8r_active_executions` - Currently running workflow executions

use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;
use std::time::Duration;

/// Global Prometheus handle for rendering metrics.
static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Initialize the Prometheus metrics exporter.
///
/// This should be called once at application startup.
/// Returns the PrometheusHandle for rendering metrics.
pub fn init_metrics() -> PrometheusHandle {
    PROMETHEUS_HANDLE
        .get_or_init(|| {
            PrometheusBuilder::new()
                .install_recorder()
                .expect("Failed to install Prometheus recorder")
        })
        .clone()
}

/// Get the Prometheus handle for rendering metrics.
///
/// Returns None if metrics have not been initialized.
pub fn get_prometheus_handle() -> Option<&'static PrometheusHandle> {
    PROMETHEUS_HANDLE.get()
}

/// Render current metrics in Prometheus text format.
pub fn render_metrics() -> String {
    match get_prometheus_handle() {
        Some(handle) => handle.render(),
        None => "# Metrics not initialized\n".to_string(),
    }
}

// =============================================================================
// Workflow Metrics
// =============================================================================

/// Record a workflow execution.
pub fn record_workflow_execution(status: &str, trigger_type: &str) {
    counter!(
        "r8r_workflows_executed_total",
        "status" => status.to_string(),
        "trigger_type" => trigger_type.to_string()
    )
    .increment(1);
}

/// Record workflow execution duration.
pub fn record_workflow_duration(duration: Duration, workflow_name: &str) {
    histogram!(
        "r8r_workflow_duration_seconds",
        "workflow" => workflow_name.to_string()
    )
    .record(duration.as_secs_f64());
}

/// Increment active executions gauge.
pub fn inc_active_executions() {
    gauge!("r8r_active_executions").increment(1.0);
}

/// Decrement active executions gauge.
pub fn dec_active_executions() {
    gauge!("r8r_active_executions").decrement(1.0);
}

// =============================================================================
// Node Metrics
// =============================================================================

/// Record a node execution.
pub fn record_node_execution(node_type: &str, status: &str) {
    counter!(
        "r8r_nodes_executed_total",
        "node_type" => node_type.to_string(),
        "status" => status.to_string()
    )
    .increment(1);
}

/// Record node execution duration.
pub fn record_node_duration(duration: Duration, node_type: &str) {
    histogram!(
        "r8r_node_duration_seconds",
        "node_type" => node_type.to_string()
    )
    .record(duration.as_secs_f64());
}

// =============================================================================
// HTTP Node Metrics
// =============================================================================

/// Record an HTTP request made by the HTTP node.
pub fn record_http_request(method: &str, status_code: u16) {
    counter!(
        "r8r_http_requests_total",
        "method" => method.to_string(),
        "status" => status_code.to_string()
    )
    .increment(1);
}

/// Record HTTP request duration.
pub fn record_http_duration(duration: Duration, method: &str) {
    histogram!(
        "r8r_http_request_duration_seconds",
        "method" => method.to_string()
    )
    .record(duration.as_secs_f64());
}

// =============================================================================
// Storage Metrics
// =============================================================================

/// Record a database operation.
pub fn record_db_operation(operation: &str, success: bool) {
    counter!(
        "r8r_db_operations_total",
        "operation" => operation.to_string(),
        "success" => success.to_string()
    )
    .increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_not_initialized() {
        // Without initialization, render should return placeholder
        // Note: In tests, metrics might already be initialized by other tests
        let result = render_metrics();
        assert!(!result.is_empty());
    }
}
