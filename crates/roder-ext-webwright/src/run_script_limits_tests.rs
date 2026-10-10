//! Limits on a `webwright.run_script` run: how much of a noisy script's output is kept, and that a timeout
//! or a cancelled call ends the script's whole process group, not only the interpreter.
#![cfg(unix)]

use std::fs;

use serde_json::json;

use crate::run_script_tests::{Fixture, WORKSPACE, assert_dies, is_running, wait_for_pid};
use crate::tools::WEBWRIGHT_RUN_SCRIPT_TOOL;
use crate::tools_tests::call;

const MIB: usize = 1024 * 1024;

#[tokio::test]
async fn a_noisy_script_cannot_grow_the_captured_output_without_bound() {
    let fixture = Fixture::new("noisy-script").await;
    // 8 MiB of filler between the root cause and the last line, then a newline so the marker is a whole line.
    fixture.script(
        "echo \"RootError: root cause\" >&2\nyes xxxxxxxx | head -c 8388608 >&2\necho >&2\necho LAST_MARKER >&2\nexit 1\n",
    );

    let run = fixture.run(60).await;

    assert!(run.is_error);
    let stderr = run.data["webwright"]["stderr"].as_str().unwrap();
    assert!(
        stderr.len() < MIB + MIB / 10,
        "stderr kept {} bytes",
        stderr.len()
    );
    assert!(
        stderr.contains("bytes of script output omitted"),
        "no omission marker in the kept output"
    );
    assert!(stderr.starts_with("RootError: root cause\n"));
    assert!(stderr.trim_end().ends_with("LAST_MARKER"));
    // The model text still carries the root cause and the end of the output.
    assert!(
        run.text.contains(
            "first error (untrusted script output, secrets redacted): RootError: root cause"
        ),
        "{}",
        run.text
    );
    assert!(run.text.contains("LAST_MARKER"), "{}", run.text);
    assert!(run.text.len() < 6000, "{} bytes", run.text.len());
    // The retained log is bounded the same way.
    let log = fs::read_to_string(
        fixture
            .workspace()
            .join("final_runs/run_001/final_script_log.txt"),
    )
    .unwrap();
    assert!(log.len() < MIB + MIB / 10, "log kept {} bytes", log.len());
    assert!(log.contains("bytes of script output omitted"));
}

#[tokio::test]
async fn timeout_kills_the_processes_the_script_started() {
    let fixture = Fixture::new("timeout-kills-group").await;
    fixture.script("sleep 30 &\necho $! > child.pid\nwait\n");

    let run = fixture.run(1).await;

    assert!(run.is_error);
    assert_eq!(run.data["webwright"]["outcome"], "timeout");
    let child = wait_for_pid(&fixture.workspace().join("final_runs/run_001/child.pid")).await;
    assert_dies(child).await;
}

#[tokio::test]
async fn dropping_the_run_kills_the_processes_the_script_started() {
    let fixture = Fixture::new("drop-kills-group").await;
    fixture.script("sleep 30 &\necho $! > child.pid\nwait\n");
    let tool = fixture.registry.get(WEBWRIGHT_RUN_SCRIPT_TOOL).unwrap();
    let ctx = fixture.ctx.clone();
    let running = tokio::spawn(async move {
        tool.execute(
            ctx,
            call(
                WEBWRIGHT_RUN_SCRIPT_TOOL,
                json!({ "workspace": WORKSPACE, "python": "sh", "timeoutSeconds": 60 }),
            ),
        )
        .await
    });

    let child = wait_for_pid(&fixture.workspace().join("final_runs/run_001/child.pid")).await;
    assert!(is_running(child));
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());

    assert_dies(child).await;
}
