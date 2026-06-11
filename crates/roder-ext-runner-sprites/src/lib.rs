use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::remote_runner::{
    RemoteRunnerProvider, RemoteRunnerProviderId, RemoteRunnerSession, RunnerCapabilities,
    RunnerDestination, RunnerSessionState,
};
use semver::Version;

mod checkpoints;
mod client;
pub mod config;
mod exec_ws;
mod filesystem;
mod policies;
mod session;
mod ws;

pub use checkpoints::{SpriteCheckpoint, is_checkpoint_restore_supported};
pub use client::{
    SpriteService, SpriteServiceState, SpritesAppServerDeployment, SpritesClient,
    SpritesCommandResponse, SpritesHttpError,
};
pub use filesystem::SpriteFsEntry;
pub use ws::{WsExecOutcome, WsExecRequest};
pub use config::{
    BASE_URL_ENV, CleanupMode, DEFAULT_APP_SERVER_TOKEN_ENV, DEFAULT_BASE_URL,
    DEFAULT_REMOTE_RODER_BASE_URL, DEFAULT_REMOTE_RODER_BINARY, LIVE_ENV, PROVIDER_ID,
    RODER_BASE_URL_ENV, RODER_TOKEN_ENV, SPRITES_EXTENSION_ID, SpritesAppServerConfig,
    SpritesConfig, TOKEN_ENV,
};
pub use exec_ws::{ExecFrame, decode_non_tty_frame};
pub use filesystem::normalize_workspace_path;
pub use session::SpritesRunnerSession;

#[derive(Debug, Default)]
pub struct SpritesRunnerExtension;

impl RoderExtension for SpritesRunnerExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: SPRITES_EXTENSION_ID.to_string(),
            name: "Fly Sprites Runner".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Runs remote-runner sessions in persistent Fly Sprites sandboxes.".to_string(),
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
        registry.remote_runner_provider(Arc::new(SpritesRunnerProvider::default()));
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct SpritesRunnerProvider {
    client: reqwest::Client,
}

#[async_trait::async_trait]
impl RemoteRunnerProvider for SpritesRunnerProvider {
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
                "set {TOKEN_ENV} (or {RODER_TOKEN_ENV}) to run Fly Sprites sandboxes; see \
                 docs/roder-fly-sprites-runner.md"
            ))
        }
    }

    fn capabilities(&self) -> RunnerCapabilities {
        RunnerCapabilities {
            command_exec: true,
            file_read: true,
            file_write: true,
            port_preview: true,
            snapshots: true,
            cancellation: true,
            artifact_export: true,
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
        let config = SpritesConfig::from_destination(&destination)?;
        let client = SpritesClient::new(self.client.clone(), config.clone());
        let sprite = client.ensure_sprite().await?;
        // Restore happens first so manifests/app-server bootstrap overlay the
        // restored filesystem rather than the other way around.
        if let Some(checkpoint_id) = &config.restore_checkpoint_id {
            client.restore_checkpoint(&sprite.name, checkpoint_id).await?;
        }
        policies::apply_configured_policies(&client, &sprite.name, &config).await?;
        client.ensure_working_dir(&sprite.name).await?;
        client
            .materialize_manifest(&sprite.name, &destination.default_manifest)
            .await?;
        let app_server = if let Some(app_server) = &config.app_server {
            Some(client.deploy_app_server(&sprite, app_server).await?)
        } else {
            None
        };
        Ok(Arc::new(SpritesRunnerSession::new(
            destination.id,
            client,
            config,
            sprite,
            app_server,
        )))
    }

    async fn validate_destination(&self, destination: &RunnerDestination) -> anyhow::Result<()> {
        let config = SpritesConfig::from_destination(destination)?;
        if config.token.trim().is_empty() {
            anyhow::bail!("sprites runner token cannot be empty");
        }
        for mount in &destination.default_manifest.mounts {
            if !matches!(
                mount.intent.kind,
                roder_api::remote_runner::RunnerMountKind::ProviderNative
            ) {
                anyhow::bail!("sprites runner only supports provider-native mounts");
            }
        }
        Ok(())
    }

    async fn resume_session(
        &self,
        state: RunnerSessionState,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        let config = SpritesConfig::from_state(&state)?;
        let client = SpritesClient::new(self.client.clone(), config.clone());
        let sprite_name = state
            .metadata
            .get("sprite_name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("sprites runner state is missing sprite_name"))?;
        let sprite = client.get_sprite(sprite_name).await?;
        Ok(Arc::new(SpritesRunnerSession::from_state(
            state, client, config, sprite,
        )))
    }
}

pub async fn run_live_smoke_if_enabled() {
    if std::env::var(LIVE_ENV).ok().as_deref() != Some("1") {
        eprintln!("set {LIVE_ENV}=1 to run the live sprites runner smoke");
        return;
    }

    let provider = SpritesRunnerProvider::default();
    let destination = RunnerDestination {
        id: "sprites-live".to_string(),
        provider_id: PROVIDER_ID.to_string(),
        config: serde_json::json!({
            "sprite_name_prefix": "roder-live",
            "cleanup": if std::env::var("RODER_SPRITES_LIVE_KEEP").ok().as_deref() == Some("1") {
                "keep"
            } else {
                "delete-on-close"
            },
            "working_dir": "/home/sprite/roder-live"
        }),
        default_manifest: roder_api::remote_runner::RunnerManifest::default(),
    };

    let session = provider.create_session(destination).await.unwrap();
    session
        .write_file(roder_api::remote_runner::RunnerFileWriteRequest {
            path: "hello.txt".into(),
            contents: b"hello sprites\n".to_vec(),
        })
        .await
        .unwrap();
    let read = session
        .read_file(roder_api::remote_runner::RunnerFileReadRequest {
            path: "hello.txt".into(),
        })
        .await
        .unwrap();
    assert_eq!(read.contents, b"hello sprites\n");
    let command = session
        .run_command(roder_api::remote_runner::RunnerCommandRequest {
            command_id: "live-python".to_string(),
            program: "python3".to_string(),
            args: vec!["-c".to_string(), "print(2+2)".to_string()],
            cwd: None,
            env: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(command.exit_code, Some(0));
    assert!(command.stdout.trim().ends_with('4'));
    assert!(session.snapshot().await.unwrap().is_some());
    session.close().await.unwrap();
}
