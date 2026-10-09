//! Starts a run's `final_script.py` and reports how it ended.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::task::JoinHandle;

use crate::workspace::FINAL_SCRIPT_FILE;

/// How long to keep collecting output after the script has ended: a background process it started can hold
/// the pipes open for far longer than the script ran.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

pub(super) struct ScriptExecution {
    pub(super) exit_code: Option<i32>,
    pub(super) timed_out: bool,
    /// Set when the interpreter could not be started or awaited.
    pub(super) launch_error: Option<String>,
    /// Raw script output, not yet redacted.
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) elapsed: Duration,
}

/// Runs `interpreter final_script.py` in `run_dir`. The script is killed when `timeout` passes, and also when
/// this future is dropped (a cancelled turn), so it never outlives its run. Output written before a timeout
/// is kept.
pub(super) async fn execute_script(
    interpreter: &str,
    run_dir: &Path,
    timeout: Duration,
) -> ScriptExecution {
    let started = Instant::now();
    let mut child = match Command::new(interpreter)
        .arg(FINAL_SCRIPT_FILE)
        .current_dir(run_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return ScriptExecution {
                exit_code: None,
                timed_out: false,
                launch_error: Some(format!("could not run `{interpreter}`: {err}")),
                stdout: String::new(),
                stderr: String::new(),
                elapsed: started.elapsed(),
            };
        }
    };
    let stdout = child.stdout.take().map(capture);
    let stderr = child.stderr.take().map(capture);
    let (exit_code, timed_out, launch_error) =
        match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => (status.code(), false, None),
            Ok(Err(err)) => {
                let _ = child.kill().await;
                (
                    None,
                    false,
                    Some(format!("waiting for `{interpreter}` failed: {err}")),
                )
            }
            Err(_) => {
                let _ = child.kill().await;
                (None, true, None)
            }
        };
    let elapsed = started.elapsed();
    let deadline = tokio::time::Instant::now() + DRAIN_GRACE;
    ScriptExecution {
        exit_code,
        timed_out,
        launch_error,
        stdout: finish(stdout, deadline).await,
        stderr: finish(stderr, deadline).await,
        elapsed,
    }
}

/// Output read so far from one pipe.
struct Capture {
    bytes: Arc<Mutex<Vec<u8>>>,
    reader: JoinHandle<()>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

fn capture(mut pipe: impl AsyncRead + Unpin + Send + 'static) -> Capture {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&bytes);
    let reader = tokio::spawn(async move {
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(read) => sink
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .extend_from_slice(&chunk[..read]),
            }
        }
    });
    Capture { bytes, reader }
}

async fn finish(capture: Option<Capture>, deadline: tokio::time::Instant) -> String {
    let Some(mut capture) = capture else {
        return String::new();
    };
    if tokio::time::timeout_at(deadline, &mut capture.reader)
        .await
        .is_err()
    {
        capture.reader.abort();
    }
    let bytes = std::mem::take(
        &mut *capture
            .bytes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    String::from_utf8_lossy(&bytes).into_owned()
}
