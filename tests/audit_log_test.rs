//! Integration tests for the audit log storage layer.
//!
//! These tests verify `save_audit_entry`, `list_audit_entries`, the full filter
//! surface, pagination, limit capping, ordering, and the `AuditEventType`
//! Display / FromStr round-trip — all against an in-memory SQLite database.

use chrono::{Duration, Utc};
use r8r::storage::{AuditEntry, AuditEventType, SqliteStorage};
use serde_json::json;

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Construct a minimal [`AuditEntry`] with sensible defaults.  Individual
/// tests override only the fields they care about.
fn make_entry(id: &str, event_type: AuditEventType) -> AuditEntry {
    AuditEntry {
        id: id.to_string(),
        timestamp: Utc::now(),
        event_type,
        execution_id: Some("exec-1".to_string()),
        workflow_name: Some("my-workflow".to_string()),
        node_id: Some("node-a".to_string()),
        actor: "system".to_string(),
        detail: "test detail".to_string(),
        metadata: None,
    }
}

/// Open a fresh in-memory database for each test so tests are fully isolated.
fn open_storage() -> SqliteStorage {
    SqliteStorage::open_in_memory().expect("in-memory storage should open")
}

// ─── Test 1: save and list round-trip ───────────────────────────────────────

#[tokio::test]
async fn test_save_and_list_audit_entry() {
    let storage = open_storage();

    let ts = Utc::now();
    let entry = AuditEntry {
        id: "entry-001".to_string(),
        timestamp: ts,
        event_type: AuditEventType::ExecutionStarted,
        execution_id: Some("exec-abc".to_string()),
        workflow_name: Some("wf-alpha".to_string()),
        node_id: Some("node-1".to_string()),
        actor: "user:alice".to_string(),
        detail: "workflow execution began".to_string(),
        metadata: None,
    };

    storage.save_audit_entry(&entry).await.unwrap();

    let results = storage
        .list_audit_entries(None, None, None, None, 10, 0)
        .await
        .unwrap();

    assert_eq!(results.len(), 1, "expected exactly one entry");
    let got = &results[0];

    assert_eq!(got.id, "entry-001");
    assert_eq!(got.event_type, AuditEventType::ExecutionStarted);
    assert_eq!(got.execution_id.as_deref(), Some("exec-abc"));
    assert_eq!(got.workflow_name.as_deref(), Some("wf-alpha"));
    assert_eq!(got.node_id.as_deref(), Some("node-1"));
    assert_eq!(got.actor, "user:alice");
    assert_eq!(got.detail, "workflow execution began");
    assert!(got.metadata.is_none());

    // Timestamps are stored as RFC 3339 text; verify sub-second precision is
    // preserved by checking that the round-tripped value is within 1 second.
    let delta = (got.timestamp - ts).num_milliseconds().abs();
    assert!(
        delta < 1000,
        "timestamp should round-trip within 1 second, delta was {delta}ms"
    );
}

// ─── Test 2: filter by execution_id ─────────────────────────────────────────

