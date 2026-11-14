//! End-to-end checks that run the real binary.
//!
//! Every test uses an empty temporary directory as the working directory so the
//! real `config.json` / `credentials/` are never touched.

use std::path::Path;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_megatem"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run megatem")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn version_matches_the_python_release() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(dir.path(), &["--version"]);
    assert!(output.status.success());
    let text = stdout(&output);
    assert!(text.contains("1.8.0"), "unexpected version output: {text}");
}

#[test]
fn help_documents_the_main_flags() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(dir.path(), &["--help"]);
    assert!(output.status.success());
    let text = stdout(&output);
    for flag in ["--keepalive", "--loop", "--parallel", "--prune", "--extract"] {
        assert!(text.contains(flag), "help is missing {flag}");
    }
}

#[test]
fn health_reports_json_for_an_empty_store() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(dir.path(), &["--health", "--json"]);
    assert!(output.status.success());
    let text = stdout(&output);
    let value: serde_json::Value = serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("health did not print JSON ({e}): {text}"));
    assert!(value.get("accounts").and_then(|v| v.as_array()).is_some());
    assert_eq!(
        value
            .get("summary")
            .and_then(|s| s.get("total"))
            .and_then(|v| v.as_u64()),
        Some(0)
    );
}

#[test]
fn json_without_health_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(dir.path(), &["--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--health"));
}

#[test]
fn unknown_provider_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(dir.path(), &["--provider", "nope", "--keepalive"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("nope"));
}

#[test]
fn extract_writes_credentials_txt() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(dir.path(), &["--extract"]);
    assert!(output.status.success());
    assert!(dir.path().join("credentials.txt").is_file());
}

#[test]
fn maintenance_commands_succeed_offline() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(dir.path(), &["--disk-usage"]).status.success());
    assert!(run(dir.path(), &["--clean"]).status.success());
}
