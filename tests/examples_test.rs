//! Every bundled example and every workflow in the README must pass
//! `r8r workflows validate`. These are the first things new users copy.

use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn validate(path: &Path) -> Result<(), String> {
    let home = TempDir::new().unwrap();
    let output = Command::cargo_bin("r8r")
        .unwrap()
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("R8R_DATA_DIR", home.path().join("r8r"))
        .args(["workflows", "validate"])
        .arg(path)
        .output()
        .unwrap();
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[test]
fn bundled_examples_validate() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml" || x == "yml"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no examples found in {}", dir.display());

    let mut failures = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        // The sandbox node only exists in builds with the `sandbox` feature.
        if !cfg!(feature = "sandbox") && text.contains("type: sandbox") {
            continue;
        }
        if let Err(e) = validate(file) {
            failures.push(format!("{}:\n{}", file.display(), e));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn readme_workflows_validate() {
    let readme =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md")).unwrap();
    let tmp = TempDir::new().unwrap();

    let mut checked = 0;
    let mut failures = Vec::new();
    let mut rest = readme.as_str();
    while let Some(start) = rest.find("```yaml\n") {
        let body_start = start + "```yaml\n".len();
        let Some(len) = rest[body_start..].find("```") else {
            break;
        };
        let block = &rest[body_start..body_start + len];
        rest = &rest[body_start + len + 3..];
        // Only whole workflows; fragments (a trigger, a config snippet) are skipped.
        if !block.lines().any(|l| l.starts_with("nodes:")) {
            continue;
        }
        checked += 1;
        let path = tmp.path().join(format!("readme-{}.yaml", checked));
        std::fs::write(&path, block).unwrap();
        if let Err(e) = validate(&path) {
            failures.push(format!("README workflow #{}:\n{}\n{}", checked, block, e));
        }
    }
    assert!(checked > 0, "no workflows found in README.md");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
