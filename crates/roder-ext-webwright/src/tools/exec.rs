//! Starts a run's `final_script.py` and reports how it ended.

use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

use crate::workspace::FINAL_SCRIPT_FILE;

/// How long to keep collecting output after the script has ended: a background process it started can hold
/// the pipes open for far longer than the script ran.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// How much of one pipe is kept: its first `HEAD_BYTES` and its last `TAIL_BYTES`. A script that prints in a
/// loop for its whole timeout must not be able to fill this process's memory, and the start (the root cause)
/// and the end (where it stopped) are what a failed run is read for.
const HEAD_BYTES: usize = 256 * 1024;
const TAIL_BYTES: usize = 768 * 1024;

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

/// Runs `interpreter final_script.py` in `run_dir`. The script, with the processes it started, is killed when
/// `timeout` passes, and also when this future is dropped (a cancelled turn), so it never outlives its run.
/// Output written before a timeout is kept, up to `HEAD_BYTES` + `TAIL_BYTES` per stream.
pub(super) async fn execute_script(
    interpreter: &str,
    run_dir: &Path,
    timeout: Duration,
) -> ScriptExecution {
    let started = Instant::now();
    let mut command = Command::new(interpreter);
    command
        .arg(FINAL_SCRIPT_FILE)
        .current_dir(run_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // A group of its own, so one signal reaches everything the script starts (see `ProcessGroup`).
    #[cfg(unix)]
    command.process_group(0);
    let mut child = match command.spawn() {
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
    let mut group = ProcessGroup::of(&child);
    let stdout = child.stdout.take().map(capture);
    let stderr = child.stderr.take().map(capture);
    let (exit_code, timed_out, launch_error) =
        match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => {
                group.release();
                (status.code(), false, None)
            }
            Ok(Err(err)) => {
                group.kill();
                let _ = child.kill().await;
                (
                    None,
                    false,
                    Some(format!("waiting for `{interpreter}` failed: {err}")),
                )
            }
            Err(_) => {
                group.kill();
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

/// The script's process group. Killing the interpreter alone would leave whatever it started (a Playwright
/// driver, a `cmd &` helper) running, so a timeout or a cancelled call signals the whole group.
///
/// Processes that moved themselves into another group or session are out of reach; they have to notice
/// their parent going away.
///
/// The group is released as soon as the interpreter has exited and been reaped: from then on its id may be
/// reused, and a script that exited on its own and left a helper behind is not this guard's concern.
struct ProcessGroup {
    leader: Option<u32>,
}

impl ProcessGroup {
    fn of(child: &Child) -> Self {
        Self { leader: child.id() }
    }

    fn release(&mut self) {
        self.leader = None;
    }

    fn kill(&mut self) {
        if let Some(leader) = self.leader.take() {
            kill_group(leader);
        }
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(unix)]
fn kill_group(leader: u32) {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    // SIGKILL is 9 on every Unix Roder runs on.
    const SIGKILL: i32 = 9;
    // Group 0 is Roder's own and 1 is init's: never a script's.
    let Ok(leader) = i32::try_from(leader) else {
        return;
    };
    if leader <= 1 {
        return;
    }
    // SAFETY: the script was spawned with `process_group(0)`, so `leader` is the id of a group of its own and
    // `-leader` addresses that group and no other. The caller has not yet reaped the leader, so the id has not
    // been reused.
    unsafe {
        kill(-leader, SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(_leader: u32) {}

/// What one pipe kept: its first `head_cap` bytes and its last `tail_cap` bytes.
struct BoundedOutput {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    head_cap: usize,
    tail_cap: usize,
    /// Bytes pushed out of the tail window.
    evicted: u64,
    /// The last byte pushed out of the tail window; a newline means the tail starts at a line start.
    last_evicted: Option<u8>,
}

impl BoundedOutput {
    fn new(head_cap: usize, tail_cap: usize) -> Self {
        Self {
            head: Vec::new(),
            tail: VecDeque::new(),
            head_cap,
            tail_cap,
            evicted: 0,
            last_evicted: None,
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        let room = self.head_cap - self.head.len();
        let (head_part, rest) = chunk.split_at(room.min(chunk.len()));
        self.head.extend_from_slice(head_part);
        self.tail.extend(rest);
        let excess = self.tail.len().saturating_sub(self.tail_cap);
        if excess > 0 {
            self.last_evicted = self.tail.get(excess - 1).copied();
            self.tail.drain(..excess);
            self.evicted += excess as u64;
        }
    }

    /// The kept output. When bytes were dropped the two ends are cut at line boundaries (the redactor works
    /// line by line, and a line cut in two can lose the keyword that marks it sensitive) and an omission
    /// marker line says how many bytes are missing.
    fn into_text(self) -> String {
        if self.evicted == 0 {
            let mut bytes = self.head;
            bytes.extend(self.tail);
            return lossy(bytes);
        }
        let mut omitted = self.evicted;
        let mut bytes = self.head;
        if let Some(newline) = bytes.iter().rposition(|byte| *byte == b'\n') {
            omitted += (bytes.len() - newline - 1) as u64;
            bytes.truncate(newline + 1);
        }
        let mut tail = Vec::from(self.tail);
        if self.last_evicted != Some(b'\n') {
            let skip = tail
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(tail.len(), |newline| newline + 1);
            omitted += skip as u64;
            tail.drain(..skip);
        }
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        bytes.extend_from_slice(
            format!("[... {omitted} bytes of script output omitted ...]\n").as_bytes(),
        );
        bytes.extend(tail);
        lossy(bytes)
    }
}

fn lossy(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes)
        .unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned())
}

/// Output read so far from one pipe.
struct Capture {
    output: Arc<Mutex<BoundedOutput>>,
    reader: JoinHandle<()>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

fn capture(mut pipe: impl AsyncRead + Unpin + Send + 'static) -> Capture {
    let output = Arc::new(Mutex::new(BoundedOutput::new(HEAD_BYTES, TAIL_BYTES)));
    let sink = Arc::clone(&output);
    let reader = tokio::spawn(async move {
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(read) => sink
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(&chunk[..read]),
            }
        }
    });
    Capture { output, reader }
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
    let output = std::mem::replace(
        &mut *capture
            .output
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        BoundedOutput::new(HEAD_BYTES, TAIL_BYTES),
    );
    output.into_text()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `lines` numbered lines of 8 bytes each: `line 00\n`.
    fn numbered_lines(lines: usize) -> Vec<u8> {
        (0..lines)
            .flat_map(|n| format!("line {n:02}\n").into_bytes())
            .collect()
    }

    fn pushed(head_cap: usize, tail_cap: usize, bytes: &[u8], chunk: usize) -> String {
        let mut output = BoundedOutput::new(head_cap, tail_cap);
        for piece in bytes.chunks(chunk) {
            output.push(piece);
        }
        output.into_text()
    }

    #[test]
    fn output_under_the_caps_is_unchanged() {
        let bytes = numbered_lines(5);
        // 40 bytes fit in 16 + 24, whatever the chunking.
        for chunk in [1, 3, 7, 40] {
            assert_eq!(
                pushed(16, 24, &bytes, chunk),
                String::from_utf8(bytes.clone()).unwrap(),
                "chunk {chunk}"
            );
        }
    }

    #[test]
    fn cut_on_line_boundaries_keeps_both_ends_and_counts_what_was_dropped() {
        // 160 bytes; the head keeps lines 00-01, the tail lines 16-19, and 112 bytes are in between.
        let text = pushed(16, 32, &numbered_lines(20), 8192);
        assert_eq!(
            text,
            "line 00\nline 01\n[... 112 bytes of script output omitted ...]\nline 16\nline 17\nline 18\nline 19\n"
        );
    }

    #[test]
    fn a_line_cut_in_two_is_dropped_whole_and_counted() {
        // The head ends inside line 02 and the tail starts inside line 16: 4 + 110 + 6 bytes are omitted.
        for chunk in [1, 5, 64, 8192] {
            let text = pushed(20, 30, &numbered_lines(20), chunk);
            assert_eq!(
                text,
                "line 00\nline 01\n[... 120 bytes of script output omitted ...]\nline 17\nline 18\nline 19\n",
                "chunk {chunk}"
            );
        }
    }

    #[test]
    fn the_result_never_exceeds_the_caps_plus_the_marker() {
        let bytes = numbered_lines(2000);
        let marker = "[... 99999 bytes of script output omitted ...]\n".len();
        let text = pushed(100, 300, &bytes, 37);
        assert!(text.len() <= 100 + 300 + marker, "{} bytes", text.len());
        assert!(text.starts_with("line 00\n"));
        assert!(text.ends_with("line 1999\n"));
    }

    #[test]
    fn a_tail_of_one_partial_line_is_dropped_rather_than_shown_without_its_start() {
        // No newline anywhere: nothing in the tail is known to start a line.
        let text = pushed(4, 8, &[b'x'; 40], 8192);
        assert_eq!(text, "xxxx\n[... 36 bytes of script output omitted ...]\n");
    }

    #[test]
    fn multi_byte_characters_at_a_cut_do_not_panic_or_leave_replacement_characters() {
        // `éé\n` is 5 bytes; the head cap falls inside an `é` and so does the start of the tail.
        let bytes = "éé\néé\néé\néé\n".as_bytes();
        let text = pushed(6, 7, bytes, 8192);
        assert_eq!(
            text,
            "éé\n[... 10 bytes of script output omitted ...]\néé\n"
        );
        // A cut inside a character with no newline to fall back on stays valid text.
        let text = pushed(5, 5, "ééééééé".as_bytes(), 3);
        assert!(text.contains("bytes of script output omitted"), "{text}");
    }
}
