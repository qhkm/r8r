use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

fn prepare_env(root: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let config_home = root.path().join("config");
    let data_home = root.path().join("data");
    std::fs::create_dir_all(&config_home).unwrap();
    std::fs::create_dir_all(&data_home).unwrap();
    (config_home, data_home)
}

fn configure_isolated_env(cmd: &mut Command, root: &TempDir) {
    let (config_home, data_home) = prepare_env(root);
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
        .env_remove("DATABASE_URL");
}

fn write_workflow(path: &Path) {
    std::fs::write(
        path,
        r#"name: test-workflow
nodes:
  - id: step1
    type: transform
    config:
      expression: '"ok"'
"#,
    )
    .unwrap();
}

#[test]
fn test_json_flag_on_workflows_list() {
    let root = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    configure_isolated_env(&mut cmd, &root);
    let output = cmd
        .arg("--json")
        .arg("workflows")
        .arg("list")
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should be valid JSON — either an array of workflows or {"ok":true,...} for empty
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("--json output should be valid JSON");
    assert!(parsed.is_array() || parsed.is_object());
}

#[test]
fn test_json_flag_on_doctor() {
    let root = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    configure_isolated_env(&mut cmd, &root);
    let output = cmd
        .arg("--json")
        .arg("doctor")
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("--json doctor output should be valid JSON");
    assert!(parsed.get("checks").is_some());
    assert!(
        !output.status.success(),
        "doctor should fail when checks fail"
    );
}

#[test]
fn test_json_init_outputs_json_and_configures_llm() {
    let root = TempDir::new().unwrap();
    let mut init_cmd = Command::cargo_bin("r8r").unwrap();
    configure_isolated_env(&mut init_cmd, &root);
    let init_output = init_cmd
        .env("OPENAI_API_KEY", "sk-test")
        .arg("--json")
        .arg("init")
        .arg("--yes")
        .output()
        .expect("failed to execute init");

    assert!(init_output.status.success());
    let init_stdout = String::from_utf8_lossy(&init_output.stdout);
    let init_json: serde_json::Value =
        serde_json::from_str(init_stdout.trim()).expect("init should emit valid JSON");
    assert_eq!(
        init_json.get("provider").and_then(|v| v.as_str()),
        Some("openai")
    );
    assert_eq!(
        init_json.get("credential_saved").and_then(|v| v.as_bool()),
        Some(true)
    );

    let mut doctor_cmd = Command::cargo_bin("r8r").unwrap();
    configure_isolated_env(&mut doctor_cmd, &root);
    let doctor_output = doctor_cmd
        .arg("--json")
        .arg("doctor")
        .output()
        .expect("failed to execute doctor");

    assert!(doctor_output.status.success());
    let doctor_stdout = String::from_utf8_lossy(&doctor_output.stdout);
    let doctor_json: serde_json::Value =
        serde_json::from_str(doctor_stdout.trim()).expect("doctor should emit valid JSON");
    let llm_check = doctor_json["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "LLM config")
        .unwrap();
    assert_eq!(llm_check["pass"].as_bool(), Some(true));
}

#[test]
fn test_json_flag_on_workflows_create() {
    let root = TempDir::new().unwrap();
    let workflow_path = root.path().join("workflow.yaml");
    write_workflow(&workflow_path);

    let mut cmd = Command::cargo_bin("r8r").unwrap();
    configure_isolated_env(&mut cmd, &root);
    let output = cmd
        .arg("--json")
        .arg("workflows")
        .arg("create")
        .arg(&workflow_path)
        .output()
        .expect("failed to execute workflows create");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("--json workflows create output should be valid JSON");
    assert_eq!(parsed["ok"].as_bool(), Some(true));
    assert_eq!(parsed["name"].as_str(), Some("test-workflow"));
}

#[test]
fn test_json_flag_on_credentials_set() {
    let root = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    configure_isolated_env(&mut cmd, &root);
    let output = cmd
        .arg("--json")
        .arg("credentials")
        .arg("set")
        .arg("openai")
        .arg("--value")
        .arg("secret")
        .output()
        .expect("failed to execute credentials set");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("--json credentials set output should be valid JSON");
    assert_eq!(parsed["ok"].as_bool(), Some(true));
    assert_eq!(parsed["service"].as_str(), Some("openai"));
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
