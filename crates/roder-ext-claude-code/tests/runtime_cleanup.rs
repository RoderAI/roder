#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use roder_api::catalog::PROVIDER_CLAUDE_CODE;
use roder_api::events::RoderEvent;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::InstructionBundle;
use roder_api::lifecycle::{TurnCleanupOwnership, TurnCleanupState, TurnLifecycleState};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest};
use roder_ext_claude_code::{ClaudeCodeConfig, ClaudeCodeEngine};

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

struct FakeClaudeCli {
    directory: PathBuf,
    executable: PathBuf,
    pid_file: PathBuf,
    ready_file: PathBuf,
}

impl FakeClaudeCli {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "roder-claude-runtime-cleanup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is after Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create fake Claude CLI directory");

        let executable = directory.join("fake-claude");
        let pid_file = directory.join("fake-claude.pid");
        let ready_file = directory.join("fake-claude.ready");
        let mut file = fs::File::create(&executable).expect("create fake Claude CLI executable");
        writeln!(
            file,
            r#"#!/bin/sh
if [ "${{1:-}}" = "-v" ]; then
  printf '2.0.0 (Claude Code)\n'
  exit 0
fi

printf '%s\n' "$$" > "${{0}}.pid"
IFS= read -r initialize || exit 1
request_id=$(printf '%s\n' "$initialize" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
[ -n "$request_id" ] || exit 2
printf '{{"type":"control_response","response":{{"subtype":"success","request_id":"%s","response":{{}}}}}}\n' "$request_id"
IFS= read -r prompt || exit 3
printf 'ready\n' > "${{0}}.ready"

# Preserve this PID while the child blocks so `kill -0` below distinguishes a
# reaped child from a merely terminated zombie.
exec sleep 600
"#
        )
        .expect("write fake Claude CLI executable");

        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&executable)
            .expect("read fake Claude CLI permissions")
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

    fn child_pid(&self) -> u32 {
        fs::read_to_string(&self.pid_file)
            .expect("read fake Claude child PID")
            .trim()
            .parse()
            .expect("fake Claude child PID is numeric")
    }
}

impl Drop for FakeClaudeCli {
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
    .unwrap_or_else(|_| panic!("fake Claude child {pid} was not terminated and reaped"));
}

#[tokio::test]
async fn runtime_interrupt_reaps_claude_cli_before_confirming_cleanup() {
    let fake_cli = FakeClaudeCli::new();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(ClaudeCodeEngine::new(ClaudeCodeConfig {
        cli_path: Some(fake_cli.executable.display().to_string()),
        workspace: Some(fake_cli.directory.clone()),
        ..ClaudeCodeConfig::default()
    })));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().expect("build extension registry"),
            RuntimeConfig {
                default_provider: PROVIDER_CLAUDE_CODE.to_string(),
                default_model: "sonnet".to_string(),
                workspace: Some(fake_cli.directory.display().to_string()),
                ..RuntimeConfig::default()
            },
        )
        .expect("construct runtime"),
    );
    let mut events = runtime.subscribe_events();
    let thread_id = "thread-claude-runtime-cleanup".to_string();
    let turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: thread_id.clone(),
            message: "wait until interrupted".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: fake_cli.directory.display().to_string(),
            instructions: InstructionBundle::default(),
            developer_context: None,
            task_ledger_required: false,
        })
        .await
        .expect("start Claude Code turn");

    wait_for_file(
        &fake_cli.ready_file,
        "the fake Claude CLI to initialize and receive the user frame",
    )
    .await;
    let pid = fake_cli.child_pid();
    assert!(pid_exists(pid), "fake Claude child {pid} should be running");

    runtime
        .interrupt_turn(thread_id.clone(), turn_id.clone())
        .await
        .expect("interrupt active Claude Code turn");

    let mut saw_interrupt_requested = false;
    let terminal = tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            let envelope = events.recv().await.expect("runtime event");
            if envelope.turn_id.as_deref() != Some(turn_id.as_str()) {
                continue;
            }
            match envelope.event {
                RoderEvent::TurnLifecycleUpdated(record) => match record.state {
                    TurnLifecycleState::InterruptRequested => {
                        saw_interrupt_requested = true;
                    }
                    TurnLifecycleState::Interrupted => break record,
                    TurnLifecycleState::Completed => {
                        panic!("interrupted Claude turn must not be marked completed")
                    }
                    _ => {}
                },
                RoderEvent::TurnCompleted(_) => {
                    panic!("interrupted Claude turn emitted turn.completed")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("interrupted lifecycle acknowledgement");

    assert!(
        saw_interrupt_requested,
        "interrupt request was not observed"
    );
    assert_eq!(terminal.cleanup, TurnCleanupState::Completed);
    assert_eq!(
        terminal.ownership,
        TurnCleanupOwnership::ProviderCleanupConfirmed
    );
    // `kill -0` remains successful for zombies. Its failure proves the child
    // was reaped before Roder emitted the confirmed lifecycle record above.
    wait_for_pid_to_be_reaped(pid).await;
}
