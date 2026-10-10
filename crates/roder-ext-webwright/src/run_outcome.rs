//! How a `webwright.run_script` run ended, decided by code: an outcome class, the record kept beside the run
//! so verification can gate on it, and the text the model reads when a run fails.

use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// Written into a run directory by `webwright.run_script`; absent for runs it did not execute.
pub(crate) const RUN_EXIT_FILE: &str = "run_exit.json";

const STDERR_TAIL_BYTES: usize = 4096;
const STDOUT_TAIL_BYTES: usize = 2048;
const FIRST_ERROR_CHARS: usize = 300;
/// Heads every line that quotes the script's output, so the model never reads that text as an instruction.
const UNTRUSTED_OUTPUT: &str = "untrusted script output, secrets redacted";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunOutcome {
    /// Written before the script starts; still there when the run was interrupted.
    Running,
    Ok,
    NonzeroExit,
    /// No exit code and no timeout: the script was ended by a signal.
    Signaled,
    Timeout,
    /// The interpreter could not be started or awaited.
    LaunchFailed,
}

impl RunOutcome {
    /// Classifies a script that ran: launch failures are decided by the caller.
    pub(crate) fn finished(exit_code: Option<i32>, timed_out: bool) -> Self {
        match (timed_out, exit_code) {
            (true, _) => Self::Timeout,
            (false, Some(0)) => Self::Ok,
            (false, Some(_)) => Self::NonzeroExit,
            (false, None) => Self::Signaled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunExitRecord {
    pub(crate) outcome: RunOutcome,
    pub(crate) exit_code: Option<i32>,
    pub(crate) elapsed_ms: u64,
}

impl RunExitRecord {
    pub(crate) fn running() -> Self {
        Self {
            outcome: RunOutcome::Running,
            exit_code: None,
            elapsed_ms: 0,
        }
    }

    /// What went wrong, for a verification message.
    pub(crate) fn failure(&self) -> String {
        match self.outcome {
            RunOutcome::Ok => "exit code 0".to_string(),
            RunOutcome::NonzeroExit => format!(
                "nonzero_exit (exit code {})",
                self.exit_code
                    .map_or_else(|| "unknown".to_string(), |code| code.to_string())
            ),
            RunOutcome::Signaled => "signaled (ended without an exit code)".to_string(),
            RunOutcome::Timeout => "timeout (the script was killed)".to_string(),
            RunOutcome::LaunchFailed => {
                "launch_failed (the interpreter could not be run)".to_string()
            }
            RunOutcome::Running => {
                "running (the run was interrupted before it finished)".to_string()
            }
        }
    }
}

pub(crate) fn write_exit_record(run_dir: &Path, record: &RunExitRecord) -> anyhow::Result<()> {
    let path = run_dir.join(RUN_EXIT_FILE);
    fs::write(&path, serde_json::to_string_pretty(record)?)
        .with_context(|| format!("write {}", path.display()))
}

pub(crate) fn read_exit_record(run_dir: &Path) -> anyhow::Result<Option<RunExitRecord>> {
    let path = run_dir.join(RUN_EXIT_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text)
        .map(Some)
        .with_context(|| format!("parse {}", path.display()))
}

/// A guess at the cause from well-known message text. Python and Playwright own that wording and change it
/// between versions, so this is only a hint and the raw tail always travels with it.
pub(crate) fn stderr_hint(stderr: &str) -> Option<&'static str> {
    const HINTS: &[(&str, &[&str])] = &[
        (
            "missing_python_module",
            &["ModuleNotFoundError", "No module named"],
        ),
        (
            "missing_browser_binary",
            &["Executable doesn't exist", "playwright install"],
        ),
        ("python_syntax_error", &["SyntaxError", "IndentationError"]),
        ("navigation_failed", &["net::ERR_", "NS_ERROR_"]),
        ("timeout_error", &["TimeoutError"]),
        ("python_exception", &["Traceback (most recent call last)"]),
    ];
    HINTS
        .iter()
        .find(|(_, needles)| needles.iter().any(|needle| stderr.contains(needle)))
        .map(|(hint, _)| *hint)
}

/// The first unindented stderr line that names an error or exception, else the last non-empty stderr line;
/// none when stderr is empty. Indented lines are traceback frames and call logs.
pub(crate) fn first_error_line(stderr: &str) -> Option<String> {
    let line = stderr
        .lines()
        .find(|line| {
            !line.starts_with(char::is_whitespace)
                && (line.contains("Error") || line.contains("Exception"))
        })
        .or_else(|| stderr.lines().rev().find(|line| !line.trim().is_empty()))?;
    let line = line.trim();
    if line.chars().count() > FIRST_ERROR_CHARS {
        let cut = line.chars().take(FIRST_ERROR_CHARS).collect::<String>();
        Some(format!("{cut}..."))
    } else {
        Some(line.to_string())
    }
}

/// The last `max_bytes` of `text`, starting at a line start when it has to be cut.
pub(crate) fn tail(text: &str, max_bytes: usize) -> &str {
    let text = text.trim_end();
    if text.len() <= max_bytes {
        return text;
    }
    let mut start = text.len() - max_bytes;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let cut = &text[start..];
    if text.as_bytes()[start - 1] == b'\n' {
        return cut;
    }
    match cut.find('\n') {
        Some(newline) => &cut[newline + 1..],
        None => cut,
    }
}

pub(crate) struct RunReport<'a> {
    pub(crate) run_id: u32,
    pub(crate) outcome: RunOutcome,
    pub(crate) exit_code: Option<i32>,
    pub(crate) elapsed: Duration,
    pub(crate) timeout_seconds: u64,
    /// Redacted script output. For `LaunchFailed`, `stderr` is the launch error.
    pub(crate) stdout: &'a str,
    pub(crate) stderr: &'a str,
}

/// The `webwright.run_script` result text: one line when the run is ok, else the outcome class, a hint, the
/// first error line and the end of the output, so the next call can fix the script instead of asking again.
/// The outcome class and the hint come from code; the first error line and the tail quote the script's output
/// and each carry the untrusted label themselves.
pub(crate) fn render_text(report: &RunReport<'_>) -> String {
    let headline = headline(report);
    if report.outcome == RunOutcome::Ok {
        return headline;
    }
    let mut lines = vec![headline];
    if report.outcome != RunOutcome::LaunchFailed {
        if let Some(hint) = stderr_hint(report.stderr) {
            lines.push(format!(
                "hint: {hint} (guessed from the stderr text; trust the tail over this label)"
            ));
        }
        if let Some(line) = first_error_line(report.stderr) {
            lines.push(format!("first error ({UNTRUSTED_OUTPUT}): {line}"));
        }
        lines.push(output_section(report));
    }
    lines.push(
        "next: fix final_script.py and call webwright.run_script again (every call allocates a new run); webwright.read_log_tail shows final_script_log.txt."
            .to_string(),
    );
    lines.join("\n")
}

fn headline(report: &RunReport<'_>) -> String {
    let id = format!("webwright run_{:03}", report.run_id);
    let elapsed = format!("{:.1}s", report.elapsed.as_secs_f64());
    match report.outcome {
        RunOutcome::Ok => format!("{id} ok: exit code 0 in {elapsed}"),
        RunOutcome::NonzeroExit => format!(
            "{id} failed (nonzero_exit): exit code {} after {elapsed}",
            report
                .exit_code
                .map_or_else(|| "unknown".to_string(), |code| code.to_string())
        ),
        RunOutcome::Signaled => {
            format!("{id} failed (signaled): ended without an exit code after {elapsed}")
        }
        RunOutcome::Timeout => format!(
            "{id} failed (timeout): killed after {elapsed} (timeoutSeconds={})",
            report.timeout_seconds
        ),
        RunOutcome::LaunchFailed => format!("{id} failed (launch_failed): {}", report.stderr),
        RunOutcome::Running => format!("{id} did not finish"),
    }
}

fn output_section(report: &RunReport<'_>) -> String {
    if !report.stderr.trim().is_empty() {
        section("stderr", "", report.stderr, STDERR_TAIL_BYTES)
    } else if !report.stdout.trim().is_empty() {
        section(
            "stdout",
            "stderr was empty; ",
            report.stdout,
            STDOUT_TAIL_BYTES,
        )
    } else {
        "no stderr or stdout output".to_string()
    }
}

fn section(label: &str, note: &str, text: &str, max_bytes: usize) -> String {
    let text = text.trim_end();
    let shown = tail(text, max_bytes);
    let size = if shown.len() < text.len() {
        format!("last {} of {} bytes", shown.len(), text.len())
    } else {
        format!("{} bytes", text.len())
    };
    format!("{label} tail ({note}{size}; {UNTRUSTED_OUTPUT}):\n{shown}")
}
