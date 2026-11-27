//! Relaunch the TUI with a different agent backend (Ctrl+P > Agent backend).
//!
//! A backend is bound when the in-process app-server starts, so switching
//! re-runs this binary with the original arguments and a new `--backend`.

use std::process::Command;

/// Original CLI arguments with backend-specific selections removed and the new
/// backend appended. `--model` names a backend-owned model, `resume` targets a
/// backend-owned thread, and a transcript path would be overwritten.
pub(crate) fn relaunch_args(original: &[String], backend: &str) -> Vec<String> {
    let mut args = Vec::with_capacity(original.len() + 2);
    let mut i = 0;
    while i < original.len() {
        let arg = original[i].as_str();
        match arg {
            "--backend" | "--model" | "--record-api-transcript" => i += 1,
            "resume" => {
                if original
                    .get(i + 1)
                    .is_some_and(|value| !value.starts_with("--"))
                {
                    i += 1;
                }
            }
            _ if arg.starts_with("--backend=")
                || arg.starts_with("--model=")
                || arg.starts_with("--record-api-transcript=") => {}
            _ => args.push(original[i].clone()),
        }
        i += 1;
    }
    if backend != "roder" {
        args.push("--backend".to_string());
        args.push(backend.to_string());
    }
    args
}

/// Replace this process with Roder running on `backend`. Only returns on error.
pub(crate) fn relaunch(backend: &str) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let original = std::env::args().skip(1).collect::<Vec<_>>();
    let mut command = Command::new(exe);
    command.args(relaunch_args(&original, backend));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        Err(command.exec().into())
    }
    #[cfg(not(unix))]
    {
        let status = command.status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

#[cfg(test)]
mod tests {
    use super::relaunch_args;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn replaces_backend_and_drops_backend_owned_selections() {
        let original = args(&[
            "--config-dir",
            "/tmp/cfg",
            "--backend",
            "codex",
            "--model=gpt-6",
            "resume",
            "thread-1",
            "--yolo",
        ]);
        assert_eq!(
            relaunch_args(&original, "antigravity"),
            args(&[
                "--config-dir",
                "/tmp/cfg",
                "--yolo",
                "--backend",
                "antigravity"
            ])
        );
    }

    #[test]
    fn native_backend_needs_no_flag() {
        let original = args(&[
            "--backend=antigravity",
            "--model",
            "gemini",
            "resume",
            "--yolo",
        ]);
        assert_eq!(relaunch_args(&original, "roder"), args(&["--yolo"]));
    }
}
