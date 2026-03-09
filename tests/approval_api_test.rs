//! Integration tests for the approval request storage layer.
//!
//! These tests verify `save_approval_request`, `get_approval_request`,
//! `list_approval_requests`, and `decide_approval_request` — all against an
//! in-memory SQLite database, following the same pattern as `audit_log_test.rs`.
//!
//! Because `approval_requests` has a FK → `executions(id)` → `workflows(id)`,
//! every test that saves an approval must first create a StoredWorkflow and
//! Execution.  The `setup_workflow_and_execution` helper does this once and
//! returns the execution ID that approvals should reference.

use chrono::Utc;
use r8r::storage::{ApprovalRequest, Execution, ExecutionStatus, SqliteStorage, StoredWorkflow};

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Open a fresh in-memory database for each test so tests are fully isolated.
fn open_storage() -> SqliteStorage {
    SqliteStorage::open_in_memory().expect("in-memory storage should open")
}

/// Save the minimum required workflow + execution so that approval FK
/// constraints are satisfied.  Returns the execution ID to embed in approvals.
async fn setup_workflow_and_execution(storage: &SqliteStorage) -> String {
    let now = Utc::now();

    let workflow = StoredWorkflow {
        id: "wf-test".to_string(),
        name: "test-workflow".to_string(),
        definition:
            "name: test-workflow\nnodes:\n  - id: gate\n    type: approval\n    config:\n      title: gate"
                .to_string(),
        enabled: true,
        created_at: now,
        updated_at: now,
    };
    storage.save_workflow(&workflow).await.unwrap();

    let execution = Execution {
        id: "exec-1".to_string(),
        workflow_id: "wf-test".to_string(),
        workflow_name: "test-workflow".to_string(),
        workflow_version: None,
        status: ExecutionStatus::Paused,
        trigger_type: "manual".to_string(),
        input: serde_json::json!({}),
        output: None,
        started_at: now,
        finished_at: None,
        error: None,
        correlation_id: None,
        idempotency_key: None,
        origin: None,
    };
    storage.save_execution(&execution).await.unwrap();

    execution.id
}

/// Construct a minimal [`ApprovalRequest`] referencing the given execution.
/// Individual tests override only the fields they care about.
fn make_approval(id: &str, status: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: id.to_string(),
        execution_id: "exec-1".to_string(),
        node_id: "gate".to_string(),
        workflow_name: "test-workflow".to_string(),
        title: format!("Approve {id}"),
        description: None,
        status: status.to_string(),
        decision_comment: None,
        decided_by: None,
        context_data: None,
        created_at: Utc::now(),
        expires_at: None,
        decided_at: None,
        assignee: None,
        groups: vec![],
    }
}

// ─── Test 1: list by status ──────────────────────────────────────────────────

#[tokio::test]
async fn test_list_approvals_by_status() {
    let storage = open_storage();
    setup_workflow_and_execution(&storage).await;

    // 2 pending, 1 approved
    storage
        .save_approval_request(&make_approval("a1", "pending"))
        .await
        .unwrap();
    storage
        .save_approval_request(&make_approval("a2", "pending"))
        .await
        .unwrap();
    storage
        .save_approval_request(&make_approval("a3", "approved"))
        .await
        .unwrap();

    // Filter: pending → should return exactly 2
    let pending = storage
        .list_approval_requests(Some("pending"), 100, 0)
        .await
        .unwrap();
    assert_eq!(pending.len(), 2, "expected 2 pending approvals");
    for r in &pending {
        assert_eq!(r.status, "pending");
    }

    // Filter: approved → should return exactly 1
    let approved = storage
        .list_approval_requests(Some("approved"), 100, 0)
        .await
        .unwrap();
    assert_eq!(approved.len(), 1, "expected 1 approved approval");
    assert_eq!(approved[0].id, "a3");

    // No filter → should return all 3
    let all = storage
        .list_approval_requests(None, 100, 0)
        .await
        .unwrap();
    assert_eq!(all.len(), 3, "expected all 3 approvals with no status filter");
}

