//! `JEV_SESSION_LOG`: one JSON line per `jev_browse` call, for grading a
//! run afterwards. `roder exec --json` carries only a tool's text, so the
//! trace, the controls and the page facts would otherwise be lost.
//!
//! Off unless `JEV_SESSION_LOG` names a directory. Each call appends to
//! `<dir>/<thread>.jsonl`: when, the thread, the call's number, the request
//! (goal, url, tab, foreground), the tab it used, the full result data and
//! the text the caller read. Typed secrets are already scrubbed from the
//! data; the goal goes wherever the transcript goes.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The log directory, when logging is on.
pub(crate) fn dir() -> Option<PathBuf> {
    std::env::var_os("JEV_SESSION_LOG")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

/// Append one call to `dir`'s log for `thread`. Best effort: a log that
/// cannot be written never fails the call.
pub(crate) fn append(dir: &Path, thread: &str, arguments: &Value, data: &Value, text: &str) {
    let line = json!({
        "at": chrono::Local::now().to_rfc3339(),
        "thread": thread,
        "call": data["session"]["call"],
        "request": {
            "goal": arguments["goal"],
            "url": arguments["url"],
            "tab": arguments["tab"],
            "foreground": arguments["foreground"],
        },
        "tab": {
            "id": data["session"]["tab"],
            "note": data["session"]["tab_note"],
            "tabs_open": data["session"]["tabs_open"],
        },
        "result": data,
        "digest": text,
    });
    let _ = std::fs::create_dir_all(dir).and_then(|()| {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(format!("{}.jsonl", file_name(thread))))?;
        writeln!(file, "{line}")
    });
}

/// A thread id as a file name: anything but letters, digits, `-` and `_`
/// becomes `_`.
fn file_name(thread: &str) -> String {
    let name = thread
        .chars()
        .map(
            |c| match c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                true => c,
                false => '_',
            },
        )
        .collect::<String>();
    match name.is_empty() {
        true => "thread".into(),
        false => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_call_appends_one_line_to_its_threads_file() {
        let dir = std::env::temp_dir().join(format!("jev-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let arguments = json!({"goal": "Search", "url": "", "tab": "current"});
        let data = json!({"status": "done", "actions": [{"step": 1, "effect": "went to x"}],
            "session": {"call": 2, "tab": "t1", "tab_note": "continued", "tabs_open": 1}});
        append(&dir, "../thread 1", &arguments, &data, "Jev: done.");
        append(&dir, "../thread 1", &arguments, &data, "Jev: done.");
        let written = std::fs::read_to_string(dir.join("___thread_1.jsonl")).unwrap();
        let lines = written.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let line: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(line["call"], json!(2));
        assert_eq!(line["request"]["goal"], json!("Search"));
        assert_eq!(line["tab"]["note"], json!("continued"));
        assert_eq!(line["result"]["actions"][0]["effect"], json!("went to x"));
        assert_eq!(line["digest"], json!("Jev: done."));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(file_name(""), "thread");
    }
}
