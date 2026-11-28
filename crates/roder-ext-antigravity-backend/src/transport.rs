use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

pub(crate) const LOGIN_HELP: &str = "Antigravity subscription authentication failed. Run `roder auth login antigravity` \
     in a terminal on this machine, then retry.";
pub(crate) const RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// Select the official CLI's file-backed OAuth flow consistently for login and
/// agent sessions, without ambient API keys or cloud credentials overriding it.
pub fn clear_api_key_environment(command: &mut Command) {
    command.env("AGY_ACP_FORCE_FILE_STORAGE", "1").env(
        "SSH_CONNECTION",
        std::env::var("SSH_CONNECTION")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "127.0.0.1 0 127.0.0.1 0".into()),
    );
    for name in [
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "GOOGLE_CLOUD_PROJECT",
        "GOOGLE_CLOUD_LOCATION",
        "GOOGLE_CLOUD_QUOTA_PROJECT",
        "GOOGLE_GENAI_USE_VERTEXAI",
        "GOOGLE_GENAI_USE_GCA",
        "GCLOUD_PROJECT",
        "CLOUDSDK_CORE_PROJECT",
        "AGY_ACP_CCPA_PROJECT",
        "AGY_ACP_ENABLE_OAUTH",
    ] {
        command.env_remove(name);
    }
}

pub(crate) struct Transport {
    child: Child,
    input: ChildStdin,
    receiver: mpsc::Receiver<anyhow::Result<Value>>,
    reader: tokio::task::JoinHandle<()>,
    next_id: u64,
    scratch: Option<Scratch>,
}

// agy's launcher extracts its runtime below TMPDIR. An owned private directory
// prevents cancelled startup probes from accumulating extracted runtimes.
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    pub(crate) fn create() -> anyhow::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "roder-agy-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        bail!("Could not create a private Antigravity runtime directory")
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Transport {
    pub(crate) async fn spawn(
        program: &Path,
        cwd: &Path,
        native_bypass: bool,
    ) -> anyhow::Result<Self> {
        let scratch = Scratch::create()?;
        let mut command = Command::new(program);
        clear_api_key_environment(&mut command);
        command.arg("acp");
        if native_bypass {
            command.arg("--dangerously-skip-permissions");
        }
        // agy uses SSH detection to require the explicit terminal login flow.
        command
            .env("BROWSER", "false")
            .env("TMPDIR", scratch.path())
            .env("TMP", scratch.path())
            .env("TEMP", scratch.path())
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = roder_api::spawn::spawn_retrying_busy(&mut command)
            .await
            .with_context(|| {
                format!(
                    "start {} acp; install the official Antigravity CLI",
                    program.display()
                )
            })?;
        let input = child
            .stdin
            .take()
            .context("Antigravity stdin unavailable")?;
        let output = child
            .stdout
            .take()
            .context("Antigravity stdout unavailable")?;
        let (sender, receiver) = mpsc::channel(256);
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(output).lines();
            loop {
                let message = match lines.next_line().await {
                    Ok(Some(line)) => serde_json::from_str(&line).map_err(|_| {
                        // Never echo a non-protocol line: login URLs may carry secrets.
                        anyhow::anyhow!("Invalid Antigravity ACP output. {LOGIN_HELP}")
                    }),
                    Ok(None) => break,
                    Err(_) => Err(anyhow::anyhow!("Antigravity ACP output closed")),
                };
                let failed = message.is_err();
                if sender.send(message).await.is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            receiver,
            reader,
            next_id: 1,
            scratch: Some(scratch),
        })
    }

    pub(crate) async fn write(&mut self, message: Value) -> anyhow::Result<()> {
        let mut bytes = serde_json::to_vec(&message)?;
        bytes.push(b'\n');
        tokio::time::timeout(RPC_TIMEOUT, async {
            self.input.write_all(&bytes).await?;
            self.input.flush().await
        })
        .await
        .context("Antigravity ACP write timed out")??;
        Ok(())
    }

    pub(crate) async fn request(&mut self, method: &str, params: Value) -> anyhow::Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        Ok(id)
    }

    pub(crate) async fn notification(&mut self, method: &str, params: Value) -> anyhow::Result<()> {
        self.write(json!({"jsonrpc":"2.0","method":method,"params":params}))
            .await
    }

    pub(crate) async fn receive(&mut self) -> anyhow::Result<Value> {
        self.receiver
            .recv()
            .await
            .context("Antigravity ACP process exited")?
    }

    pub(crate) async fn stop(&mut self) -> anyhow::Result<()> {
        self.reader.abort();
        self.kill_group();
        if self.child.try_wait()?.is_none() {
            self.child.kill().await?;
        }
        self.child.wait().await?;
        self.scratch.take();
        Ok(())
    }

    fn kill_group(&self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            // This child was made its own process group before exec. Kill workers
            // as well as the launcher; no other user's process shares this group.
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.reader.abort();
        self.kill_group();
        let _ = self.child.start_kill();
    }
}

pub(crate) fn result(message: Value, method: &str) -> anyhow::Result<Value> {
    if let Some(error) = message.get("error") {
        let code = error["code"].as_i64().unwrap_or_default();
        if method == "authenticate" || code == -32000 || code == -32001 {
            bail!("{LOGIN_HELP}");
        }
        // Error data can include signed URLs or credential material.
        bail!("Antigravity ACP {method} failed (code {code})");
    }
    message
        .get("result")
        .cloned()
        .context("Antigravity ACP response missing result")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_environment_clears_ambient_auth_and_matches_login_storage() {
        let mut command = Command::new("agy");
        clear_api_key_environment(&mut command);
        let env = command
            .as_std()
            .get_envs()
            .collect::<std::collections::HashMap<_, _>>();
        for name in [
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "GOOGLE_CLOUD_PROJECT",
            "GOOGLE_CLOUD_LOCATION",
            "GOOGLE_CLOUD_QUOTA_PROJECT",
            "GOOGLE_GENAI_USE_VERTEXAI",
            "GOOGLE_GENAI_USE_GCA",
            "GCLOUD_PROJECT",
            "CLOUDSDK_CORE_PROJECT",
            "AGY_ACP_CCPA_PROJECT",
            "AGY_ACP_ENABLE_OAUTH",
        ] {
            assert_eq!(env.get(std::ffi::OsStr::new(name)), Some(&None), "{name}");
        }
        assert_eq!(
            env.get(std::ffi::OsStr::new("AGY_ACP_FORCE_FILE_STORAGE")),
            Some(&Some(std::ffi::OsStr::new("1"))),
        );
        assert!(
            env.get(std::ffi::OsStr::new("SSH_CONNECTION"))
                .and_then(|value| *value)
                .is_some_and(|value| !value.is_empty())
        );
        assert!(!env.contains_key(std::ffi::OsStr::new("BROWSER")));
    }

    #[test]
    fn scratch_directory_is_private_and_owned() {
        let scratch = Scratch::create().unwrap();
        let path = scratch.0.clone();
        assert!(path.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        drop(scratch);
        assert!(!path.exists());
    }
}
