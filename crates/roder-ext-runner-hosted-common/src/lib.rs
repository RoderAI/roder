use std::sync::Arc;

use anyhow::{Context, bail};
use roder_api::remote_runner::{
    RemoteRunnerProvider, RemoteRunnerProviderId, RemoteRunnerSession, RunnerArtifactExportRequest,
    RunnerArtifactExportResult, RunnerCapabilities, RunnerCommandId, RunnerCommandRequest,
    RunnerCommandResult, RunnerDestination, RunnerFileReadRequest, RunnerFileReadResult,
    RunnerFileWriteRequest, RunnerPortRequest, RunnerPortResult, RunnerSessionState,
    RunnerSnapshotRef,
};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct HostedRunnerSpec {
    pub provider_id: &'static str,
    pub token_env: &'static str,
    pub base_url_env: &'static str,
    pub default_base_url: &'static str,
    pub live_env: &'static str,
}

impl HostedRunnerSpec {
    pub fn capabilities(&self) -> RunnerCapabilities {
        RunnerCapabilities {
            command_exec: true,
            file_read: true,
            file_write: true,
            port_preview: true,
            snapshots: true,
            cancellation: true,
            artifact_export: true,
            mounts: roder_api::remote_runner::RunnerMountCapabilities {
                s3: true,
                gcs: true,
                r2: true,
                azure_blob: true,
                box_storage: true,
                provider_native: true,
            },
            pausable: false,
            detachable: false,
        }
    }
}

#[derive(Debug)]
pub struct HostedRunnerProvider {
    spec: HostedRunnerSpec,
    client: reqwest::Client,
}

impl HostedRunnerProvider {
    pub fn new(spec: HostedRunnerSpec) -> Self {
        Self {
            spec,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl RemoteRunnerProvider for HostedRunnerProvider {
    fn id(&self) -> RemoteRunnerProviderId {
        self.spec.provider_id.to_string()
    }

    fn capabilities(&self) -> RunnerCapabilities {
        self.spec.capabilities()
    }

    async fn create_session(
        &self,
        destination: RunnerDestination,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        let config = HostedRunnerConfig::from_destination(&self.spec, &destination)?;
        let payload = serde_json::json!({
            "destination_id": destination.id,
            "provider_id": self.spec.provider_id,
            "manifest": destination.default_manifest,
            "config": redacted_config(&destination.config),
        });
        let response = self
            .client
            .post(format!("{}/sessions", config.base_url))
            .bearer_auth(&config.token)
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&payload)?)
            .send()
            .await
            .with_context(|| format!("create {} runner session", self.spec.provider_id))?;
        let created: CreateSessionResponse = decode_response(response, "create session").await?;
        let state = RunnerSessionState {
            provider_id: self.spec.provider_id.to_string(),
            session_id: created.session_id,
            destination_id: destination.id,
            snapshot: created.snapshot,
            metadata: serde_json::json!({
                "base_url": config.base_url,
                "provider": self.spec.provider_id,
            }),
        };
        Ok(Arc::new(HostedRunnerSession {
            spec: self.spec.clone(),
            client: self.client.clone(),
            state,
            token: config.token,
            base_url: config.base_url,
        }))
    }

    async fn validate_destination(&self, destination: &RunnerDestination) -> anyhow::Result<()> {
        let _ = HostedRunnerConfig::from_destination(&self.spec, destination)?;
        for mount in &destination.default_manifest.mounts {
            validate_mount_supported(mount, &self.spec.capabilities().mounts)?;
        }
        Ok(())
    }

    async fn resume_session(
        &self,
        state: RunnerSessionState,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        let config = HostedRunnerConfig::from_state(&self.spec, &state)?;
        let response = self
            .client
            .get(format!("{}/sessions/{}", config.base_url, state.session_id))
            .bearer_auth(&config.token)
            .send()
            .await
            .with_context(|| format!("resume {} runner session", self.spec.provider_id))?;
        let _: serde_json::Value = decode_response(response, "resume session").await?;
        Ok(Arc::new(HostedRunnerSession {
            spec: self.spec.clone(),
            client: self.client.clone(),
            state,
            token: config.token,
            base_url: config.base_url,
        }))
    }
}

fn validate_mount_supported(
    mount: &roder_api::remote_runner::RunnerMount,
    capabilities: &roder_api::remote_runner::RunnerMountCapabilities,
) -> anyhow::Result<()> {
    use roder_api::remote_runner::RunnerMountKind;

    let supported = match mount.intent.kind {
        RunnerMountKind::S3 => capabilities.s3,
        RunnerMountKind::Gcs => capabilities.gcs,
        RunnerMountKind::R2 => capabilities.r2,
        RunnerMountKind::AzureBlob => capabilities.azure_blob,
        RunnerMountKind::BoxStorage => capabilities.box_storage,
        RunnerMountKind::ProviderNative => capabilities.provider_native,
    };
    if !supported {
        anyhow::bail!(
            "{} runner does not support {:?} mounts",
            mount.name,
            mount.intent.kind
        );
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct HostedRunnerConfig {
    token: String,
    base_url: String,
}

impl HostedRunnerConfig {
    fn from_destination(
        spec: &HostedRunnerSpec,
        destination: &RunnerDestination,
    ) -> anyhow::Result<Self> {
        let token = destination
            .config
            .get("token")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| std::env::var(spec.token_env).ok())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{} runner requires {} or destination config token",
                    spec.provider_id,
                    spec.token_env
                )
            })?;
        let base_url = destination
            .config
            .get("base_url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| std::env::var(spec.base_url_env).ok())
            .unwrap_or_else(|| spec.default_base_url.to_string())
            .trim_end_matches('/')
            .to_string();
        Ok(Self { token, base_url })
    }

