//! `webwright.run_script` and `webwright.verify_run` against scripted shell "interpreters": what a failed run
//! tells the model, that no child outlives its run, and that the exit status gates verification.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use roder_api::tools::{ToolContributor, ToolExecutionContext, ToolRegistry, ToolResult};
use serde_json::{Value, json};

use crate::tools::{
    WEBWRIGHT_ALLOCATE_RUN_TOOL, WEBWRIGHT_LIST_ARTIFACTS_TOOL, WEBWRIGHT_PREPARE_WORKSPACE_TOOL,
    WEBWRIGHT_RUN_SCRIPT_TOOL, WEBWRIGHT_VERIFY_RUN_TOOL, WebwrightToolContributor,
};
use crate::tools_tests::{call, tempdir, webwright_context};

const WORKSPACE: &str = ".roder/webwright/fixture";
const REDACTED: &str = "[redacted sensitive Webwright output line]";

/// Everything verification looks for except a clean exit: a screenshot and a final datum line.
const EVIDENCE: &str = "mkdir -p screenshots\nprintf png > screenshots/final_execution_001_ok.png\nprintf 'step 1 action: ok\\nfinal datum: Fixture Heading\\n' > final_script_log.txt\n";

struct Fixture {
    root: PathBuf,
    registry: ToolRegistry,
    ctx: ToolExecutionContext,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        let root = tempdir(name);
        let mut registry = ToolRegistry::default();
        WebwrightToolContributor.contribute(&mut registry).unwrap();
        let fixture = Self {
            ctx: webwright_context(root.clone()),
            root,
            registry,
        };
        let prepared = fixture
            .tool(
                WEBWRIGHT_PREPARE_WORKSPACE_TOOL,
                json!({ "task": "Open fixture page", "taskId": "fixture" }),
            )
            .await;
        assert!(!prepared.is_error, "{}", prepared.text);
        fs::write(
            fixture.workspace().join("plan.md"),
            "# Critical Points\n- [x] CP1: Complete the requested Webwright task: Open fixture page\n",
        )
        .unwrap();
        fixture
    }

    fn workspace(&self) -> PathBuf {
        self.root.join(WORKSPACE)
    }

    fn script(&self, body: &str) {
        fs::write(self.workspace().join("final_script.py"), body).unwrap();
    }

    async fn tool(&self, name: &str, arguments: Value) -> ToolResult {
        self.registry
            .get(name)
            .unwrap()
            .execute(self.ctx.clone(), call(name, arguments))
            .await
            .unwrap()
    }

    /// Runs the script with `sh` as the interpreter.
    async fn run(&self, timeout_seconds: u64) -> ToolResult {
        self.run_with("sh", timeout_seconds).await
    }

    async fn run_with(&self, python: &str, timeout_seconds: u64) -> ToolResult {
        self.tool(
            WEBWRIGHT_RUN_SCRIPT_TOOL,
            json!({ "workspace": WORKSPACE, "python": python, "timeoutSeconds": timeout_seconds }),
        )
        .await
    }

    async fn verify(&self) -> ToolResult {
        self.tool(WEBWRIGHT_VERIFY_RUN_TOOL, json!({ "workspace": WORKSPACE }))
            .await
    }
}

fn failed_check(verification: &ToolResult, id: &str) -> bool {
    verification.data["webwright"]["verification"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check["id"] == id && check["passed"] == false)
}

fn passed_check(verification: &ToolResult, id: &str) -> bool {
    verification.data["webwright"]["verification"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check["id"] == id && check["passed"] == true)
}

async fn wait_for_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(pid) = fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "no pid file at {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A zombie is dead; only a process that can still run counts.
fn is_running(pid: u32) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&out.stdout);
    let stat = stat.trim();
    !stat.is_empty() && !stat.starts_with('Z')
}

