use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::remote_runner::{
    RemoteRunnerProvider, RemoteRunnerProviderId, RemoteRunnerSession, RunnerCapabilities,
    RunnerCommandId, RunnerCommandRequest, RunnerCommandResult, RunnerDestination,
    RunnerFileReadRequest, RunnerFileReadResult, RunnerFileWriteRequest, RunnerPortRequest,
    RunnerPortResult, RunnerSessionState, RunnerSnapshotRef,
};
use semver::Version;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const PROVIDER_ID: &str = "docker";
const DEFAULT_IMAGE: &str = "rust:latest";

#[derive(Debug, Default)]
pub struct DockerRunnerExtension;

impl RoderExtension for DockerRunnerExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-runner-docker".to_string(),
            name: "Docker Runner".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Runs remote-runner sessions in local Docker containers.".to_string(),
            ),
            provides: vec![ProvidedService::RemoteRunnerProvider(
                PROVIDER_ID.to_string(),
            )],
            required_capabilities: vec![
                CapabilityRequest::new("fs.readwrite.workspace"),
                CapabilityRequest::new("process.spawn.docker"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.remote_runner_provider(Arc::new(DockerRunnerProvider));
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct DockerRunnerProvider;

#[async_trait::async_trait]
impl RemoteRunnerProvider for DockerRunnerProvider {
    fn id(&self) -> RemoteRunnerProviderId {
        PROVIDER_ID.to_string()
    }

    fn capabilities(&self) -> RunnerCapabilities {
        RunnerCapabilities {
            command_exec: true,
            file_read: true,
            file_write: true,
            port_preview: true,
            snapshots: true,
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
        let config = DockerRunnerConfig::from_destination(&destination)?;
        let args = DockerCommandBuilder::run_args(&config);
        let output = Command::new("docker")
            .args(&args)
            .output()
            .await
            .context("start docker runner container")?;
        ensure_success(&output, "docker run")?;
        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if container_id.is_empty() {
            bail!("docker did not return a container id");
        }
        Ok(Arc::new(DockerRunnerSession {
            destination_id: destination.id,
            config,
            container_id,
        }))
    }

    async fn validate_destination(&self, destination: &RunnerDestination) -> anyhow::Result<()> {
        reject_remote_storage_mounts(&destination.default_manifest)
    }

    async fn resume_session(
        &self,
        state: RunnerSessionState,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        let container_id = state.session_id;
        let status = Command::new("docker")
            .args(["inspect", "-f", "{{.State.Running}}", &container_id])
            .output()
            .await
            .context("inspect docker runner container")?;
        ensure_success(&status, "docker inspect")?;
        if String::from_utf8_lossy(&status.stdout).trim() != "true" {
            bail!("docker runner container {container_id} is not running");
        }
        let config = DockerRunnerConfig {
            image: state
                .metadata
                .get("image")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(DEFAULT_IMAGE)
                .to_string(),
            root: state
                .metadata
                .get("root")
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from),
            ports: Vec::new(),
        };
        Ok(Arc::new(DockerRunnerSession {
            destination_id: state.destination_id,
            config,
            container_id,
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
            anyhow::bail!("docker runner only supports provider-native mounts");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerRunnerConfig {
    image: String,
    root: Option<PathBuf>,
    ports: Vec<u16>,
}

impl DockerRunnerConfig {
    fn from_destination(destination: &RunnerDestination) -> anyhow::Result<Self> {
        let image = destination
            .config
            .get("image")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(DEFAULT_IMAGE)
            .to_string();
        let root = destination
            .config
            .get("root")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from);
        let ports = destination
            .config
            .get("ports")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_u64)
            .map(|port| u16::try_from(port).context("docker runner port out of range"))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self { image, root, ports })
    }
}

#[derive(Debug)]
struct DockerRunnerSession {
    destination_id: String,
    config: DockerRunnerConfig,
    container_id: String,
}

#[async_trait::async_trait]
impl RemoteRunnerSession for DockerRunnerSession {
    fn state(&self) -> RunnerSessionState {
        RunnerSessionState {
            provider_id: PROVIDER_ID.to_string(),
            session_id: self.container_id.clone(),
            destination_id: self.destination_id.clone(),
            snapshot: None,
            metadata: serde_json::json!({
                "image": self.config.image,
                "root": self.config.root.as_ref().map(|path| path.display().to_string()),
            }),
        }
    }

    async fn run_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        let args = DockerCommandBuilder::exec_args(&self.container_id, &request);
        let output = Command::new("docker")
            .args(&args)
            .output()
            .await
            .context("docker exec runner command")?;
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
        validate_container_path(&request.path)?;
        let output = Command::new("docker")
            .args(DockerCommandBuilder::read_file_args(
                &self.container_id,
                &request.path,
            ))
            .output()
            .await
            .context("docker read runner file")?;
        ensure_success(&output, "docker exec cat")?;
        Ok(RunnerFileReadResult {
            path: request.path,
            contents: output.stdout,
        })
    }

    async fn cancel_command(&self, _command_id: &RunnerCommandId) -> anyhow::Result<bool> {
        let output = Command::new("docker")
            .args(["kill", &self.container_id])
            .output()
            .await
            .context("cancel docker runner command")?;
        ensure_success(&output, "docker kill")?;
        Ok(true)
    }

    async fn write_file(&self, request: RunnerFileWriteRequest) -> anyhow::Result<()> {
        validate_container_path(&request.path)?;
        let mut child = Command::new("docker")
            .args(DockerCommandBuilder::write_file_args(
                &self.container_id,
                &request.path,
            ))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("docker write runner file")?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("open docker write stdin"))?;
        stdin.write_all(&request.contents).await?;
        drop(stdin);
        let output = child.wait_with_output().await?;
        ensure_success(&output, "docker exec write")?;
        Ok(())
    }

    async fn expose_port(&self, request: RunnerPortRequest) -> anyhow::Result<RunnerPortResult> {
        Ok(RunnerPortResult {
            port: request.port,
            url: Some(format!("http://127.0.0.1:{}", request.port)),
        })
    }

    async fn snapshot(&self) -> anyhow::Result<Option<RunnerSnapshotRef>> {
        let snapshot_id = format!("docker-export-{}", self.container_id);
        let archive_path = std::env::temp_dir().join(format!("{snapshot_id}.tar"));
        let archive_path_str = archive_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("snapshot archive path is not utf-8"))?;
        let output = Command::new("docker")
            .args(["export", "-o", archive_path_str, &self.container_id])
            .output()
            .await
            .context("export docker runner snapshot")?;
        ensure_success(&output, "docker export")?;
        Ok(Some(RunnerSnapshotRef {
            provider_id: PROVIDER_ID.to_string(),
            snapshot_id,
            metadata: serde_json::json!({
                "container_id": self.container_id,
                "archive_path": archive_path,
                "strategy": "docker_export",
            }),
        }))
    }

    async fn close(&self) -> anyhow::Result<()> {
        let output = Command::new("docker")
            .args(["rm", "-f", &self.container_id])
            .output()
            .await
            .context("remove docker runner container")?;
        ensure_success(&output, "docker rm")?;
        Ok(())
    }
}

struct DockerCommandBuilder;

impl DockerCommandBuilder {
    fn run_args(config: &DockerRunnerConfig) -> Vec<String> {
        let args = ["run", "-d", "--rm", "-w", "/workspace"];
        let mut owned = args
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        if let Some(root) = &config.root {
            owned.push("-v".to_string());
            owned.push(format!("{}:/workspace", root.display()));
        }
        for port in &config.ports {
            owned.push("-p".to_string());
            owned.push(format!("{port}:{port}"));
        }
        owned.push(config.image.clone());
        owned.push("sleep".to_string());
        owned.push("infinity".to_string());
        owned
    }

    fn exec_args(container_id: &str, request: &RunnerCommandRequest) -> Vec<String> {
        let mut args = vec!["exec".to_string(), "-w".to_string()];
        args.push(container_cwd(request.cwd.as_deref()));
        for (name, value) in &request.env {
            args.push("-e".to_string());
            args.push(format!("{name}={value}"));
        }
        args.push(container_id.to_string());
        args.push(request.program.clone());
        args.extend(request.args.clone());
        args
    }

    fn read_file_args(container_id: &str, path: &Path) -> Vec<String> {
        vec![
            "exec".to_string(),
            container_id.to_string(),
            "cat".to_string(),
            container_path(path),
        ]
    }

    fn write_file_args(container_id: &str, path: &Path) -> Vec<String> {
        let path = container_path(path);
        let parent = Path::new(&path)
            .parent()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|| "/workspace".to_string());
        vec![
            "exec".to_string(),
            "-i".to_string(),
            container_id.to_string(),
            "sh".to_string(),
            "-lc".to_string(),
            format!(
                "mkdir -p {} && cat > {}",
                shell_quote(&parent),
                shell_quote(&path)
            ),
        ]
    }
}

