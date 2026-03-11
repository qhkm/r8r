// Tests for r8r CLI flag behavior and exit-code semantics.
// Run with: cargo test --test cli_flags_test

#[test]
fn exit_42_when_waiting_for_approval_non_interactive() {
    assert!(r8r::should_exit_42("waiting_for_approval", false));
}

#[test]
fn no_exit_42_when_interactive() {
    assert!(!r8r::should_exit_42("waiting_for_approval", true));
}

#[test]
fn no_exit_42_when_status_is_completed() {
    assert!(!r8r::should_exit_42("completed", false));
}

#[test]
fn no_exit_42_when_status_is_failed() {
    assert!(!r8r::should_exit_42("failed", false));
}