#[tokio::test]
async fn test_list_filter_by_execution_id() {
    let storage = open_storage();

    let mut e1 = make_entry("e1", AuditEventType::NodeStarted);
    e1.execution_id = Some("exec-A".to_string());

    let mut e2 = make_entry("e2", AuditEventType::NodeCompleted);
    e2.execution_id = Some("exec-B".to_string());

    let mut e3 = make_entry("e3", AuditEventType::NodeFailed);
    e3.execution_id = Some("exec-A".to_string());

    storage.save_audit_entry(&e1).await.unwrap();
    storage.save_audit_entry(&e2).await.unwrap();
    storage.save_audit_entry(&e3).await.unwrap();

    let results = storage
        .list_audit_entries(Some("exec-A"), None, None, None, 100, 0)
        .await
        .unwrap();

    assert_eq!(results.len(), 2, "should return only exec-A entries");
    for r in &results {
        assert_eq!(r.execution_id.as_deref(), Some("exec-A"));
    }

    let results_b = storage
        .list_audit_entries(Some("exec-B"), None, None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(results_b.len(), 1);
    assert_eq!(results_b[0].id, "e2");
}

// ─── Test 3: filter by workflow_name ────────────────────────────────────────

#[tokio::test]
async fn test_list_filter_by_workflow_name() {
    let storage = open_storage();

    let mut e1 = make_entry("w1", AuditEventType::WorkflowCreated);
    e1.workflow_name = Some("alpha-workflow".to_string());

    let mut e2 = make_entry("w2", AuditEventType::WorkflowUpdated);
    e2.workflow_name = Some("beta-workflow".to_string());

    let mut e3 = make_entry("w3", AuditEventType::WorkflowDeleted);
    e3.workflow_name = Some("alpha-workflow".to_string());

    storage.save_audit_entry(&e1).await.unwrap();
    storage.save_audit_entry(&e2).await.unwrap();
    storage.save_audit_entry(&e3).await.unwrap();

    let alpha = storage
        .list_audit_entries(None, Some("alpha-workflow"), None, None, 100, 0)
        .await
        .unwrap();

    assert_eq!(alpha.len(), 2, "should return only alpha-workflow entries");
    for r in &alpha {
        assert_eq!(r.workflow_name.as_deref(), Some("alpha-workflow"));
    }

    let beta = storage
        .list_audit_entries(None, Some("beta-workflow"), None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(beta.len(), 1);
    assert_eq!(beta[0].id, "w2");
}

// ─── Test 4: filter by event_type ───────────────────────────────────────────

#[tokio::test]
async fn test_list_filter_by_event_type() {
    let storage = open_storage();

    storage
        .save_audit_entry(&make_entry("et1", AuditEventType::ExecutionStarted))
        .await
        .unwrap();
    storage
        .save_audit_entry(&make_entry("et2", AuditEventType::ExecutionCompleted))
        .await
        .unwrap();
    storage
        .save_audit_entry(&make_entry("et3", AuditEventType::ExecutionStarted))
        .await
        .unwrap();
    storage
        .save_audit_entry(&make_entry("et4", AuditEventType::NodeFailed))
        .await
        .unwrap();

    let started = storage
        .list_audit_entries(None, None, Some("execution_started"), None, 100, 0)
        .await
        .unwrap();

    assert_eq!(
        started.len(),
        2,
        "should return 2 execution_started entries"
    );
    for r in &started {
        assert_eq!(r.event_type, AuditEventType::ExecutionStarted);
    }

    let completed = storage
        .list_audit_entries(None, None, Some("execution_completed"), None, 100, 0)
        .await
        .unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].id, "et2");
}

// ─── Test 5: filter by since ─────────────────────────────────────────────────

#[tokio::test]
async fn test_list_filter_by_since() {
    let storage = open_storage();

    let now = Utc::now();
    let one_hour_ago = now - Duration::hours(1);
    let two_hours_ago = now - Duration::hours(2);

    let mut old_entry = make_entry("old", AuditEventType::CheckpointSaved);
    old_entry.timestamp = two_hours_ago;

    let mut mid_entry = make_entry("mid", AuditEventType::CheckpointSaved);
    mid_entry.timestamp = one_hour_ago;

    let mut recent_entry = make_entry("recent", AuditEventType::CheckpointSaved);
    recent_entry.timestamp = now;

    storage.save_audit_entry(&old_entry).await.unwrap();
    storage.save_audit_entry(&mid_entry).await.unwrap();
    storage.save_audit_entry(&recent_entry).await.unwrap();

    // Filter: entries at or after 90 minutes ago — should include mid and recent,
    // but not old.
    let cutoff = now - Duration::minutes(90);
    let results = storage
        .list_audit_entries(None, None, None, Some(cutoff), 100, 0)
        .await
        .unwrap();

    assert_eq!(
        results.len(),
        2,
        "should return mid and recent entries, not old"
    );
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&"mid"), "mid entry should be included");
    assert!(ids.contains(&"recent"), "recent entry should be included");
    assert!(!ids.contains(&"old"), "old entry should be excluded");
}

// ─── Test 6: combined filters (AND semantics) ────────────────────────────────

#[tokio::test]
async fn test_list_combined_filters() {
    let storage = open_storage();

    // Entry that matches all filters
    let mut target = make_entry("target", AuditEventType::NodeCompleted);
    target.execution_id = Some("exec-X".to_string());
    target.workflow_name = Some("wf-X".to_string());

    // Matches execution_id and workflow_name but wrong event_type
    let mut wrong_type = make_entry("wrong-type", AuditEventType::NodeFailed);
    wrong_type.execution_id = Some("exec-X".to_string());
    wrong_type.workflow_name = Some("wf-X".to_string());

    // Matches event_type and workflow_name but wrong execution_id
    let mut wrong_exec = make_entry("wrong-exec", AuditEventType::NodeCompleted);
    wrong_exec.execution_id = Some("exec-Y".to_string());
    wrong_exec.workflow_name = Some("wf-X".to_string());

    // Matches execution_id and event_type but wrong workflow_name
    let mut wrong_wf = make_entry("wrong-wf", AuditEventType::NodeCompleted);
    wrong_wf.execution_id = Some("exec-X".to_string());
    wrong_wf.workflow_name = Some("wf-Z".to_string());

    for e in [&target, &wrong_type, &wrong_exec, &wrong_wf] {
        storage.save_audit_entry(e).await.unwrap();
    }

    let results = storage
        .list_audit_entries(
            Some("exec-X"),
            Some("wf-X"),
            Some("node_completed"),
            None,
            100,
            0,
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1, "only the target entry should match");
    assert_eq!(results[0].id, "target");
}

