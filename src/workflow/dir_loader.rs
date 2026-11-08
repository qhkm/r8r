//! Load every workflow YAML in a directory into storage (`r8r server --workflows <dir>`).

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::nodes::NodeRegistry;
use crate::storage::{Storage, StoredWorkflow};

use super::{parse_workflow, validate_workflow_with_registry};

/// What happened to each YAML file in the directory.
#[derive(Debug, Default)]
pub struct DirLoadReport {
    /// Workflow names that were saved.
    pub loaded: Vec<String>,
    /// Files that were not workflows or failed validation, with the reason.
    pub skipped: Vec<(PathBuf, String)>,
}

/// Parse, validate and save every `*.yaml` / `*.yml` file directly inside `dir`.
///
/// A file that is not a valid workflow is skipped and reported rather than
/// aborting the load, because a directory like `.` may also hold unrelated YAML
/// such as `docker-compose.yml`. Saving is an upsert by workflow name, and a new
/// version is only recorded when the definition changed, so loading the same
/// directory on every start is harmless.
pub async fn load_workflows_from_dir(
    dir: &Path,
    storage: &dyn Storage,
    registry: &NodeRegistry,
) -> Result<DirLoadReport> {
    if !dir.is_dir() {
        return Err(Error::Parse(format!(
            "--workflows: {} is not a directory",
            dir.display()
        )));
    }

    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| Error::Parse(format!("cannot read {}: {e}", dir.display())))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file() && matches!(p.extension().and_then(|e| e.to_str()), Some("yaml" | "yml"))
        })
        .collect();
    files.sort();

    let mut report = DirLoadReport::default();
    for path in files {
        let definition = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) => {
                report.skipped.push((path, e.to_string()));
                continue;
            }
        };
        let workflow = match parse_workflow(&definition)
            .and_then(|wf| validate_workflow_with_registry(&wf, registry).map(|_| wf))
        {
            Ok(wf) => wf,
            Err(e) => {
                report.skipped.push((path, e.to_string()));
                continue;
            }
        };

        let now = chrono::Utc::now();
        storage
            .save_workflow(&StoredWorkflow {
                id: uuid::Uuid::new_v4().to_string(),
                name: workflow.name.clone(),
                definition,
                enabled: true,
                created_at: now,
                updated_at: now,
            })
            .await?;
        report.loaded.push(workflow.name);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SqliteStorage;

    const HELLO: &str = r#"name: hello-world
nodes:
  - id: greet
    type: transform
    config:
      expression: '"Hello"'
"#;

    #[tokio::test]
    async fn loads_workflows_and_skips_other_yaml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.yaml"), HELLO).unwrap();
        std::fs::write(
            dir.path().join("docker-compose.yml"),
            "services:\n  web:\n    image: nginx\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not yaml").unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let report = load_workflows_from_dir(dir.path(), &storage, &NodeRegistry::new())
            .await
            .unwrap();

        assert_eq!(report.loaded, vec!["hello-world".to_string()]);
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].0.ends_with("docker-compose.yml"));
        assert!(storage.get_workflow("hello-world").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn reloading_is_an_upsert() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.yaml"), HELLO).unwrap();
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();

        load_workflows_from_dir(dir.path(), &storage, &registry)
            .await
            .unwrap();
        load_workflows_from_dir(dir.path(), &storage, &registry)
            .await
            .unwrap();

        assert_eq!(storage.list_workflows().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn missing_directory_is_an_error() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let err = load_workflows_from_dir(
            Path::new("/nonexistent/r8r-workflows"),
            &storage,
            &NodeRegistry::new(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not a directory"));
    }
}
