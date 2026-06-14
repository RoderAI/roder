//! Process-level non-TTY exec smoke (roadmap phase 65).
//!
//! Spawns the real `roder` binary against an isolated config/data directory
//! so the default provider resolves to the offline fake provider, then
//! asserts the documented exec stdout/JSONL/exit-code contract without a TTY
//! and without network access.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn roder_binary() -> &'static str {
    env!("CARGO_BIN_EXE_roder")
}

struct ExecRun {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn run_exec(home: &Path, workspace: &Path, args: &[&str], stdin: &str) -> ExecRun {
    let mut child = Command::new(roder_binary())
        .arg("exec")
        .args(args)
        .current_dir(workspace)
        .env("RODER_CONFIG_DIR", home)
        .env("RODER_DATA_DIR", home)
        .env_remove("RODER_PROVIDER")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn roder exec");
    child
        .stdin
        .as_mut()
        .expect("exec stdin")
        .write_all(stdin.as_bytes())
        .expect("write exec prompt");
    let output = child.wait_with_output().expect("wait for roder exec");
    ExecRun {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "roder-exec-smoke-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn exec_process_smoke_json_mode_emits_typed_jsonl_with_fake_provider() {
    let home = temp_dir("home");
    let workspace = temp_dir("ws");

    let run = run_exec(
        &home,
        &workspace,
        &["--json", "--profile", "eval", "--mode", "bypass", "-"],
        "Reply with exactly: ok\n",
    );

    assert!(
        run.status.success(),
        "exec failed: status={:?} stderr={}",
        run.status,
        run.stderr
    );
    let mut types = Vec::new();
    for line in run.stdout.lines().filter(|line| !line.trim().is_empty()) {
        let event: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|err| panic!("invalid JSONL line {line:?}: {err}"));
        types.push(
            event
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        );
    }
    assert!(
        types.iter().any(|kind| kind == "thread.started"),
        "missing thread.started in {types:?}"
    );
    assert!(
        types.iter().any(|kind| kind == "turn.started"),
        "missing turn.started in {types:?}"
    );
    assert!(
        types.iter().any(|kind| kind == "turn.completed"),
        "missing turn.completed in {types:?}"
    );

    let _ = std::fs::remove_dir_all(home);
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn exec_process_smoke_default_mode_prints_only_final_message() {
    let home = temp_dir("home-text");
    let workspace = temp_dir("ws-text");
    let last_message = home.join("last-message.txt");

    let run = run_exec(
        &home,
        &workspace,
        &[
            "--profile",
            "eval",
            "--mode",
            "bypass",
            "--output-last-message",
            last_message.to_str().unwrap(),
            "-",
        ],
        "Reply with exactly: ok\n",
    );

    assert!(
        run.status.success(),
        "exec failed: status={:?} stderr={}",
        run.status,
        run.stderr
    );
    // The offline fake provider always answers with a fixed message; default
    // stdout must contain only that final assistant message.
    assert_eq!(
        run.stdout.trim(),
        "hello from roder",
        "stdout: {:?}",
        run.stdout
    );
    let written = std::fs::read_to_string(&last_message).expect("last message file");
    assert_eq!(written.trim(), "hello from roder");

    let _ = std::fs::remove_dir_all(home);
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn exec_process_smoke_records_api_transcript_for_debugging() {
    let home = temp_dir("home-transcript");
    let workspace = temp_dir("ws-transcript");
    let transcript = home.join("captures/exec-transcript.jsonl");

    let run = run_exec(
        &home,
        &workspace,
        &[
            "--json",
            "--profile",
            "eval",
            "--mode",
            "bypass",
            "--record-api-transcript",
            transcript.to_str().unwrap(),
            "-",
        ],
        "Reply with exactly: ok\n",
    );

    assert!(
        run.status.success(),
        "exec failed: status={:?} stderr={}",
        run.status,
        run.stderr
    );
    let raw = std::fs::read_to_string(&transcript).expect("transcript file");
    let lines: Vec<&str> = raw.lines().filter(|line| !line.trim().is_empty()).collect();
    assert!(
        lines.len() > 3,
        "expected header plus request/response/notification records, got {} lines",
        lines.len()
    );
    let header: serde_json::Value = serde_json::from_str(lines[0]).expect("header record");
    assert_eq!(header["kind"], "header");
    assert!(
        header["features"]
            .as_array()
            .is_some_and(|features| features.iter().any(|value| value == "exec")),
        "header must mark the exec feature: {header}"
    );
    assert!(
        raw.contains("turn/start"),
        "transcript must capture turn/start"
    );
    assert!(
        raw.contains("thread/start"),
        "transcript must capture thread/start"
    );

    let _ = std::fs::remove_dir_all(home);
    let _ = std::fs::remove_dir_all(workspace);
}