    fn from_state(spec: &HostedRunnerSpec, state: &RunnerSessionState) -> anyhow::Result<Self> {
        let token = std::env::var(spec.token_env).map_err(|_| {
            anyhow::anyhow!(
                "{} runner resume requires {}",
                spec.provider_id,
                spec.token_env
            )
        })?;
        let base_url = state
            .metadata
            .get("base_url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| std::env::var(spec.base_url_env).ok())
            .unwrap_or_else(|| spec.default_base_url.to_string())
            .trim_end_matches('/')
            .to_string();
        Ok(Self { token, base_url })
    }
}

#[derive(Debug)]
struct HostedRunnerSession {
    spec: HostedRunnerSpec,
    client: reqwest::Client,
    state: RunnerSessionState,
    token: String,
    base_url: String,
}

#[async_trait::async_trait]
impl RemoteRunnerSession for HostedRunnerSession {
    fn state(&self) -> RunnerSessionState {
        self.state.clone()
    }

    async fn run_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        let response = self
            .client
            .post(format!(
                "{}/sessions/{}/commands",
                self.base_url, self.state.session_id
            ))
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&request)?)
            .send()
            .await
            .with_context(|| format!("run {} runner command", self.spec.provider_id))?;
        decode_response(response, "run command").await
    }

    async fn cancel_command(&self, command_id: &RunnerCommandId) -> anyhow::Result<bool> {
        let response = self
            .client
            .delete(format!(
                "{}/sessions/{}/commands/{}",
                self.base_url, self.state.session_id, command_id
            ))
            .bearer_auth(&self.token)
            .send()
            .await
            .with_context(|| format!("cancel {} runner command", self.spec.provider_id))?;
        let result: CancelCommandResponse = decode_response(response, "cancel command").await?;
        Ok(result.cancelled)
    }

    async fn read_file(
        &self,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        let response = self
            .client
            .get(format!(
                "{}/sessions/{}/files?path={}",
                self.base_url,
                self.state.session_id,
                urlencoding::encode(&request.path.to_string_lossy())
            ))
            .bearer_auth(&self.token)
            .send()
            .await
            .with_context(|| format!("read {} runner file", self.spec.provider_id))?;
        let bytes = decode_bytes(response, "read file").await?;
        Ok(RunnerFileReadResult {
            path: request.path,
            contents: bytes,
        })
    }

    async fn write_file(&self, request: RunnerFileWriteRequest) -> anyhow::Result<()> {
        let response = self
            .client
            .put(format!(
                "{}/sessions/{}/files?path={}",
                self.base_url,
                self.state.session_id,
                urlencoding::encode(&request.path.to_string_lossy())
            ))
            .bearer_auth(&self.token)
            .body(request.contents)
            .send()
            .await
            .with_context(|| format!("write {} runner file", self.spec.provider_id))?;
        decode_empty(response, "write file").await
    }

    async fn expose_port(&self, request: RunnerPortRequest) -> anyhow::Result<RunnerPortResult> {
        let response = self
            .client
            .post(format!(
                "{}/sessions/{}/ports",
                self.base_url, self.state.session_id
            ))
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&request)?)
            .send()
            .await
            .with_context(|| format!("expose {} runner port", self.spec.provider_id))?;
        decode_response(response, "expose port").await
    }

    async fn export_artifact(
        &self,
        request: RunnerArtifactExportRequest,
    ) -> anyhow::Result<RunnerArtifactExportResult> {
        let response = self
            .client
            .post(format!(
                "{}/sessions/{}/artifacts",
                self.base_url, self.state.session_id
            ))
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&request)?)
            .send()
            .await
            .with_context(|| format!("export {} runner artifact", self.spec.provider_id))?;
        decode_response(response, "export artifact").await
    }

    async fn snapshot(&self) -> anyhow::Result<Option<RunnerSnapshotRef>> {
        let response = self
            .client
            .post(format!(
                "{}/sessions/{}/snapshots",
                self.base_url, self.state.session_id
            ))
            .bearer_auth(&self.token)
            .send()
            .await
            .with_context(|| format!("snapshot {} runner", self.spec.provider_id))?;
        let result: SnapshotResponse = decode_response(response, "snapshot").await?;
        Ok(result.snapshot)
    }

    async fn close(&self) -> anyhow::Result<()> {
        let response = self
            .client
            .delete(format!(
                "{}/sessions/{}",
                self.base_url, self.state.session_id
            ))
            .bearer_auth(&self.token)
            .send()
            .await
            .with_context(|| format!("delete {} runner session", self.spec.provider_id))?;
        decode_empty(response, "delete session").await
    }
}

