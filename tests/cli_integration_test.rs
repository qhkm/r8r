use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn test_json_flag_on_workflows_list() {
    let output = Command::cargo_bin("r8r")
        .unwrap()
        .arg("--json")
        .arg("workflows")
        .arg("list")
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should be valid JSON — either an array of workflows or {"ok":true,...} for empty
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("--json output should be valid JSON");
    assert!(parsed.is_array() || parsed.is_object());
}

#[test]
fn test_json_flag_on_doctor() {
    let output = Command::cargo_bin("r8r")
        .unwrap()
        .arg("--json")
        .arg("doctor")
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("--json doctor output should be valid JSON");
    assert!(parsed.get("checks").is_some());
}

#[test]
fn test_init_help_shows_flags() {
    Command::cargo_bin("r8r")
        .unwrap()
        .arg("init")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--yes"))
        .stdout(predicate::str::contains("--force"));
}
