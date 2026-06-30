use std::sync::Arc;

use async_trait::async_trait;
use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::remote_runner::{
    RemoteRunnerProvider, RemoteRunnerProviderId, RemoteRunnerSession, RunnerCapabilities,
    RunnerDestination, RunnerMountCapabilities, RunnerSessionState,
};
use semver::Version;

mod client;
pub mod config;
mod session;

pub use client::{BlaxelClient, ProcessResponse, Sandbox, wait_until_ready};
pub use config::{
    BASE_URL_ENV, BlaxelConfig, CleanupMode, DEFAULT_BASE_URL, EXTENSION_ID, LIVE_ENV, PROVIDER_ID,
    Redacted, TOKEN_ENV, WORKSPACE_ENV,
};
pub use session::BlaxelRunnerSession;

const READINESS_ATTEMPTS: u32 = 20;

#[derive(Debug, Default)]
pub struct BlaxelRunnerExtension;

impl RoderExtension for BlaxelRunnerExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: EXTENSION_ID.to_string(),
            name: "Blaxel Runner".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Runs remote-runner sessions inside Blaxel sandboxes with pause, resume, detach, \
                 and rejoin support."
                    .to_string(),
            ),
            provides: vec![ProvidedService::RemoteRunnerProvider(
                PROVIDER_ID.to_string(),
            )],
            required_capabilities: vec![
                CapabilityRequest::new("network.http"),
                CapabilityRequest::new(format!("secret.read.{TOKEN_ENV}")),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.remote_runner_provider(Arc::new(BlaxelRunnerProvider::default()));
        Ok(())
    }
}

/// User-Agent sent on every Blaxel request. Blaxel's edge (CloudFront) rejects
/// requests with an empty User-Agent (403), so this must be non-empty.
const USER_AGENT: &str = concat!("roder-ext-runner-blaxel/", env!("CARGO_PKG_VERSION"));

#[derive(Debug)]
pub struct BlaxelRunnerProvider {
    http: reqwest::Client,
}

impl Default for BlaxelRunnerProvider {
    fn default() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .unwrap_or_default();
        Self { http }
    }
}

impl BlaxelRunnerProvider {
    fn client(&self, config: &BlaxelConfig) -> BlaxelClient {
        BlaxelClient::new(self.http.clone(), config)
    }
}

#[async_trait]
impl RemoteRunnerProvider for BlaxelRunnerProvider {
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
            mounts: RunnerMountCapabilities::default(),
            pausable: true,
            detachable: true,
        }
    }

    fn default_workspace(&self) -> Option<String> {
        Some(config::DEFAULT_WORKING_DIR.to_string())
    }

    fn setup_hint(&self) -> Option<String> {
        let has_token = [config::RODER_TOKEN_ENV, TOKEN_ENV, config::BL_TOKEN_ENV]
            .iter()
            .any(|name| {
                std::env::var(name)
                    .ok()
                    .is_some_and(|value| !value.trim().is_empty())
            });
        if has_token {
            None
        } else {
            Some(format!(
                "set {TOKEN_ENV} (or BL_API_KEY) and {WORKSPACE_ENV} to run Blaxel sandboxes; \
                 see docs/roder-blaxel-runner.md"
            ))
        }
    }

    async fn validate_destination(&self, destination: &RunnerDestination) -> anyhow::Result<()> {
        let config = BlaxelConfig::from_destination(destination)?;
        if config.token.is_empty() {
            anyhow::bail!("blaxel runner token cannot be empty");
        }
        if !destination.default_manifest.mounts.is_empty() {
            anyhow::bail!("blaxel runner does not support managed mounts");
        }
        Ok(())
    }

    async fn create_session(
        &self,
        destination: RunnerDestination,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        let mut config = BlaxelConfig::from_destination(&destination)?;
        let external_id = config
            .external_id
            .clone()
            .unwrap_or_else(|| destination.id.clone());
        config.external_id = Some(external_id.clone());
        let name = config
            .sandbox_name
            .clone()
            .unwrap_or_else(|| sanitize_name(&format!("{}-{}", config.sandbox_name_prefix, external_id)));
        config.sandbox_name = Some(name.clone());

        let client = self.client(&config);
        let sandbox = client.create_sandbox(&name, &config).await?;
        let (sandbox, endpoint) = wait_until_ready(&client, sandbox, READINESS_ATTEMPTS).await?;
        client.make_dir(&endpoint, &config.working_dir).await.ok();

        Ok(Arc::new(BlaxelRunnerSession::new(
            client,
            &config,
            destination.id,
            sandbox.metadata.name,
            Some(external_id),
            endpoint,
            false,
        )))
    }

    async fn resume_session(
        &self,
        state: RunnerSessionState,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        self.rejoin_session(state).await
    }

    async fn rejoin_session(
        &self,
        state: RunnerSessionState,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        let config = BlaxelConfig::from_state(&state)?;
        let client = self.client(&config);
        let name = config
            .sandbox_name
            .clone()
            .ok_or_else(|| anyhow::anyhow!("blaxel runner state is missing sandbox_name"))?;

        // Prefer the named sandbox; fall back to external-id recovery so a lost
        // name still rejoins a non-terminated sandbox.
        let sandbox = match client.get_sandbox(&name).await {
            Ok(sandbox) if !sandbox.is_terminated() => sandbox,
            other => {
                let recovered = if let Some(external_id) = &config.external_id {
                    client.get_sandbox_by_external_id(external_id).await?
                } else {
                    None
                };
                match recovered {
                    Some(sandbox) => sandbox,
                    None => other?,
                }
            }
        };

        let external_id = sandbox.metadata.external_id.clone().or(config.external_id.clone());
        let (sandbox, endpoint) = wait_until_ready(&client, sandbox, READINESS_ATTEMPTS).await?;
        Ok(Arc::new(BlaxelRunnerSession::new(
            client,
            &config,
            state.destination_id,
            sandbox.metadata.name,
            external_id,
            endpoint,
            false,
        )))
    }
}

