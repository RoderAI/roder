//! Spawning and reaping a stdio MCP server process.
//!
//! A local MCP server is often a launcher (`uvx`, `npx`) that starts an
//! interpreter, which in turn may start a browser. Killing only the direct
//! child would orphan the rest, so on Unix the server runs as the leader of
//! its own process group and every stop signals the whole group. A small
//! shell guard in that group also watches Roder's pid and signals the group
//! when Roder is gone, so a crash or `kill -9` of Roder does not leave the
//! server and its browser running.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use tokio::process::{Child, Command};

/// How to launch one stdio MCP server.
///
/// The child gets exactly `env` (the parent's environment is not
/// inherited), so secrets reach only the server they are meant for. Values
/// in `env` are never printed: `Debug` shows the variable names only.
#[derive(Clone)]
pub struct McpStdioServerConfig {
    /// Short identifier used in error messages.
    pub name: String,
    /// Absolute path of the program to run.
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The child's complete environment.
    pub env: BTreeMap<String, String>,
    /// Values scrubbed from anything this client reports (stderr tails,
    /// errors), typically the API keys placed in `env`.
    pub redact: Vec<String>,
    /// How long `initialize` may take, including a first-run package install.
    pub startup_timeout: Duration,
}

impl std::fmt::Debug for McpStdioServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpStdioServerConfig")
            .field("name", &self.name)
            .field("program", &self.program)
            .field("args", &self.args)
            .field("env", &self.env.keys().collect::<Vec<_>>())
            .field("redact", &format_args!("[{} values]", self.redact.len()))
            .field("startup_timeout", &self.startup_timeout)
            .finish()
    }
}

/// Replaces every non-empty `secret` in `text` with `[redacted]`.
pub fn redact_secrets(text: &str, secrets: &[String]) -> String {
    let mut text = text.to_string();
    for secret in secrets {
        let secret = secret.trim();
        if secret.len() >= 4 {
            text = text.replace(secret, "[redacted]");
        }
    }
    text
}

/// Guard run by `/bin/sh`: a background loop signals the whole process group
/// once the Roder pid (`$1`) is gone, then the shell `exec`s the server so the
/// server keeps the group-leader pid. The loop's stdio is detached so it never
/// holds the server's pipes open.
#[cfg(unix)]
const PARENT_GUARD: &str = "p=$1; shift; \
    ( while kill -0 \"$p\" 2>/dev/null; do sleep 1; done; kill -TERM 0 ) \
    </dev/null >/dev/null 2>&1 & exec \"$@\"";

/// Builds the command that launches `config`: on Unix wrapped in the parent
/// guard and placed in a new process group.
pub(crate) fn server_command(config: &McpStdioServerConfig) -> Command {
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(PARENT_GUARD)
            .arg("roder-mcp-guard")
            .arg(std::process::id().to_string())
            .arg(&config.program)
            .args(&config.args);
        command.process_group(0);
        command
    };
    #[cfg(not(unix))]
    let mut command = {
        let mut command = Command::new(&config.program);
        command.args(&config.args);
        command
    };
    command
        .env_clear()
        .envs(&config.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

pub(crate) fn spawn_server(config: &McpStdioServerConfig) -> anyhow::Result<Child> {
    server_command(config).spawn().with_context(|| {
        format!(
            "start MCP server {} ({})",
            config.name,
            config.program.display()
        )
    })
}

/// Signals the server's process group (or, off Unix, the child alone).
pub(crate) fn signal_group(pid: u32, force: bool) {
    #[cfg(unix)]
    {
        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
        // SAFETY: the server was spawned as the leader of its own process
        // group (`process_group(0)`), so a negative pid targets only that
        // group, never Roder's.
        unsafe {
            libc::kill(-(pid as libc::pid_t), signal);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, force);
    }
}

/// Stops the server: closes nothing itself (the caller drops stdin first),
/// waits `grace` for a clean exit, then terminates and finally kills the
/// whole process group.
pub(crate) async fn stop_child(child: &mut Child, grace: Duration) {
    // `Child::id` is `None` once the child is reaped, so take the group id
    // before waiting.
    let pid = child.id();
    if tokio::time::timeout(grace, child.wait()).await.is_ok() {
        // The leader exited; still sweep the group for stragglers such as a
        // browser the server launched or the parent guard.
        if let Some(pid) = pid {
            signal_group(pid, true);
        }
        return;
    }
    let Some(pid) = pid else {
        return;
    };
    signal_group(pid, false);
    if tokio::time::timeout(Duration::from_secs(3), child.wait())
        .await
        .is_err()
    {
        let _ = child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    }
    // Whatever ignored SIGTERM (or outlived the leader) goes now.
    signal_group(pid, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> McpStdioServerConfig {
        McpStdioServerConfig {
            name: "demo".into(),
            program: PathBuf::from("/usr/bin/true"),
            args: vec!["--mcp".into()],
            env: BTreeMap::from([("OPENAI_API_KEY".into(), "sk-secret-value".into())]),
            redact: vec!["sk-secret-value".into()],
            startup_timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn debug_output_names_env_vars_but_not_values() {
        let debug = format!("{:?}", config());
        assert!(debug.contains("OPENAI_API_KEY"), "{debug}");
        assert!(!debug.contains("sk-secret-value"), "{debug}");
    }

    #[test]
    fn redaction_scrubs_every_secret() {
        let text = "key=sk-secret-value and again sk-secret-value";
        let redacted = redact_secrets(text, &["sk-secret-value".into(), String::new()]);
        assert_eq!(redacted, "key=[redacted] and again [redacted]");
    }

    #[cfg(unix)]
    #[test]
    fn unix_command_runs_the_server_under_the_parent_guard() {
        let command = server_command(&config());
        let std = command.as_std();
        assert_eq!(std.get_program(), "/bin/sh");
        let args: Vec<_> = std
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "-c");
        assert_eq!(args[2], "roder-mcp-guard");
        assert_eq!(args[3], std::process::id().to_string());
        assert_eq!(&args[4..], ["/usr/bin/true", "--mcp"]);
    }
}