// ─── Test 2: pagination ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_approvals_pagination() {
    let storage = open_storage();
    setup_workflow_and_execution(&storage).await;

    // Insert 5 pending approvals with distinct timestamps so ORDER BY is stable
    for i in 0..5u32 {
        let mut approval = make_approval(&format!("p{i}"), "pending");
        // Spread timestamps so newest-first ordering is deterministic
        approval.created_at = Utc::now()
            + chrono::Duration::seconds(i as i64);
        storage.save_approval_request(&approval).await.unwrap();
    }

    // Page 1: limit=2, offset=0 → 2 items
    let page1 = storage
        .list_approval_requests(Some("pending"), 2, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2, "page 1 should return 2 items");

    // Page 2: limit=2, offset=2 → 2 items
    let page2 = storage
        .list_approval_requests(Some("pending"), 2, 2)
        .await
        .unwrap();
    assert_eq!(page2.len(), 2, "page 2 should return 2 items");

    // Page 3: limit=2, offset=4 → 1 item (last one)
    let page3 = storage
        .list_approval_requests(Some("pending"), 2, 4)
        .await
        .unwrap();
    assert_eq!(page3.len(), 1, "page 3 should return 1 remaining item");

    // All IDs across pages should be distinct
    let mut all_ids: Vec<String> = page1
        .iter()
        .chain(page2.iter())
        .chain(page3.iter())
        .map(|r| r.id.clone())
        .collect();
    all_ids.sort();
    all_ids.dedup();
    assert_eq!(all_ids.len(), 5, "all 5 distinct items should appear across pages");
}

// ─── Test 3: get found ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_approval_found() {
    let storage = open_storage();
    setup_workflow_and_execution(&storage).await;

    let approval = ApprovalRequest {
        id: "get-me".to_string(),
        execution_id: "exec-1".to_string(),
        node_id: "gate".to_string(),
        workflow_name: "test-workflow".to_string(),
        title: "Please approve this".to_string(),
        description: Some("Detailed description".to_string()),
        status: "pending".to_string(),
        decision_comment: None,
        decided_by: None,
        context_data: Some(serde_json::json!({"order_id": "X99"})),
        created_at: Utc::now(),
        expires_at: None,
        decided_at: None,
        assignee: None,
        groups: vec![],
    };

    storage.save_approval_request(&approval).await.unwrap();

    let found = storage
        .get_approval_request("get-me")
        .await
        .unwrap()
        .expect("approval should be found by ID");

    assert_eq!(found.id, "get-me");
    assert_eq!(found.execution_id, "exec-1");
    assert_eq!(found.node_id, "gate");
    assert_eq!(found.workflow_name, "test-workflow");
    assert_eq!(found.title, "Please approve this");
    assert_eq!(found.description.as_deref(), Some("Detailed description"));
    assert_eq!(found.status, "pending");
    assert!(found.decision_comment.is_none());
    assert!(found.decided_by.is_none());
    assert!(found.decided_at.is_none());
    assert!(found.expires_at.is_none());

    let ctx = found
        .context_data
        .as_ref()
        .expect("context_data should be present");
    assert_eq!(ctx["order_id"], serde_json::json!("X99"));
}

// ─── Test 4: get not found ───────────────────────────────────────────────────

#[tokio::test]
async fn test_get_approval_not_found() {
    let storage = open_storage();

    let result = storage
        .get_approval_request("does-not-exist")
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "non-existent ID should return Ok(None)"
    );
}

// ─── Test 5: decide approval success ────────────────────────────────────────