/// Sanitize an arbitrary id into a Blaxel sandbox name: lowercase alphanumeric
/// with single hyphens, max 49 chars (per the control-plane name rule).
pub fn sanitize_name(raw: &str) -> String {
    let mut out = String::new();
    let mut last_hyphen = false;
    for ch in raw.to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_hyphen = false;
        } else if !last_hyphen && !out.is_empty() {
            out.push('-');
            last_hyphen = true;
        }
    }
    let trimmed = out.trim_matches('-');
    let truncated: String = trimmed.chars().take(49).collect();
    let name = truncated.trim_end_matches('-').to_string();
    if name.is_empty() {
        "roder-sandbox".to_string()
    } else {
        name
    }
}

/// Opt-in live smoke (`RODER_LIVE_BLAXEL_RUNNER=1`) exercising create -> exec ->
/// pause -> resume -> detach -> rejoin -> delete against a real Blaxel account.
pub async fn run_live_smoke_if_enabled() {
    if std::env::var(LIVE_ENV).ok().as_deref() != Some("1") {
        eprintln!("set {LIVE_ENV}=1 to run the live blaxel runner smoke");
        return;
    }
    if let Err(error) = run_live_smoke().await {
        eprintln!("blaxel live smoke failed: {error:?}");
    }
}

async fn run_live_smoke() -> anyhow::Result<()> {
    use roder_api::remote_runner::{
        RunnerCommandRequest, RunnerFileReadRequest, RunnerFileWriteRequest, RunnerManifest,
        RunnerPortRequest,
    };

    let provider = BlaxelRunnerProvider::default();
    let destination = RunnerDestination {
        id: "blaxel-live".to_string(),
        provider_id: PROVIDER_ID.to_string(),
        config: serde_json::json!({
            "sandbox_name_prefix": "roder-live",
            "working_dir": "/home/user/roder-live",
            "cleanup": "delete-on-close"
        }),
        default_manifest: RunnerManifest::default(),
    };
    let session = provider.create_session(destination).await?;

    // Command exec.
    let out = session
        .run_command(RunnerCommandRequest {
            command_id: "smoke".to_string(),
            program: "echo".to_string(),
            args: vec!["blaxel-live".to_string()],
            cwd: None,
            env: Vec::new(),
        })
        .await?;
    println!("live exec stdout: {}", out.stdout.trim());
    anyhow::ensure!(out.stdout.trim() == "blaxel-live", "unexpected exec stdout");

    // File write then read round-trips through the per-sandbox filesystem API.
    session
        .write_file(RunnerFileWriteRequest {
            path: "hello.txt".into(),
            contents: b"hello blaxel".to_vec(),
        })
        .await?;
    let file = session
        .read_file(RunnerFileReadRequest {
            path: "hello.txt".into(),
        })
        .await?;
    anyhow::ensure!(file.contents == b"hello blaxel", "file round-trip mismatch");
    println!("live file read: {}", String::from_utf8_lossy(&file.contents));

    // Port preview returns a public preview URL.
    let preview = session
        .expose_port(RunnerPortRequest {
            port: 3000,
            label: Some("web".to_string()),
        })
        .await?;
    println!("live preview url: {:?}", preview.url);

    // Lifecycle: pause -> resume -> detach -> rejoin -> delete.
    session.pause().await?;
    session.resume().await?;
    let state = session.detach().await?;
    let rejoined = provider.rejoin_session(state).await?;
    // Prove rejoin reuses the same sandbox (filesystem state survived).
    let after = rejoined
        .read_file(RunnerFileReadRequest {
            path: "hello.txt".into(),
        })
        .await?;
    anyhow::ensure!(
        after.contents == b"hello blaxel",
        "rejoined sandbox lost filesystem state"
    );
    println!("live rejoin read: {}", String::from_utf8_lossy(&after.contents));
    rejoined.close().await?;
    println!("live smoke OK: exec, file rw, preview, pause/resume/detach/rejoin, delete");
    Ok(())
}
