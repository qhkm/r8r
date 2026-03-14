/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Tests for bridge event types and envelope serialization.

use r8r::bridge::events::{Ack, BridgeEvent, BridgeEventEnvelope};
use serde_json::json;

#[test]
fn test_approval_requested_serializes_correctly() {
    let event = BridgeEvent::ApprovalRequested {
        approval_id: "apr_001".to_string(),
        workflow: "deploy-prod".to_string(),
        execution_id: "exec_abc".to_string(),
        node_id: "approve-deploy".to_string(),
        message: "Deploy to production?".to_string(),
        timeout_secs: 3600,
        requester: Some("agent-1".to_string()),
        context: json!({"environment": "production"}),
    };

    let envelope = BridgeEventEnvelope::new(event, None);

    // Verify envelope structure
    assert!(envelope.id.starts_with("evt_"));
    assert_eq!(envelope.event_type, "r8r.approval.requested");
    assert!(envelope.correlation_id.is_none());

    // Serialize to JSON and verify structure
    let json_str = serde_json::to_string(&envelope).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    assert_eq!(parsed["type"], "r8r.approval.requested");
    assert!(parsed["id"].as_str().unwrap().starts_with("evt_"));
    assert!(parsed["timestamp"].as_str().is_some());

    // Verify data fields
    let data = &parsed["data"];
    assert_eq!(data["approval_id"], "apr_001");
    assert_eq!(data["workflow"], "deploy-prod");
    assert_eq!(data["execution_id"], "exec_abc");
    assert_eq!(data["node_id"], "approve-deploy");
    assert_eq!(data["message"], "Deploy to production?");
    assert_eq!(data["timeout_secs"], 3600);
    assert_eq!(data["requester"], "agent-1");
    assert_eq!(data["context"]["environment"], "production");
}

#[test]
fn test_approval_decision_deserializes_correctly() {
    let json_str = r#"{
        "id": "evt_12345",
        "type": "zeptoclaw.approval.decision",
        "timestamp": "2026-03-14T10:30:00Z",
        "data": {
            "approval_id": "apr_001",
            "execution_id": "exec_abc",
            "node_id": "approve-deploy",
            "decision": "approved",
            "reason": "Looks good",
            "decided_by": "admin@example.com",
            "channel": "slack"
        },
        "correlation_id": "corr_xyz"
    }"#;

    let envelope: BridgeEventEnvelope = serde_json::from_str(json_str).unwrap();

    assert_eq!(envelope.id, "evt_12345");
    assert_eq!(envelope.event_type, "zeptoclaw.approval.decision");
    assert_eq!(envelope.correlation_id.as_deref(), Some("corr_xyz"));

    // Deserialize the event from the envelope
    let event = BridgeEvent::from_type_and_data(&envelope.event_type, &envelope.data).unwrap();

    match event {
        BridgeEvent::ApprovalDecision {
            approval_id,
            execution_id,
            node_id,
            decision,
            reason,
            decided_by,
            channel,
        } => {
            assert_eq!(approval_id, "apr_001");
            assert_eq!(execution_id, "exec_abc");
            assert_eq!(node_id, "approve-deploy");
            assert_eq!(decision, "approved");
            assert_eq!(reason, "Looks good");
            assert_eq!(decided_by, "admin@example.com");
            assert_eq!(channel, "slack");
        }
        _ => panic!("Expected ApprovalDecision variant"),
    }
}

#[test]
fn test_ack_serializes_correctly() {
    let ack = Ack {
        event_id: "evt_abc123".to_string(),
    };

    let json_str = serde_json::to_string(&ack).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    assert_eq!(parsed["type"], "ack");
    assert_eq!(parsed["event_id"], "evt_abc123");
}

#[test]
fn test_all_event_types_roundtrip() {
    let events: Vec<(BridgeEvent, &str)> = vec![
        (
            BridgeEvent::ApprovalRequested {
                approval_id: "apr_1".to_string(),
                workflow: "w1".to_string(),
                execution_id: "e1".to_string(),
                node_id: "n1".to_string(),
                message: "Approve?".to_string(),
                timeout_secs: 60,
                requester: None,
                context: json!({}),
            },
            "r8r.approval.requested",
        ),
        (
            BridgeEvent::ApprovalTimeout {
                workflow: "w1".to_string(),
                execution_id: "e1".to_string(),
                node_id: "n1".to_string(),
                elapsed_secs: 120,
            },
            "r8r.approval.timeout",
        ),
        (
            BridgeEvent::ExecutionCompleted {
                workflow: "w1".to_string(),
                execution_id: "e1".to_string(),
                status: "completed".to_string(),
                duration_ms: 500,
                node_count: 3,
            },
            "r8r.execution.completed",
        ),
        (
            BridgeEvent::ExecutionFailed {
                workflow: "w1".to_string(),
                execution_id: "e1".to_string(),
                status: "failed".to_string(),
                error_code: "NODE_ERROR".to_string(),
                error_message: "HTTP timeout".to_string(),
                failed_node: "fetch-data".to_string(),
                duration_ms: 30000,
            },
            "r8r.execution.failed",
        ),
        (
            BridgeEvent::HealthStatus {
                version: "0.4.0".to_string(),
                uptime_secs: 86400,
                active_executions: 2,
                pending_approvals: 1,
                workflows_loaded: 5,
            },
            "r8r.health.status",
        ),
        (
            BridgeEvent::ApprovalDecision {
                approval_id: "apr_1".to_string(),
                execution_id: "e1".to_string(),
                node_id: "n1".to_string(),
                decision: "rejected".to_string(),
                reason: "Not ready".to_string(),
                decided_by: "user@co.com".to_string(),
                channel: "web".to_string(),
            },
            "zeptoclaw.approval.decision",
        ),
        (
            BridgeEvent::WorkflowTrigger {
                workflow: "nightly-report".to_string(),
                params: json!({"date": "2026-03-14"}),
                triggered_by: "scheduler".to_string(),
                channel: "api".to_string(),
            },
            "zeptoclaw.workflow.trigger",
        ),
        (BridgeEvent::HealthPing, "zeptoclaw.health.ping"),
    ];

    for (event, expected_type) in events {
        // Create envelope
        let envelope = BridgeEventEnvelope::new(event, None);
        assert_eq!(
            envelope.event_type, expected_type,
            "Type mismatch for {}",
            expected_type
        );

        // Serialize to JSON
        let json_str = serde_json::to_string(&envelope).unwrap();

        // Deserialize envelope back
        let parsed_envelope: BridgeEventEnvelope = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed_envelope.event_type, expected_type);

        // Reconstruct the event from type + data
        let reconstructed =
            BridgeEvent::from_type_and_data(&parsed_envelope.event_type, &parsed_envelope.data)
                .unwrap_or_else(|e| {
                    panic!("Failed to reconstruct {} from data: {}", expected_type, e)
                });

        // Verify the type string matches
        assert_eq!(
            reconstructed.event_type_str(),
            expected_type,
            "Roundtrip type mismatch for {}",
            expected_type
        );
    }
}
