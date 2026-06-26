use std::sync::Arc;

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

pub use client::{CelestoClient, Computer, ExecResponse, PublishedPort};
pub use config::{
    BASE_URL_ENV, CELESTO_EXTENSION_ID, CelestoConfig, CleanupMode, DEFAULT_BASE_URL,
    DEFAULT_TEMPLATE_ID, DEFAULT_WORKING_DIR, LIVE_ENV, PROVIDER_ID, RODER_BASE_URL_ENV,
    RODER_TOKEN_ENV, TOKEN_ENV,
};
pub use session::CelestoRunnerSession;

#[derive(Debug, Default)]
pub struct CelestoRunnerExtension;

impl RoderExtension for CelestoRunnerExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: CELESTO_EXTENSION_ID.to_string(),
            name: "Celesto Runner".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Runs remote-runner sessions in Celesto sandboxed computers.".to_string(),
            ),
            provides: vec![ProvidedService::RemoteRunnerProvider(
                PROVIDER_ID.to_string(),
            )],
            required_capabilities: vec![
                CapabilityRequest::new("network.http"),
                CapabilityRequest::new(format!("secret.read.{TOKEN_ENV}")),
                CapabilityRequest::new(format!("secret.read.{RODER_TOKEN_ENV}")),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.remote_runner_provider(Arc::new(CelestoRunnerProvider::default()));
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct CelestoRunnerProvider {
    client: reqwest::Client,
}

#[async_trait::async_trait]
impl RemoteRunnerProvider for CelestoRunnerProvider {
    fn id(&self) -> RemoteRunnerProviderId {
        PROVIDER_ID.to_string()
    }

    fn setup_hint(&self) -> Option<String> {
        let has_token = [RODER_TOKEN_ENV, TOKEN_ENV].iter().any(|name| {
            std::env::var(name)
                .ok()
                .is_some_and(|value| !value.trim().is_empty())
        });
        if has_token {
            None
        } else {
            Some(format!(
                "set {TOKEN_ENV} (or {RODER_TOKEN_ENV}) to run Celesto sandboxed computers"
            ))
        }
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
        }
    }

    async fn create_session(
        &self,
        destination: RunnerDestination,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        let config = CelestoConfig::from_destination(&destination)?;
        let client = CelestoClient::new(self.client.clone(), config.clone());
        let computer = client.ensure_computer().await?;
        client.ensure_working_dir(&computer.id).await?;
        Ok(Arc::new(CelestoRunnerSession::new(
            destination.id,
            client,
            config,
            computer,
        )))
    }

    async fn validate_destination(&self, destination: &RunnerDestination) -> anyhow::Result<()> {
        let config = CelestoConfig::from_destination(destination)?;
        if config.token.trim().is_empty() {
            anyhow::bail!("celesto runner token cannot be empty");
        }
        if !destination.default_manifest.mounts.is_empty() {
            anyhow::bail!("celesto runner does not support object-store mounts");
        }
        Ok(())
    }

    async fn resume_session(
        &self,
        state: RunnerSessionState,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        let config = CelestoConfig::from_state(&state)?;
        let client = CelestoClient::new(self.client.clone(), config.clone());
        let computer_id = state
            .metadata
            .get("computer_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("celesto runner state is missing computer_id"))?;
        let computer = client.get_computer(computer_id).await?;
        let computer = client.ensure_running(computer).await?;
        Ok(Arc::new(CelestoRunnerSession::from_state(
            state, client, config, computer,
        )))
    }
}

/// Optional live smoke test, gated behind `RODER_LIVE_CELESTO_RUNNER=1` and a
/// real `CELESTO_API_KEY`. Creates a computer, runs a command, and deletes it.
pub async fn run_live_smoke_if_enabled() {
    if std::env::var(LIVE_ENV).ok().as_deref() != Some("1") {
        eprintln!("set {LIVE_ENV}=1 to run the live celesto runner smoke");
        return;
    }

    let provider = CelestoRunnerProvider::default();
    let destination = RunnerDestination {
        id: "celesto-live".to_string(),
        provider_id: PROVIDER_ID.to_string(),
        config: serde_json::json!({
            "template_id": DEFAULT_TEMPLATE_ID,
            "working_dir": "/root/roder-live",
            "cleanup": if std::env::var("RODER_CELESTO_LIVE_KEEP").ok().as_deref() == Some("1") {
                "keep"
            } else {
                "delete-on-close"
            }
        }),
        default_manifest: roder_api::remote_runner::RunnerManifest::default(),
    };

    let session = provider
        .create_session(destination)
        .await
        .expect("create celesto session");
    let result = session
        .run_command(roder_api::remote_runner::RunnerCommandRequest {
            command_id: "live-1".to_string(),
            program: "uname".to_string(),
            args: vec!["-a".to_string()],
            cwd: None,
            env: Vec::new(),
        })
        .await
        .expect("run live command");
    eprintln!("celesto live uname: {}", result.stdout.trim());
    session.close().await.expect("close celesto session");
}
