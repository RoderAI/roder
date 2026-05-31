use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, bail};
use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::remote_runner::{
    RemoteRunnerProvider, RemoteRunnerProviderId, RemoteRunnerSession, RunnerCapabilities,
    RunnerCommandRequest, RunnerCommandResult, RunnerDestination, RunnerFileReadRequest,
    RunnerFileReadResult, RunnerFileWriteRequest, RunnerPortRequest, RunnerPortResult,
    RunnerSessionState, RunnerSnapshotRef,
};
use semver::Version;
use tokio::process::Command;

const PROVIDER_ID: &str = "unix-local";

#[derive(Debug, Default)]
pub struct UnixLocalRunnerExtension;

impl RoderExtension for UnixLocalRunnerExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-runner-unix-local".to_string(),
            name: "Unix Local Runner".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Runs remote-runner sessions against the local Unix workspace.".to_string(),
            ),
            provides: vec![ProvidedService::RemoteRunnerProvider(
                PROVIDER_ID.to_string(),
            )],
            required_capabilities: vec![
                CapabilityRequest::new("fs.readwrite.workspace"),
                CapabilityRequest::new("process.spawn.shell"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.remote_runner_provider(Arc::new(UnixLocalRunnerProvider::default()));
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct UnixLocalRunnerProvider {
    next_session_id: AtomicU64,
}

#[async_trait::async_trait]
impl RemoteRunnerProvider for UnixLocalRunnerProvider {
    fn id(&self) -> RemoteRunnerProviderId {
        PROVIDER_ID.to_string()
    }

    fn capabilities(&self) -> RunnerCapabilities {
        RunnerCapabilities {
            command_exec: true,
            file_read: true,
            file_write: true,
            port_preview: true,
            snapshots: false,
            cancellation: false,
            artifact_export: false,
            mounts: roder_api::remote_runner::RunnerMountCapabilities {
                provider_native: true,
                ..Default::default()
            },
        }
    }

    async fn create_session(
        &self,
        destination: RunnerDestination,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        let root = destination_root(&destination)?;
        let session_id = format!(
            "unix-local-{}",
            self.next_session_id.fetch_add(1, Ordering::SeqCst) + 1
        );
        Ok(Arc::new(UnixLocalRunnerSession {
            root,
            state: RunnerSessionState {
                provider_id: PROVIDER_ID.to_string(),
                session_id,
                destination_id: destination.id,
                snapshot: None,
                metadata: serde_json::json!({ "kind": "unix-local" }),
            },
        }))
    }

    async fn validate_destination(&self, destination: &RunnerDestination) -> anyhow::Result<()> {
        reject_remote_storage_mounts(&destination.default_manifest)
    }

    async fn resume_session(
        &self,
        state: RunnerSessionState,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        let root = state
            .metadata
            .get("root")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?);
        Ok(Arc::new(UnixLocalRunnerSession {
            root: canonical_root(root)?,
            state,
        }))
    }
}

fn reject_remote_storage_mounts(
    manifest: &roder_api::remote_runner::RunnerManifest,
) -> anyhow::Result<()> {
    for mount in &manifest.mounts {
        if !matches!(
            mount.intent.kind,
            roder_api::remote_runner::RunnerMountKind::ProviderNative
        ) {
            anyhow::bail!("unix-local runner only supports provider-native mounts");
        }
    }
    Ok(())
}

#[derive(Debug)]
struct UnixLocalRunnerSession {
    root: PathBuf,
    state: RunnerSessionState,
}

#[async_trait::async_trait]
impl RemoteRunnerSession for UnixLocalRunnerSession {
    fn state(&self) -> RunnerSessionState {
        let mut state = self.state.clone();
        state.metadata["root"] = serde_json::Value::String(self.root.display().to_string());
        state
    }

    async fn run_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        let cwd = match request.cwd.as_ref() {
            Some(cwd) => self.resolve_existing(cwd)?,
            None => self.root.clone(),
        };
        let output = Command::new(&request.program)
            .args(&request.args)
            .current_dir(cwd)
            .envs(request.env)
            .output()
            .await
            .with_context(|| format!("failed to run {}", request.program))?;
        Ok(RunnerCommandResult {
            command_id: request.command_id,
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }

    async fn read_file(
        &self,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        let path = self.resolve_existing(&request.path)?;
        let contents = tokio::fs::read(&path).await?;
        Ok(RunnerFileReadResult {
            path: self.display(&path).into(),
            contents,
        })
    }

    async fn write_file(&self, request: RunnerFileWriteRequest) -> anyhow::Result<()> {
        let path = self.resolve_for_write(&request.path)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, request.contents).await?;
        Ok(())
    }

    async fn expose_port(&self, request: RunnerPortRequest) -> anyhow::Result<RunnerPortResult> {
        Ok(RunnerPortResult {
            port: request.port,
            url: Some(format!("http://127.0.0.1:{}", request.port)),
        })
    }

    async fn snapshot(&self) -> anyhow::Result<Option<RunnerSnapshotRef>> {
        Ok(None)
    }

    async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

impl UnixLocalRunnerSession {
    fn resolve_existing(&self, input: &Path) -> anyhow::Result<PathBuf> {
        let candidate = self.candidate(input)?;
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("path does not exist: {}", input.display()))?;
        self.ensure_inside(&canonical)?;
        Ok(canonical)
    }

    fn resolve_for_write(&self, input: &Path) -> anyhow::Result<PathBuf> {
        let candidate = self.normalize(self.candidate(input)?)?;
        self.ensure_inside(&candidate)?;
        Ok(candidate)
    }

    fn candidate(&self, input: &Path) -> anyhow::Result<PathBuf> {
        if input.as_os_str().is_empty() {
            bail!("path is required");
        }
        if input.is_absolute() {
            Ok(input.to_path_buf())
        } else {
            Ok(self.root.join(input))
        }
    }

    fn ensure_inside(&self, path: &Path) -> anyhow::Result<()> {
        if !path.starts_with(&self.root) {
            bail!(
                "path {} is outside workspace {}",
                path.display(),
                self.root.display()
            );
        }
        Ok(())
    }

    fn normalize(&self, path: PathBuf) -> anyhow::Result<PathBuf> {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
                Component::RootDir => normalized.push(component.as_os_str()),
                Component::CurDir => {}
                Component::Normal(part) => normalized.push(part),
                Component::ParentDir => {
                    if !normalized.pop() {
                        bail!("path escapes workspace");
                    }
                }
            }
        }
        Ok(normalized)
    }

    fn display(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

fn destination_root(destination: &RunnerDestination) -> anyhow::Result<PathBuf> {
    let root = destination
        .config
        .get("root")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    canonical_root(root)
}

fn canonical_root(root: PathBuf) -> anyhow::Result<PathBuf> {
    root.canonicalize()
        .with_context(|| format!("workspace root does not exist: {}", root.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use roder_api::remote_runner::RunnerFileReadRequest;
    use roder_api::remote_runner::{RunnerFileWriteRequest, RunnerManifest};

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_local_runner_reads_writes_and_runs_commands_inside_workspace() {
        let root = test_workspace("unix-local-basic");
        let provider = UnixLocalRunnerProvider::default();
        let session = provider
            .create_session(RunnerDestination {
                id: "unix-local".to_string(),
                provider_id: PROVIDER_ID.to_string(),
                config: serde_json::json!({ "root": root }),
                default_manifest: RunnerManifest::default(),
            })
            .await
            .unwrap();

        session
            .write_file(RunnerFileWriteRequest {
                path: "src/main.rs".into(),
                contents: b"fn main() {}\n".to_vec(),
            })
            .await
            .unwrap();
        let read = session
            .read_file(RunnerFileReadRequest {
                path: "src/main.rs".into(),
            })
            .await
            .unwrap();
        assert_eq!(read.path, PathBuf::from("src/main.rs"));
        assert_eq!(read.contents, b"fn main() {}\n");

        let command = session
            .run_command(RunnerCommandRequest {
                command_id: "cmd-1".to_string(),
                program: "sh".to_string(),
                args: vec!["-lc".to_string(), "pwd && ls src".to_string()],
                cwd: None,
                env: Vec::new(),
            })
            .await
            .unwrap();
        assert_eq!(command.exit_code, Some(0));
        assert!(command.stdout.contains("main.rs"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn unix_local_runner_rejects_path_escapes() {
        let root = test_workspace("unix-local-guard");
        let provider = UnixLocalRunnerProvider::default();
        let session = provider
            .create_session(RunnerDestination {
                id: "unix-local".to_string(),
                provider_id: PROVIDER_ID.to_string(),
                config: serde_json::json!({ "root": root }),
                default_manifest: RunnerManifest::default(),
            })
            .await
            .unwrap();

        let err = session
            .write_file(RunnerFileWriteRequest {
                path: "../escape.txt".into(),
                contents: b"nope".to_vec(),
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("outside workspace"));

        let _ = std::fs::remove_dir_all(root);
    }

    fn test_workspace(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("roder-runner-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
