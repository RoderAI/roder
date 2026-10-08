use crate::CuaConfig;
use async_trait::async_trait;
use roder_api::remote_runner::{RemoteWorkspace, RunnerCommandRequest, RunnerFileReadRequest};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub struct DriverReply {
    pub observation: Value,
    pub is_error: bool,
}

#[async_trait]
pub trait CuaTransport: Send + Sync + 'static {
    async fn call(
        &self,
        workspace: &RemoteWorkspace,
        tool: &str,
        arguments: Value,
        timeout_ms: u64,
    ) -> anyhow::Result<DriverReply>;
}

pub struct RunnerCuaTransport {
    program: String,
}
impl RunnerCuaTransport {
    pub fn new(config: &CuaConfig) -> Self {
        Self {
            program: config.program.clone(),
        }
    }
}

/// Cancel the runner command if its owning tool future is dropped. The driver
/// exposes only atomic input operations; an uncertain action is never replayed.
struct CommandGuard {
    session: Arc<dyn roder_api::remote_runner::RemoteRunnerSession>,
    id: String,
    armed: bool,
}
impl Drop for CommandGuard {
    fn drop(&mut self) {
        if self.armed
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let session = self.session.clone();
            let id = self.id.clone();
            runtime.spawn(async move {
                let _ =
                    tokio::time::timeout(Duration::from_secs(5), session.cancel_command(&id)).await;
            });
        }
    }
}

#[async_trait]
impl CuaTransport for RunnerCuaTransport {
    async fn call(
        &self,
        workspace: &RemoteWorkspace,
        tool: &str,
        arguments: Value,
        timeout_ms: u64,
    ) -> anyhow::Result<DriverReply> {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let id = format!("cua-{nonce}");
        let path = format!("/tmp/roder-cua-results/{nonce}.json");
        let _file_guard = ResultFileGuard {
            session: workspace.session.clone(),
            path: path.clone(),
            nonce: nonce.clone(),
        };
        let mut guard = CommandGuard {
            session: workspace.session.clone(),
            id: id.clone(),
            armed: true,
        };
        let result = tokio::time::timeout(Duration::from_millis(timeout_ms), workspace.session.run_command(RunnerCommandRequest {
            command_id: id,
            program: self.program.clone(),
            args: vec![tool.into(), arguments.to_string(), path.clone()],
            cwd: None,
            env: vec![],
            timeout_ms: Some(timeout_ms),
        })).await.map_err(|_| anyhow::anyhow!("Cua command timed out; input outcome may be uncertain. Observe before deciding another action."))?
            .map_err(|_| anyhow::anyhow!("Cua runner command failed; input outcome may be uncertain. Check runner lifecycle and observe before continuing."))?;
        guard.armed = false;
        anyhow::ensure!(
            result.exit_code == Some(0),
            "Cua launcher failed. Install/recover the pinned driver in this runner; no input retry was attempted."
        );
        anyhow::ensure!(result.stdout.len() <= 4096, "invalid Cua launcher envelope");
        let envelope: Value = serde_json::from_str(&result.stdout)
            .map_err(|_| anyhow::anyhow!("invalid Cua launcher envelope"))?;
        anyhow::ensure!(
            envelope["result_file"].as_str() == Some(path.as_str()),
            "Cua returned a foreign result path"
        );
        // Blaxel process output is capped at 64 KiB. Read the complete bounded
        // response through the existing authenticated runner filesystem API.
        let file = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            workspace
                .session
                .read_file(RunnerFileReadRequest { path: path.into() }),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Cua result read timed out"))??;
        anyhow::ensure!(
            file.contents.len() <= 12 * 1024 * 1024,
            "Cua response exceeds 12 MiB"
        );
        let mut wire: Value = serde_json::from_slice(&file.contents)
            .map_err(|_| anyhow::anyhow!("invalid Cua result file"))?;
        anyhow::ensure!(
            wire["driver_version"].as_str() == Some(crate::DRIVER_VERSION),
            "unsupported Cua Driver version; expected {}",
            crate::DRIVER_VERSION
        );
        let is_error = wire["is_error"]
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("missing Cua status"))?;
        let observation = wire
            .get_mut("observation")
            .ok_or_else(|| anyhow::anyhow!("missing Cua observation"))?
            .take();
        anyhow::ensure!(observation.is_object(), "Cua observation must be an object");
        Ok(DriverReply {
            observation,
            is_error,
        })
    }
}

struct ResultFileGuard {
    session: Arc<dyn roder_api::remote_runner::RemoteRunnerSession>,
    path: String,
    nonce: String,
}
impl Drop for ResultFileGuard {
    fn drop(&mut self) {
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let session = self.session.clone();
            let path = self.path.clone();
            let nonce = self.nonce.clone();
            runtime.spawn(async move {
                let _ = tokio::time::timeout(
                    Duration::from_secs(5),
                    session.run_command(RunnerCommandRequest {
                        command_id: format!("cua-clean-{nonce}"),
                        program: "/bin/rm".into(),
                        args: vec![
                            "-f".into(),
                            "--".into(),
                            path.clone(),
                            path.replace(".json", ".tmp"),
                        ],
                        cwd: None,
                        env: vec![],
                        timeout_ms: Some(5000),
                    }),
                )
                .await;
            });
        }
    }
}