// ─── Test 7: pagination (limit and offset) ───────────────────────────────────

#[tokio::test]
async fn test_list_pagination() {
    let storage = open_storage();

    // Insert 10 entries.  Use deliberately spaced timestamps so ordering is
    // deterministic (newest first).
    let base = Utc::now();
    for i in 0..10u32 {
        let mut e = make_entry(&format!("page-{i}"), AuditEventType::ExecutionStarted);
        e.timestamp = base + Duration::seconds(i as i64);
        storage.save_audit_entry(&e).await.unwrap();
    }

    // Newest-first ordering: page-9 is index 0, page-0 is index 9.

    // First page: 3 items, offset 0 → should be page-9, page-8, page-7
    let page1 = storage
        .list_audit_entries(None, None, None, None, 3, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 3);
    assert_eq!(page1[0].id, "page-9");
    assert_eq!(page1[1].id, "page-8");
    assert_eq!(page1[2].id, "page-7");

    // Second page: 3 items, offset 3 → should be page-6, page-5, page-4
    let page2 = storage
        .list_audit_entries(None, None, None, None, 3, 3)
        .await
        .unwrap();
    assert_eq!(page2.len(), 3);
    assert_eq!(page2[0].id, "page-6");
    assert_eq!(page2[1].id, "page-5");
    assert_eq!(page2[2].id, "page-4");

    // Last partial page: 3 items requested, offset 9 → only 1 remains (page-0)
    let page4 = storage
        .list_audit_entries(None, None, None, None, 3, 9)
        .await
        .unwrap();
    assert_eq!(page4.len(), 1);
    assert_eq!(page4[0].id, "page-0");

    // Beyond the end
    let empty = storage
        .list_audit_entries(None, None, None, None, 10, 100)
        .await
        .unwrap();
    assert!(empty.is_empty(), "offset past end should return empty vec");
}

// ─── Test 8: limit is capped at MAX_AUDIT_LIMIT (1000) ──────────────────────

#[tokio::test]
async fn test_list_limit_capped() {
    let storage = open_storage();

    // Insert just a handful of entries — we only need to verify the query
    // succeeds and does not return more than what exists (not that it's
    // artificially capped at 1000 when fewer rows exist).
    for i in 0..5u32 {
        let e = make_entry(&format!("cap-{i}"), AuditEventType::DlqEntryCreated);
        storage.save_audit_entry(&e).await.unwrap();
    }

    // Request an absurdly large limit — the implementation caps it at 1000
    // internally.  The query must not error and must return all 5 rows.
    let results = storage
        .list_audit_entries(None, None, None, None, u32::MAX, 0)
        .await
        .unwrap();

    assert_eq!(
        results.len(),
        5,
        "all 5 entries should be returned when limit is capped to MAX_AUDIT_LIMIT"
    );
}

// ─── Test 9: results are ordered newest-first ────────────────────────────────

#[tokio::test]
async fn test_list_ordering() {
    let storage = open_storage();

    let base = Utc::now();

    // Insert entries deliberately out of chronological order to ensure the
    // ORDER BY clause, not insertion order, governs the result.
    let timestamps = [
        Duration::seconds(5),
        Duration::seconds(1),
        Duration::seconds(3),
        Duration::seconds(0),
        Duration::seconds(2),
    ];

    for (i, delta) in timestamps.iter().enumerate() {
        let mut e = make_entry(&format!("ord-{i}"), AuditEventType::NodeStarted);
        e.timestamp = base + *delta;
        storage.save_audit_entry(&e).await.unwrap();
    }

    let results = storage
        .list_audit_entries(None, None, None, None, 10, 0)
        .await
        .unwrap();

    assert_eq!(results.len(), 5);

    // Verify strict descending order of timestamps.
    for window in results.windows(2) {
        assert!(
            window[0].timestamp >= window[1].timestamp,
            "entries should be newest-first: {:?} >= {:?} violated",
            window[0].timestamp,
            window[1].timestamp,
        );
    }

    // The entry with the largest delta (5s) should be first.
    assert_eq!(
        results[0].id, "ord-0",
        "entry with +5s delta should be first"
    );
    // The entry with delta 0 should be last.
    assert_eq!(
        results[4].id, "ord-3",
        "entry with +0s delta should be last"
    );
}

