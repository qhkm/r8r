/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! CLI integration tests for the job queue subcommands:
//! `r8r enqueue` and `r8r worker [--once]`.
//!
//! NOTE: MemoryQueue is per-process, so a full enqueue-then-worker roundtrip
//! across two CLI invocations cannot be tested here — each `Command::cargo_bin`
//! spawns a fresh process. End-to-end execution is covered at the unit level
//! (`src/queue/worker.rs` tests) and needs Redis for cross-process smoke tests.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

fn configure_isolated_env(cmd: &mut Command, root: &TempDir) {
    let config_home = root.path().join("config");
    let data_home = root.path().join("data");
    std::fs::create_dir_all(&config_home).unwrap();
    std::fs::create_dir_all(&data_home).unwrap();
    cmd.env("HOME", root.path())
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_DATA_HOME", data_home)
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("R8R_LLM_API_KEY")
        .env_remove("R8R_LLM_PROVIDER")
        .env_remove("R8R_LLM_MODEL")
        .env_remove("R8R_LLM_ENDPOINT")
        .env_remove("R8R_LLM_TIMEOUT_SECONDS")
        .env_remove("R8R_GENERATOR_API_KEY")
        .env_remove("R8R_GENERATOR_PROVIDER")
        .env_remove("R8R_GENERATOR_MODEL")
        .env_remove("R8R_GENERATOR_ENDPOINT")
        .env_remove("R8R_AGENT_API_KEY")
        .env_remove("R8R_AGENT_PROVIDER")
        .env_remove("R8R_AGENT_MODEL")
        .env_remove("R8R_AGENT_ENDPOINT")
        .env_remove("R8R_DATABASE_PATH")
        .env_remove("DATABASE_URL")
        // Never leak a host Redis into CLI tests.
        .env_remove("REDIS_URL")
        .env_remove("R8R_REDIS_URL");
}

fn write_workflow(path: &Path) {
    std::fs::write(
        path,
        r#"name: queue-test-workflow
nodes:
  - id: step1
    type: transform
    config:
      expression: '"ok"'
"#,
    )
    .unwrap();
}

/// Create the test workflow in the isolated env so `enqueue` can find it.
fn create_workflow(root: &TempDir) {
    let workflow_path = root.path().join("workflow.yaml");
    write_workflow(&workflow_path);

    let mut cmd = Command::cargo_bin("r8r").unwrap();
    configure_isolated_env(&mut cmd, root);
    cmd.arg("workflows").arg("create").arg(&workflow_path);
    let output = cmd.output().expect("failed to execute workflows create");
    assert!(
        output.status.success(),
        "workflows create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_queue_subcommands_in_help() {
    Command::cargo_bin("r8r")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("enqueue"))
        .stdout(predicate::str::contains("worker"));
}

#[test]
fn test_enqueue_missing_workflow_fails() {
    let root = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    configure_isolated_env(&mut cmd, &root);
    let output = cmd
        .arg("enqueue")
        .arg("does-not-exist")
        .output()
        .expect("failed to execute enqueue");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found"),
        "expected 'not found' error, got: {stderr}"
    );
}

#[test]
fn test_enqueue_memory_backend_prints_job_id() {
    let root = TempDir::new().unwrap();
    create_workflow(&root);

    let mut cmd = Command::cargo_bin("r8r").unwrap();
    configure_isolated_env(&mut cmd, &root);
    let output = cmd
        .arg("enqueue")
        .arg("queue-test-workflow")
        .arg("--input")
        .arg(r#"{"from": "cli-test"}"#)
        .output()
        .expect("failed to execute enqueue");

    assert!(
        output.status.success(),
        "enqueue failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Enqueued job"),
        "missing job id line, got: {stdout}"
    );
    assert!(
        stdout.contains("queue backend: memory"),
        "expected memory backend, got: {stdout}"
    );
    assert!(
        stdout.contains("workflow: queue-test-workflow (v1)"),
        "expected workflow/version line, got: {stdout}"
    );
}

#[test]
fn test_enqueue_invalid_json_fails() {
    let root = TempDir::new().unwrap();
    create_workflow(&root);

    let mut cmd = Command::cargo_bin("r8r").unwrap();
    configure_isolated_env(&mut cmd, &root);
    let output = cmd
        .arg("enqueue")
        .arg("queue-test-workflow")
        .arg("--input")
        .arg("{not json")
        .output()
        .expect("failed to execute enqueue");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Invalid JSON"),
        "expected Invalid JSON error, got: {stderr}"
    );
}

#[test]
fn test_worker_once_empty_queue_exits_cleanly() {
    let root = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    configure_isolated_env(&mut cmd, &root);
    let output = cmd
        .arg("worker")
        .arg("--once")
        .arg("--concurrency")
        .arg("1")
        .output()
        .expect("failed to execute worker --once");

    assert!(
        output.status.success(),
        "worker --once should exit 0 with an empty queue, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("no jobs in queue"),
        "expected 'no jobs in queue' message, got: {stdout}"
    );
}

#[test]
fn test_worker_rejects_redis_without_feature() {
    // When built with --all-features the RedisQueue path is compiled, so the
    // feature-gate error branch can't be exercised — skip in that case.
    if r8r::queue::HAS_REDIS_BACKEND {
        eprintln!("built with redis feature — skipping redis-absent branch test");
        return;
    }
    let root = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    configure_isolated_env(&mut cmd, &root);
    let output = cmd
        .arg("worker")
        .arg("--redis-url")
        .arg("redis://localhost:6379")
        .output()
        .expect("failed to execute worker with redis url");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("built without the `redis` feature"),
        "expected feature-gate error, got: {stderr}"
    );
}
