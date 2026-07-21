#![cfg(unix)]

use claude_code_sdk_rust::{ClaudeAgentClient, ClaudeAgentOptions};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

struct FakeCli {
    directory: PathBuf,
    executable: PathBuf,
    pid_file: PathBuf,
    ready_file: PathBuf,
}

impl FakeCli {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "claude-sdk-spawned-stream-child-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).expect("create fake CLI directory");

        let executable = directory.join("fake-claude");
        let pid_file = directory.join("cli.pid");
        let ready_file = directory.join("cli.ready");
        let mut file = fs::File::create(&executable).expect("create fake CLI executable");
        writeln!(
            file,
            r#"#!/bin/sh
if [ "${{1:-}}" = "-v" ]; then
  printf '2.0.0 (Claude Code)\n'
  exit 0
fi

printf '%s\n' "$$" > "$CLAUDE_FAKE_PID_FILE"
IFS= read -r initialize || exit 1
request_id=$(printf '%s\n' "$initialize" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
[ -n "$request_id" ] || exit 2
printf '{{"type":"control_response","response":{{"subtype":"success","request_id":"%s","response":{{}}}}}}\n' "$request_id"
IFS= read -r prompt || exit 3
printf 'ready\n' > "$CLAUDE_FAKE_READY_FILE"

# Keep the actual SDK-owned child alive after stdin is closed. `exec` preserves
# this script's PID, so the test can prove the transport kills and reaps it.
exec sleep 600
"#
        )
        .expect("write fake CLI executable");

        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&executable)
            .expect("read fake CLI permissions")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("make fake CLI executable");

        Self {
            directory,
            executable,
            pid_file,
            ready_file,
        }
    }

    fn options(&self) -> ClaudeAgentOptions {
        ClaudeAgentOptions::builder()
            .cli_path(self.executable.to_string_lossy().to_string())
            .env_var(
                "CLAUDE_FAKE_PID_FILE",
                self.pid_file.to_string_lossy().to_string(),
            )
            .env_var(
                "CLAUDE_FAKE_READY_FILE",
                self.ready_file.to_string_lossy().to_string(),
            )
            .build()
    }

    fn child_pid(&self) -> u32 {
        fs::read_to_string(&self.pid_file)
            .expect("read fake CLI PID")
            .trim()
            .parse()
            .expect("fake CLI PID must be numeric")
    }
}

impl Drop for FakeCli {
    fn drop(&mut self) {
        if let Ok(pid) = fs::read_to_string(&self.pid_file)
            .ok()
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .parse::<u32>()
        {
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

async fn wait_for_file(path: &Path, description: &str) {
    tokio::time::timeout(TEST_TIMEOUT, async {
        while !path.is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {description}"));
}

fn pid_exists(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

async fn wait_for_pid_to_be_reaped(pid: u32) {
    tokio::time::timeout(TEST_TIMEOUT, async {
        while pid_exists(pid) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("fake CLI child {pid} was not terminated and reaped"));
}

#[tokio::test]
async fn dropping_spawned_stream_receiver_terminates_and_reaps_cli_child() {
    let fake_cli = FakeCli::new();
    let events = ClaudeAgentClient::spawn_stream_message(fake_cli.options(), "hello");

    wait_for_file(
        &fake_cli.ready_file,
        "the fake CLI to finish initialization and receive the user frame",
    )
    .await;
    let pid = fake_cli.child_pid();
    assert!(pid_exists(pid), "fake CLI child {pid} should be running");

    drop(events);

    // `kill -0` continues to succeed for a zombie process. Its failure proves
    // that the SDK's owned child has been both terminated and reaped.
    wait_for_pid_to_be_reaped(pid).await;
}
