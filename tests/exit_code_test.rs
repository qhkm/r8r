//! Integration tests for `r8r` CLI flags and exit codes.
//! Run all: `cargo test --test exit_code_test`
//! Run ignoring slow/server tests: `cargo test --test exit_code_test` (ignored are skipped by default)

use assert_cmd::Command;
use predicates::prelude::*;

// ── r8r prompt ────────────────────────────────────────────────────────────────

#[test]
fn prompt_requires_description() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.arg("prompt");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Please provide a description"));
}

#[test]
fn prompt_patch_requires_description() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--patch", "some-workflow"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Please describe the change"));
}

#[test]
fn prompt_emit_flag_is_recognised() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--emit"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Please provide a description"));
}

#[test]
fn prompt_dry_run_flag_is_recognised() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--dry-run"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Please provide a description"));
}

#[test]
fn prompt_json_flag_is_recognised() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--json"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Please provide a description"));
}

#[test]
fn prompt_yes_flag_is_recognised() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--yes"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Please provide a description"));
}

// ── flag output correctness (require LLM API key, skipped in CI) ─────────────

/// Verifies --emit writes valid YAML to stdout and exits 0.
/// Run manually: `cargo test --test exit_code_test prompt_emit_writes_yaml_to_stdout -- --ignored`
#[test]
#[ignore = "requires LLM API key"]
fn prompt_emit_writes_yaml_to_stdout() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--emit", "--yes", "fetch HN top stories"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("nodes:"));
}

/// Verifies --dry-run does not save anything to storage.
/// Run manually: `cargo test --test exit_code_test prompt_dry_run_saves_nothing -- --ignored`
#[test]
#[ignore = "requires LLM API key"]
fn prompt_dry_run_saves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--dry-run", "--yes", "fetch HN top stories"])
        .env("R8R_DATA_DIR", dir.path());
    cmd.assert().success();
    let db_count = std::fs::read_dir(dir.path())
        .unwrap()
        .filter(|e| e.as_ref().unwrap().path().extension().map_or(false, |x| x == "db"))
        .count();
    assert_eq!(db_count, 0, "dry-run should not write to storage");
}

// ── exit code 42 ─────────────────────────────────────────────────────────────

/// Full end-to-end exit-42 test. Requires a running r8r server with an approval workflow.
/// Run manually: `cargo test --test exit_code_test exit_42_end_to_end -- --ignored`
#[test]
#[ignore = "requires running r8r server with approval workflow"]
fn exit_42_end_to_end() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["run", "needs-approval"])
        .write_stdin("")
        .env("R8R_BASE_URL", "http://localhost:3000");
    cmd.assert().code(42);
}

// ── help ──────────────────────────────────────────────────────────────────────

#[test]
fn help_flag_exits_zero() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.arg("--help");
    cmd.assert().success();
}

#[test]
fn prompt_help_mentions_emit_flag() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("--emit"));
}
