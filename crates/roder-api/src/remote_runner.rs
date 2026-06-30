use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub type RemoteRunnerProviderId = String;
pub type RemoteRunnerSessionId = String;
pub type RunnerDestinationId = String;
pub type RunnerCommandId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerCapabilities {
    pub command_exec: bool,
    pub file_read: bool,
    pub file_write: bool,
    pub port_preview: bool,
    pub snapshots: bool,
    pub cancellation: bool,
    #[serde(default)]
    pub artifact_export: bool,
    #[serde(default)]
    pub mounts: RunnerMountCapabilities,
    /**
     * Provider can transition a live session toward a paused/standby state and
     * later resume it without losing the session's filesystem/process state.
     */
    #[serde(default)]
    pub pausable: bool,
    /**
     * Provider can detach a session (releasing the local handle while keeping
     * the remote sandbox alive) and later rejoin it from persisted state.
     */
    #[serde(default)]
    pub detachable: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerMountCapabilities {
    #[serde(default)]
    pub s3: bool,
    #[serde(default)]
    pub gcs: bool,
    #[serde(default)]
    pub r2: bool,
    #[serde(default)]
    pub azure_blob: bool,
    #[serde(default)]
    pub box_storage: bool,
    #[serde(default)]
    pub provider_native: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerDestination {
    pub id: RunnerDestinationId,
    pub provider_id: RemoteRunnerProviderId,
    #[serde(default)]
    pub config: serde_json::Value,
    #[serde(default)]
    pub default_manifest: RunnerManifest,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerManifest {
    #[serde(default)]
    pub entries: Vec<RunnerManifestEntry>,
    #[serde(default)]
    pub mounts: Vec<RunnerMount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerManifestEntry {
    pub source: PathBuf,
    pub target: PathBuf,
    pub writable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerMount {
    pub name: String,
    pub path: PathBuf,
    pub read_only: bool,
    #[serde(default)]
    pub intent: RunnerMountIntent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerMountIntent {
    pub kind: RunnerMountKind,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<RunnerSecretRef>,
}

impl Default for RunnerMountIntent {
    fn default() -> Self {
        Self {
            kind: RunnerMountKind::ProviderNative,
            uri: String::new(),
            credentials: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerMountKind {
    S3,
    Gcs,
    R2,
    AzureBlob,
    BoxStorage,
    ProviderNative,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerSecretRef {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerSnapshotRef {
    pub provider_id: RemoteRunnerProviderId,
    pub snapshot_id: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerSessionState {
    pub provider_id: RemoteRunnerProviderId,
    pub session_id: RemoteRunnerSessionId,
    pub destination_id: RunnerDestinationId,
    #[serde(default)]
    pub snapshot: Option<RunnerSnapshotRef>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/**
 * Per-thread remote-runner binding chosen at thread creation. Native coding
 * tools for a bound thread execute against this runner instead of the local
 * filesystem; the destination config is persisted with the thread, so secrets
 * must reach the provider through its environment, not this config.
 */
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadRunnerBinding {
    pub destination: RunnerDestination,
    /// Absolute path on the runner used as the thread's coding-tool workspace root.
    pub workspace: PathBuf,
    /**
     * Extra absolute runner paths that file reads may resolve under, in
     * addition to `workspace`. Writes and the working directory stay confined
     * to `workspace`; these only widen read resolution (e.g. read-only
     * resource mounts outside the writable workspace root).
     */
    #[serde(default)]
    pub read_roots: Vec<PathBuf>,
}

/**
 * Remote workspace handle carried on the tool execution context for
 * runner-bound threads. Tools route file and shell operations through
 * `session` with paths scoped under `root` (a path on the runner, not the
 * local filesystem).
 */
#[derive(Clone)]
pub struct RemoteWorkspace {
    pub session: Arc<dyn RemoteRunnerSession>,
    pub root: PathBuf,
    /**
     * Extra absolute runner paths reads may resolve under, beyond `root`.
     * Writes and the working directory stay confined to `root`.
     */
    pub read_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerCommandRequest {
    pub command_id: RunnerCommandId,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerCommandResult {
    pub command_id: RunnerCommandId,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerFileReadRequest {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerFileReadResult {
    pub path: PathBuf,
    pub contents: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerFileWriteRequest {
    pub path: PathBuf,
    pub contents: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerPortRequest {
    pub port: u16,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerPortResult {
    pub port: u16,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerArtifactExportRequest {
    pub path: PathBuf,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunnerArtifactExportResult {
    pub path: PathBuf,
    pub artifact_id: String,
    pub url: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[async_trait::async_trait]
pub trait RemoteRunnerProvider: Send + Sync + 'static {
    fn id(&self) -> RemoteRunnerProviderId;
    fn capabilities(&self) -> RunnerCapabilities;

    /**
     * Optional setup guidance shown by runner pickers when the provider is
     * installed but not yet usable (for example a missing credential env
     * var). Must name only documented env vars and never include secret
     * values. `None` means the provider is ready or needs no setup hint.
     */
    fn setup_hint(&self) -> Option<String> {
        None
    }

    /**
     * Default absolute workspace path on the runner for threads that select
     * this provider as a runtime-level destination without an explicit
     * per-thread workspace. When `Some`, selecting this runner (e.g. from the
     * TUI runner picker or config `default_destination`) routes a new thread's
     * coding tools into the runner at this path. `None` (the default) keeps the
     * legacy behavior where only an explicit `thread/start` binding routes
     * tools, so other providers are unchanged.
     */
    fn default_workspace(&self) -> Option<String> {
        None
    }

    async fn create_session(
        &self,
        destination: RunnerDestination,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>>;

    async fn validate_destination(&self, _destination: &RunnerDestination) -> anyhow::Result<()> {
        Ok(())
    }

    async fn resume_session(
        &self,
        state: RunnerSessionState,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>>;

    /**
     * Reattach to a previously created remote sandbox from persisted state
     * without provisioning a new one. Providers that expose a durable,
     * rejoinable sandbox (see `RunnerCapabilities::detachable`) override this;
     * the default reuses `resume_session`.
     */
    async fn rejoin_session(
        &self,
        state: RunnerSessionState,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        self.resume_session(state).await
    }
}

#[async_trait::async_trait]
pub trait RemoteRunnerSession: Send + Sync + 'static {
    fn state(&self) -> RunnerSessionState;

    /**
     * Move the session toward a paused/standby state to save cost. Default is a
     * no-op for providers that do not support pausing
     * (`RunnerCapabilities::pausable == false`). Returns the post-pause state.
     */
    async fn pause(&self) -> anyhow::Result<RunnerSessionState> {
        Ok(self.state())
    }

    /**
     * Wake a paused/standby session so subsequent commands run immediately.
     * Default is a no-op. Returns the post-resume state.
     */
    async fn resume(&self) -> anyhow::Result<RunnerSessionState> {
        Ok(self.state())
    }

    /**
     * Release the local session handle while keeping the remote sandbox alive
     * for a later `rejoin_session`. Returns the durable state that callers must
     * persist to rejoin. Default errors for providers that are not detachable.
     */
    async fn detach(&self) -> anyhow::Result<RunnerSessionState> {
        anyhow::bail!("runner detach is not supported by this provider")
    }

    async fn run_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult>;

    async fn cancel_command(&self, _command_id: &RunnerCommandId) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn read_file(
        &self,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult>;

    async fn write_file(&self, request: RunnerFileWriteRequest) -> anyhow::Result<()>;

    async fn expose_port(&self, request: RunnerPortRequest) -> anyhow::Result<RunnerPortResult>;

    async fn export_artifact(
        &self,
        _request: RunnerArtifactExportRequest,
    ) -> anyhow::Result<RunnerArtifactExportResult> {
        anyhow::bail!("runner artifact export is not supported by this provider")
    }

    async fn snapshot(&self) -> anyhow::Result<Option<RunnerSnapshotRef>>;

    async fn close(&self) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_runner_types_round_trip_json() {
        let destination = RunnerDestination {
            id: "local".to_string(),
            provider_id: "unix-local".to_string(),
            config: serde_json::json!({ "root": "." }),
            default_manifest: RunnerManifest {
                entries: vec![RunnerManifestEntry {
                    source: "src".into(),
                    target: "workspace/src".into(),
                    writable: true,
                }],
                mounts: vec![RunnerMount {
                    name: "cache".to_string(),
                    path: ".cache".into(),
                    read_only: false,
                    intent: RunnerMountIntent::default(),
                }],
            },
        };

        let encoded = serde_json::to_value(&destination).unwrap();
        let decoded: RunnerDestination = serde_json::from_value(encoded).unwrap();

        assert_eq!(decoded, destination);
    }

    #[test]
    fn capabilities_round_trip_includes_lifecycle_flags() {
        let capabilities = RunnerCapabilities {
            command_exec: true,
            file_read: true,
            file_write: true,
            port_preview: true,
            snapshots: false,
            cancellation: true,
            artifact_export: false,
            mounts: RunnerMountCapabilities::default(),
            pausable: true,
            detachable: true,
        };
        let encoded = serde_json::to_value(&capabilities).unwrap();
        let decoded: RunnerCapabilities = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, capabilities);
        assert!(decoded.pausable);
        assert!(decoded.detachable);

        // Older payloads without the lifecycle flags default to false.
        let legacy: RunnerCapabilities = serde_json::from_value(serde_json::json!({
            "command_exec": true,
            "file_read": true,
            "file_write": true,
            "port_preview": false,
            "snapshots": false,
            "cancellation": false
        }))
        .unwrap();
        assert!(!legacy.pausable);
        assert!(!legacy.detachable);
    }

    #[test]
    fn command_and_port_operations_are_protocol_safe() {
        let command = RunnerCommandRequest {
            command_id: "cmd-1".to_string(),
            program: "sh".to_string(),
            args: vec!["-lc".to_string(), "echo hi".to_string()],
            cwd: Some("workspace".into()),
            env: vec![("RUST_LOG".to_string(), "info".to_string())],
        };
        let port = RunnerPortResult {
            port: 3000,
            url: Some("https://preview.example".to_string()),
        };

        assert_eq!(
            serde_json::from_value::<RunnerCommandRequest>(serde_json::to_value(&command).unwrap())
                .unwrap(),
            command
        );
        assert_eq!(
            serde_json::from_value::<RunnerPortResult>(serde_json::to_value(&port).unwrap())
                .unwrap(),
            port
        );
    }

    #[test]
    fn mount_and_artifact_operations_are_protocol_safe() {
        let mount = RunnerMount {
            name: "dataset".to_string(),
            path: "mnt/dataset".into(),
            read_only: true,
            intent: RunnerMountIntent {
                kind: RunnerMountKind::R2,
                uri: "r2://bucket/prefix".to_string(),
                credentials: Some(RunnerSecretRef {
                    id: "r2-readonly".to_string(),
                }),
            },
        };
        let artifact = RunnerArtifactExportResult {
            path: "out/report.json".into(),
            artifact_id: "artifact-1".to_string(),
            url: Some("https://artifacts.example/report.json".to_string()),
            metadata: serde_json::json!({ "size": 128 }),
        };

        assert_eq!(
            serde_json::from_value::<RunnerMount>(serde_json::to_value(&mount).unwrap()).unwrap(),
            mount
        );
        assert_eq!(
            serde_json::from_value::<RunnerArtifactExportResult>(
                serde_json::to_value(&artifact).unwrap()
            )
            .unwrap(),
            artifact
        );
    }
}
