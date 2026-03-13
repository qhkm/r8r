use std::process::Command;

fn run_install_snippet(script_path: &str, snippet: &str) -> std::process::Output {
    Command::new("sh")
        .arg("-ceu")
        .arg(snippet)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("SCRIPT_UNDER_TEST", script_path)
        .output()
        .expect("failed to execute shell snippet")
}

#[test]
fn install_script_prefers_cdn_latest_file_when_present() {
    let output = run_install_snippet(
        "install.sh",
        r#"
curl() {
  case "$*" in
    *"/latest.txt"*)
      printf '%s' 'v1.2.3'
      ;;
    *"github.com/qhkm/r8r/releases/latest"*)
      echo "unexpected GitHub fallback" >&2
      return 1
      ;;
    *)
      echo "unexpected curl args: $*" >&2
      return 1
      ;;
  esac
}

R8R_INSTALLER_SKIP_MAIN=1
. "./${SCRIPT_UNDER_TEST}"
say() { :; }
resolve_version
[ "${VERSION}" = "v1.2.3" ]
"#,
    );

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn install_script_falls_back_to_github_release_redirect() {
    let output = run_install_snippet(
        "install.sh",
        r#"
curl() {
  case "$*" in
    *"/latest.txt"*)
      return 22
      ;;
    *"github.com/qhkm/r8r/releases/latest"*)
      printf '%s' 'https://github.com/qhkm/r8r/releases/tag/v9.8.7'
      ;;
    *)
      echo "unexpected curl args: $*" >&2
      return 1
      ;;
  esac
}

R8R_INSTALLER_SKIP_MAIN=1
. "./${SCRIPT_UNDER_TEST}"
say() { :; }
resolve_version
[ "${VERSION}" = "v9.8.7" ]
"#,
    );

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