#[tokio::test]
async fn test_decide_approval_success() {
    let storage = open_storage();
    setup_workflow_and_execution(&storage).await;

    storage
        .save_approval_request(&make_approval("decide-me", "pending"))
        .await
        .unwrap();

    let decided_at = Utc::now();
    let updated = storage
        .decide_approval_request(
            "decide-me",
            "approved",
            "alice",
            Some("Looks good"),
            decided_at,
            "gate",
        )
        .await
        .unwrap();

    assert!(updated, "decide should return true when a pending row is updated");

    let got = storage
        .get_approval_request("decide-me")
        .await
        .unwrap()
        .expect("approval should still exist after deciding");

    assert_eq!(got.status, "approved");
    assert_eq!(got.decided_by.as_deref(), Some("alice"));
    assert_eq!(got.decision_comment.as_deref(), Some("Looks good"));
    assert!(
        got.decided_at.is_some(),
        "decided_at should be set after decision"
    );
}

// ─── Test 6: decide already-decided (atomic guard) ──────────────────────────

#[tokio::test]
async fn test_decide_approval_already_decided() {
    let storage = open_storage();
    setup_workflow_and_execution(&storage).await;

    storage
        .save_approval_request(&make_approval("double-decide", "pending"))
        .await
        .unwrap();

    // First decision — should succeed
    let first = storage
        .decide_approval_request(
            "double-decide",
            "approved",
            "alice",
            None,
            Utc::now(),
            "gate",
        )
        .await
        .unwrap();
    assert!(first, "first decide should return true");

    // Second decision on the same row — should be a no-op (already approved,
    // not pending) and return false
    let second = storage
        .decide_approval_request(
            "double-decide",
            "rejected",
            "bob",
            Some("Changed my mind"),
            Utc::now(),
            "gate",
        )
        .await
        .unwrap();
    assert!(!second, "second decide on already-decided approval should return false");

    // The stored record must reflect the first decision, not the second
    let got = storage
        .get_approval_request("double-decide")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.status, "approved", "status should not change after double-decide");
    assert_eq!(
        got.decided_by.as_deref(),
        Some("alice"),
        "decided_by should remain alice after double-decide"
    );
}

// ─── Test 7: decide not found ────────────────────────────────────────────────

#[tokio::test]
async fn test_decide_approval_not_found() {
    let storage = open_storage();

    let result = storage
        .decide_approval_request(
            "ghost-id",
            "approved",
            "nobody",
            None,
            Utc::now(),
            "gate",
        )
        .await
        .unwrap();

    assert!(
        !result,
        "deciding a non-existent approval should return false (0 rows affected)"
    );
}

// ─── Test 8: decide with comment ────────────────────────────────────────────

#[tokio::test]
async fn test_decide_approval_with_comment() {
    let storage = open_storage();
    setup_workflow_and_execution(&storage).await;

    storage
        .save_approval_request(&make_approval("with-comment", "pending"))
        .await
        .unwrap();

    let comment = "Approved after reviewing the audit trail in detail.";
    storage
        .decide_approval_request(
            "with-comment",
            "approved",
            "reviewer",
            Some(comment),
            Utc::now(),
            "gate",
        )
        .await
        .unwrap();

    let got = storage
        .get_approval_request("with-comment")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        got.decision_comment.as_deref(),
        Some(comment),
        "decision_comment should persist verbatim"
    );
}

// ─── Test 9: decide with rejected status ────────────────────────────────────

#[tokio::test]
async fn test_decide_approval_reject() {
    let storage = open_storage();
    setup_workflow_and_execution(&storage).await;

    storage
        .save_approval_request(&make_approval("reject-me", "pending"))
        .await
        .unwrap();

    let updated = storage
        .decide_approval_request(
            "reject-me",
            "rejected",
            "manager",
            Some("Policy violation"),
            Utc::now(),
            "gate",
        )
        .await
        .unwrap();

    assert!(updated, "reject should return true when a pending row is updated");

    let got = storage
        .get_approval_request("reject-me")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(got.status, "rejected", "status should be rejected");
    assert_eq!(got.decided_by.as_deref(), Some("manager"));
    assert_eq!(
        got.decision_comment.as_deref(),
        Some("Policy violation")
    );
    assert!(got.decided_at.is_some(), "decided_at should be set on rejection");
}