#[derive(Debug, Deserialize)]
struct CreateSessionResponse {
    session_id: String,
    #[serde(default)]
    snapshot: Option<RunnerSnapshotRef>,
}

#[derive(Debug, Deserialize)]
struct CancelCommandResponse {
    cancelled: bool,
}

#[derive(Debug, Deserialize)]
struct SnapshotResponse {
    #[serde(default)]
    snapshot: Option<RunnerSnapshotRef>,
}

async fn decode_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    operation: &str,
) -> anyhow::Result<T> {
    let status = response.status();
    let body = response.bytes().await?;
    if !status.is_success() {
        bail!("{operation} failed with {status}: {}", redact_body(&body));
    }
    serde_json::from_slice(&body).with_context(|| format!("decode {operation} response"))
}

async fn decode_bytes(response: reqwest::Response, operation: &str) -> anyhow::Result<Vec<u8>> {
    let status = response.status();
    let body = response.bytes().await?;
    if !status.is_success() {
        bail!("{operation} failed with {status}: {}", redact_body(&body));
    }
    Ok(body.to_vec())
}

async fn decode_empty(response: reqwest::Response, operation: &str) -> anyhow::Result<()> {
    let status = response.status();
    let body = response.bytes().await?;
    if !status.is_success() {
        bail!("{operation} failed with {status}: {}", redact_body(&body));
    }
    Ok(())
}

fn redacted_config(config: &serde_json::Value) -> serde_json::Value {
    let mut value = config.clone();
    if let Some(object) = value.as_object_mut() {
        for key in ["token", "api_key", "secret"] {
            if object.contains_key(key) {
                object.insert(
                    key.to_string(),
                    serde_json::Value::String("<redacted>".into()),
                );
            }
        }
    }
    value
}

fn redact_body(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let mut out = text.to_string();
    for marker in ["token", "api_key", "secret", "authorization"] {
        out = out.replace(marker, "<redacted>");
    }
    out
}

pub async fn run_mock_lifecycle_test(spec: HostedRunnerSpec) {
    use roder_api::remote_runner::{
        RunnerDestination, RunnerFileReadRequest, RunnerFileWriteRequest, RunnerManifest,
        RunnerPortRequest,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let provider_id = spec.provider_id.to_string();
    let server = tokio::spawn(async move {
        for _ in 0..9 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0_u8; 16384];
            let read = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]);
            let first = request.lines().next().unwrap_or_default();
            let (status, content_type, body) = mock_response(&provider_id, first);
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\ncontent-type: {content_type}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
        }
    });

    let provider = HostedRunnerProvider::new(spec.clone());
    assert_eq!(provider.capabilities(), spec.capabilities());
    let session = provider
        .create_session(RunnerDestination {
            id: spec.provider_id.to_string(),
            provider_id: spec.provider_id.to_string(),
            config: serde_json::json!({
                "token": "test-token",
                "base_url": base_url,
            }),
            default_manifest: RunnerManifest::default(),
        })
        .await
        .unwrap();
    let state = session.state();
    assert_eq!(state.provider_id, spec.provider_id);

    let command = session
        .run_command(RunnerCommandRequest {
            command_id: "cmd-1".to_string(),
            program: "echo".to_string(),
            args: vec!["hi".to_string()],
            cwd: None,
            env: Vec::new(),
            timeout_ms: None,
        })
        .await
        .unwrap();
    assert_eq!(command.stdout, "hi\n");

    let read = session
        .read_file(RunnerFileReadRequest {
            path: "README.md".into(),
        })
        .await
        .unwrap();
    assert_eq!(read.contents, b"file contents");

    session
        .write_file(RunnerFileWriteRequest {
            path: "README.md".into(),
            contents: b"new contents".to_vec(),
        })
        .await
        .unwrap();
    let port = session
        .expose_port(RunnerPortRequest {
            port: 3000,
            label: Some("web".to_string()),
        })
        .await
        .unwrap();
    assert_eq!(port.url.as_deref(), Some("https://preview.example"));
    let artifact = session
        .export_artifact(RunnerArtifactExportRequest {
            path: "out/report.txt".into(),
            recursive: false,
        })
        .await
        .unwrap();
    assert_eq!(artifact.artifact_id, "artifact-1");
    assert!(session.snapshot().await.unwrap().is_some());
    assert!(session.cancel_command(&"cmd-1".to_string()).await.unwrap());
    session.close().await.unwrap();
    server.await.unwrap();
}