// ─── Test 10: AuditEventType Display <-> FromStr round-trip ─────────────────

#[test]
fn test_audit_event_type_display_roundtrip() {
    let all_variants = [
        AuditEventType::ExecutionStarted,
        AuditEventType::ExecutionCompleted,
        AuditEventType::ExecutionFailed,
        AuditEventType::ExecutionPaused,
        AuditEventType::ExecutionResumed,
        AuditEventType::ExecutionCancelled,
        AuditEventType::NodeStarted,
        AuditEventType::NodeCompleted,
        AuditEventType::NodeFailed,
        AuditEventType::NodeSkipped,
        AuditEventType::ApprovalRequested,
        AuditEventType::ApprovalDecided,
        AuditEventType::ApprovalTimedOut,
        AuditEventType::WorkflowCreated,
        AuditEventType::WorkflowUpdated,
        AuditEventType::WorkflowDeleted,
        AuditEventType::DlqEntryCreated,
        AuditEventType::CheckpointSaved,
    ];

    for variant in &all_variants {
        let displayed = variant.to_string();
        assert!(
            !displayed.is_empty(),
            "{variant:?}.to_string() should not be empty"
        );

        let parsed: AuditEventType = displayed
            .parse()
            .unwrap_or_else(|_| panic!("FromStr should parse \"{displayed}\" back into a variant"));

        assert_eq!(
            &parsed, variant,
            "round-trip failed for {variant:?}: display=\"{displayed}\""
        );
    }

    // Confirm that an unknown string is rejected (Err variant).
    let bad: std::result::Result<AuditEventType, _> = "definitely_not_a_variant".parse();
    assert!(bad.is_err(), "unknown event type string should return Err");
}

// ─── Test 11: entry with JSON metadata persists and deserializes ─────────────

#[tokio::test]
async fn test_save_entry_with_metadata() {
    let storage = open_storage();

    let meta = json!({
        "retry_count": 3,
        "node": "send-email",
        "tags": ["critical", "pagerduty"],
        "nested": {
            "key": "value"
        }
    });

    let entry = AuditEntry {
        id: "meta-entry".to_string(),
        timestamp: Utc::now(),
        event_type: AuditEventType::NodeFailed,
        execution_id: Some("exec-meta".to_string()),
        workflow_name: Some("alert-wf".to_string()),
        node_id: Some("send-email".to_string()),
        actor: "engine".to_string(),
        detail: "node failed after 3 retries".to_string(),
        metadata: Some(meta.clone()),
    };

    storage.save_audit_entry(&entry).await.unwrap();

    let results = storage
        .list_audit_entries(None, None, None, None, 10, 0)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    let got = &results[0];

    let got_meta = got
        .metadata
        .as_ref()
        .expect("metadata should be present after round-trip");

    assert_eq!(got_meta["retry_count"], json!(3));
    assert_eq!(got_meta["node"], json!("send-email"));
    assert_eq!(got_meta["tags"], json!(["critical", "pagerduty"]));
    assert_eq!(got_meta["nested"]["key"], json!("value"));
}

// ─── Test 12: entry with all nullable fields set to None ─────────────────────

#[tokio::test]
async fn test_save_entry_with_null_optionals() {
    let storage = open_storage();

    let entry = AuditEntry {
        id: "null-optionals".to_string(),
        timestamp: Utc::now(),
        event_type: AuditEventType::WorkflowCreated,
        execution_id: None,
        workflow_name: None,
        node_id: None,
        actor: "admin".to_string(),
        detail: "workflow provisioned via API".to_string(),
        metadata: None,
    };

    storage
        .save_audit_entry(&entry)
        .await
        .expect("saving an entry with all-None optionals should succeed");

    let results = storage
        .list_audit_entries(None, None, None, None, 10, 0)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    let got = &results[0];

    assert_eq!(got.id, "null-optionals");
    assert!(got.execution_id.is_none(), "execution_id should be None");
    assert!(got.workflow_name.is_none(), "workflow_name should be None");
    assert!(got.node_id.is_none(), "node_id should be None");
    assert!(got.metadata.is_none(), "metadata should be None");
    assert_eq!(got.actor, "admin");
    assert_eq!(got.detail, "workflow provisioned via API");

    // A null-optional entry should not appear when filtering by execution_id,
    // because NULL != "anything".
    let filtered = storage
        .list_audit_entries(Some("exec-1"), None, None, None, 10, 0)
        .await
        .unwrap();
    assert!(
        filtered.is_empty(),
        "entry with NULL execution_id should not match an execution_id filter"
    );
}