fn container_cwd(cwd: Option<&Path>) -> String {
    cwd.map(container_path)
        .unwrap_or_else(|| "/workspace".to_string())
}

fn container_path(path: &Path) -> String {
    if path.as_os_str().is_empty() || path == Path::new(".") {
        return "/workspace".to_string();
    }
    if path.is_absolute() {
        path.to_string_lossy().to_string()
    } else {
        format!("/workspace/{}", path.to_string_lossy())
    }
}

fn validate_container_path(path: &Path) -> anyhow::Result<()> {
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::Prefix(_)
        )
    }) {
        bail!("runner file path {} escapes workspace", path.display());
    }
    Ok(())
}

fn ensure_success(output: &std::process::Output, action: &str) -> anyhow::Result<()> {
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "{action} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_run_args_include_workspace_image_and_ports() {
        let args = DockerCommandBuilder::run_args(&DockerRunnerConfig {
            image: "rust:latest".to_string(),
            root: Some("/tmp/work".into()),
            ports: vec![3000],
        });

        assert_eq!(args[0], "run");
        assert!(args.contains(&"-v".to_string()));
        assert!(args.contains(&"/tmp/work:/workspace".to_string()));
        assert!(args.contains(&"3000:3000".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("infinity"));
    }

    #[test]
    fn docker_exec_args_include_cwd_env_and_program() {
        let args = DockerCommandBuilder::exec_args(
            "container-1",
            &RunnerCommandRequest {
                command_id: "cmd".to_string(),
                program: "sh".to_string(),
                args: vec!["-lc".to_string(), "echo hi".to_string()],
                cwd: Some("subdir".into()),
                env: vec![("A".to_string(), "B".to_string())],
            },
        );

        assert_eq!(
            args,
            vec![
                "exec",
                "-w",
                "/workspace/subdir",
                "-e",
                "A=B",
                "container-1",
                "sh",
                "-lc",
                "echo hi"
            ]
        );
    }

    #[test]
    fn docker_file_paths_reject_parent_escapes() {
        assert!(validate_container_path(Path::new("src/main.rs")).is_ok());
        assert!(validate_container_path(Path::new("../secret")).is_err());
    }
}