async fn assert_dies(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while is_running(pid) {
        assert!(
            Instant::now() < deadline,
            "script process {pid} is still running"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn failed_run_text_explains_the_failure() {
    let fixture = Fixture::new("explain-failure").await;
    fixture.script(
        r#"printf "Traceback (most recent call last):\n  File \"final_script.py\", line 9, in <module>\nModuleNotFoundError: No module named 'playwright'\nAuthorization: Bearer hunter2secret\n" >&2
exit 3
"#,
    );

    let run = fixture.run(10).await;

    assert!(run.is_error);
    let first_line = run.text.lines().next().unwrap();
    assert!(
        first_line.starts_with("webwright run_001 failed (nonzero_exit): exit code 3 after "),
        "{}",
        run.text
    );
    assert!(first_line.ends_with('s'), "{first_line}");
    assert!(
        run.text.contains("hint: missing_python_module"),
        "{}",
        run.text
    );
    assert!(
        run.text
            .contains("first error (untrusted script output, secrets redacted): ModuleNotFoundError: No module named 'playwright'"),
        "{}",
        run.text
    );
    assert!(run.text.contains("stderr tail"), "{}", run.text);
    assert!(run.text.contains("Traceback (most recent call last):"));
    assert!(run.text.contains(REDACTED), "{}", run.text);
    assert!(!run.text.contains("hunter2secret"), "{}", run.text);
    assert!(run.text.contains("webwright.read_log_tail"), "{}", run.text);

    let data = &run.data["webwright"];
    assert_eq!(data["outcome"], "nonzero_exit");
    assert_eq!(data["exitCode"], 3);
    assert!(data["elapsedMs"].is_u64(), "{data}");
    assert_eq!(data["hint"], "missing_python_module");
    assert!(!data["stderr"].as_str().unwrap().contains("hunter2secret"));
}

#[tokio::test]
async fn successful_run_text_is_one_line() {
    let fixture = Fixture::new("one-line-success").await;
    fixture.script(&format!("{EVIDENCE}echo 'noise on stderr' >&2\n"));

    let run = fixture.run(10).await;

    assert!(!run.is_error, "{}", run.text);
    assert_eq!(run.text.lines().count(), 1, "{}", run.text);
    assert!(
        run.text
            .starts_with("webwright run_001 ok: exit code 0 in "),
        "{}",
        run.text
    );
    assert_eq!(run.data["webwright"]["outcome"], "ok");
}

#[tokio::test]
async fn stderr_tail_is_capped_and_first_error_comes_from_the_whole_output() {
    let fixture = Fixture::new("cap-tail").await;
    fixture.script(
        r#"echo "RootError: root cause" >&2
i=0
while [ $i -lt 600 ]; do echo "filler line $i xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx" >&2; i=$((i+1)); done
echo "LAST LINE MARKER" >&2
exit 1
"#,
    );

    let run = fixture.run(10).await;

    assert!(run.is_error);
    assert!(run.text.len() < 6000, "{} bytes", run.text.len());
    assert!(run.text.contains("LAST LINE MARKER"), "{}", run.text);
    assert!(
        run.text.contains(
            "first error (untrusted script output, secrets redacted): RootError: root cause"
        ),
        "{}",
        run.text
    );
    assert!(!run.text.contains("filler line 0 "), "{}", run.text);
    assert!(
        run.data["webwright"]["stderr"]
            .as_str()
            .unwrap()
            .contains("filler line 0 "),
        "data keeps the full redacted stderr"
    );
}

#[tokio::test]
async fn stdout_tail_is_shown_when_stderr_is_empty() {
    let fixture = Fixture::new("stdout-tail").await;
    fixture.script("echo 'boom on stdout'\nexit 2\n");

    let run = fixture.run(10).await;

    assert!(run.is_error);
    assert!(run.text.contains("stdout tail"), "{}", run.text);
    assert!(run.text.contains("boom on stdout"), "{}", run.text);
}

#[tokio::test]
async fn launch_failure_is_a_classified_result_not_a_tool_error() {
    let fixture = Fixture::new("launch-failure").await;
    fixture.script("exit 0\n");

    let run = fixture
        .run_with("/nonexistent/webwright-interpreter", 10)
        .await;

    assert!(run.is_error);
    assert!(
        run.text
            .starts_with("webwright run_001 failed (launch_failed)"),
        "{}",
        run.text
    );
    assert!(
        run.text.contains("/nonexistent/webwright-interpreter"),
        "{}",
        run.text
    );
    assert_eq!(run.data["webwright"]["outcome"], "launch_failed");
    let verification = fixture.verify().await;
    assert!(verification.is_error);
    assert!(
        failed_check(&verification, "script_exit"),
        "{verification:?}"
    );
}

#[tokio::test]
async fn timeout_kills_the_child_and_keeps_partial_output() {
    let fixture = Fixture::new("timeout-kills").await;
    fixture.script("echo 'reached step 2' >&2\necho $$ > pid.txt\nexec sleep 30\n");

    let started = Instant::now();
    let run = fixture.run(1).await;

    assert!(started.elapsed() < Duration::from_secs(15));
    assert!(run.is_error);
    let pid = wait_for_pid(&fixture.workspace().join("final_runs/run_001/pid.txt")).await;
    assert_dies(pid).await;
    assert!(
        run.text
            .starts_with("webwright run_001 failed (timeout): killed after "),
        "{}",
        run.text
    );
    assert!(run.text.contains("timeoutSeconds=1"), "{}", run.text);
    assert!(run.text.contains("reached step 2"), "{}", run.text);
    assert_eq!(run.data["webwright"]["outcome"], "timeout");
    assert_eq!(run.data["webwright"]["timedOut"], true);
}

#[tokio::test]
async fn dropping_the_run_kills_the_child() {
    let fixture = Fixture::new("drop-kills").await;
    fixture.script("echo $$ > pid.txt\nexec sleep 30\n");
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

    let pid = wait_for_pid(&fixture.workspace().join("final_runs/run_001/pid.txt")).await;
    assert!(is_running(pid));
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());

    assert_dies(pid).await;
}

#[tokio::test]
async fn run_returns_without_waiting_for_background_children_that_hold_the_pipes() {
    let fixture = Fixture::new("background-child").await;
    fixture.script(&format!("{EVIDENCE}sleep 6 &\nexit 0\n"));

    let started = Instant::now();
    let run = fixture.run(30).await;

    assert!(!run.is_error, "{}", run.text);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "waited {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn verification_fails_when_the_script_exited_nonzero() {
    let fixture = Fixture::new("verify-nonzero").await;
    fixture.script(&format!("{EVIDENCE}exit 1\n"));

    let run = fixture.run(10).await;
    assert!(run.is_error);

    let verification = fixture.verify().await;
    // The evidence checks all pass; only the exit status says the run failed.
    for id in ["critical_points", "screenshot_count", "final_datum"] {
        assert!(passed_check(&verification, id), "{id}: {verification:?}");
    }
    assert!(verification.is_error, "{}", verification.text);
    assert!(
        failed_check(&verification, "script_exit"),
        "{verification:?}"
    );
    assert!(
        verification.text.contains("exit code 1"),
        "{}",
        verification.text
    );
}

#[tokio::test]
async fn verification_fails_when_the_script_timed_out_after_writing_its_evidence() {
    let fixture = Fixture::new("verify-timeout").await;
    fixture.script(&format!("{EVIDENCE}exec sleep 30\n"));

    let run = fixture.run(1).await;
    assert!(run.is_error);

    let verification = fixture.verify().await;
    assert!(verification.is_error, "{}", verification.text);
    assert!(
        failed_check(&verification, "script_exit"),
        "{verification:?}"
    );
    assert!(
        verification.text.contains("timeout"),
        "{}",
        verification.text
    );
}

#[tokio::test]
async fn verification_passes_after_a_clean_exit() {
    let fixture = Fixture::new("verify-clean").await;
    fixture.script(EVIDENCE);

    let run = fixture.run(10).await;
    assert!(!run.is_error, "{}", run.text);

    let verification = fixture.verify().await;
    assert!(!verification.is_error, "{}", verification.text);
    assert!(
        passed_check(&verification, "script_exit"),
        "{verification:?}"
    );
}

#[tokio::test]
async fn preparing_again_keeps_the_manifest_and_run_state() {
    let fixture = Fixture::new("keep-manifest").await;
    let allocated = fixture
        .tool(
            WEBWRIGHT_ALLOCATE_RUN_TOOL,
            json!({ "workspace": WORKSPACE }),
        )
        .await;
    assert!(!allocated.is_error, "{}", allocated.text);

    let again = fixture
        .tool(
            WEBWRIGHT_PREPARE_WORKSPACE_TOOL,
            json!({
                "task": "A different task",
                "taskId": "fixture",
                "browser": "chromium"
            }),
        )
        .await;
    assert!(!again.is_error, "{}", again.text);

    let listed = fixture
        .tool(
            WEBWRIGHT_LIST_ARTIFACTS_TOOL,
            json!({ "workspace": WORKSPACE }),
        )
        .await;
    let manifest = &listed.data["webwright"]["workspace"]["manifest"];
    assert_eq!(manifest["latestRun"], 1, "{manifest}");
    assert_eq!(manifest["task"], "Open fixture page");
    assert_eq!(manifest["browser"], "firefox");
    // The tool says the request was not applied, so the model is not misled.
    assert!(again.text.contains("kept its existing"), "{}", again.text);
}