fn mock_response(provider_id: &str, first_line: &str) -> (&'static str, &'static str, String) {
    if first_line.starts_with("POST /sessions ") {
        return (
            "200 OK",
            "application/json",
            serde_json::json!({ "session_id": format!("{provider_id}-session") }).to_string(),
        );
    }
    if first_line.starts_with("POST /sessions/")
        && first_line.contains("/commands ")
        && !first_line.contains("/commands/")
    {
        return (
            "200 OK",
            "application/json",
            serde_json::json!({
                "command_id": "cmd-1",
                "exit_code": 0,
                "stdout": "hi\n",
                "stderr": "",
            })
            .to_string(),
        );
    }
    if first_line.starts_with("GET /sessions/") && first_line.contains("/files?") {
        return (
            "200 OK",
            "application/octet-stream",
            "file contents".to_string(),
        );
    }
    if first_line.starts_with("PUT /sessions/") && first_line.contains("/files?") {
        return ("204 No Content", "application/json", String::new());
    }
    if first_line.starts_with("POST /sessions/") && first_line.contains("/ports ") {
        return (
            "200 OK",
            "application/json",
            serde_json::json!({ "port": 3000, "url": "https://preview.example" }).to_string(),
        );
    }
    if first_line.starts_with("POST /sessions/") && first_line.contains("/artifacts ") {
        return (
            "200 OK",
            "application/json",
            serde_json::json!({
                "path": "out/report.txt",
                "artifact_id": "artifact-1",
                "url": "https://artifacts.example/report.txt",
                "metadata": {}
            })
            .to_string(),
        );
    }
    if first_line.starts_with("POST /sessions/") && first_line.contains("/snapshots ") {
        return (
            "200 OK",
            "application/json",
            serde_json::json!({
                "snapshot": {
                    "provider_id": provider_id,
                    "snapshot_id": "snap-1",
                    "metadata": {}
                }
            })
            .to_string(),
        );
    }
    if first_line.starts_with("DELETE /sessions/") && first_line.contains("/commands/") {
        return (
            "200 OK",
            "application/json",
            serde_json::json!({ "cancelled": true }).to_string(),
        );
    }
    if first_line.starts_with("DELETE /sessions/") {
        return ("204 No Content", "application/json", String::new());
    }
    (
        "404 Not Found",
        "application/json",
        serde_json::json!({ "error": "missing route" }).to_string(),
    )
}

pub async fn run_live_smoke_if_enabled(spec: HostedRunnerSpec) {
    if std::env::var(spec.live_env).ok().as_deref() != Some("1") {
        eprintln!(
            "set {}=1 to run the live {} runner smoke",
            spec.live_env, spec.provider_id
        );
        return;
    }
    let provider = HostedRunnerProvider::new(spec.clone());
    let session = provider
        .create_session(RunnerDestination {
            id: spec.provider_id.to_string(),
            provider_id: spec.provider_id.to_string(),
            config: serde_json::Value::Null,
            default_manifest: roder_api::remote_runner::RunnerManifest::default(),
        })
        .await
        .unwrap();
    session.close().await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secret_config_fields() {
        let redacted = redacted_config(&serde_json::json!({
            "token": "plain-token",
            "api_key": "plain-key",
            "region": "iad",
        }));

        assert_eq!(redacted["token"], "<redacted>");
        assert_eq!(redacted["api_key"], "<redacted>");
        assert_eq!(redacted["region"], "iad");
        assert!(!redacted.to_string().contains("plain-token"));
        assert!(!redacted.to_string().contains("plain-key"));
    }

    #[test]
    fn redacts_secret_error_bodies() {
        let body = br#"{"error":"authorization token secret api_key leaked"}"#;
        let redacted = redact_body(body);

        assert!(!redacted.contains("authorization"));
        assert!(!redacted.contains("token"));
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("api_key"));
    }
}
