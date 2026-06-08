use roder_api::remote_runner::{
    RemoteRunnerSession, RunnerArtifactExportRequest, RunnerArtifactExportResult, RunnerCommandId,
    RunnerCommandRequest, RunnerCommandResult, RunnerDestinationId, RunnerFileReadRequest,
    RunnerFileReadResult, RunnerFileWriteRequest, RunnerPortRequest, RunnerPortResult,
    RunnerSessionState, RunnerSnapshotRef,
};

use crate::client::{Sprite, SpritesAppServerDeployment, SpritesClient};
use crate::config::{CleanupMode, PROVIDER_ID, SpritesConfig};

#[derive(Debug)]
pub struct SpritesRunnerSession {
    destination_id: RunnerDestinationId,
    client: SpritesClient,
    config: SpritesConfig,
    sprite: Sprite,
    app_server: Option<SpritesAppServerDeployment>,
}

impl SpritesRunnerSession {
    pub fn new(
        destination_id: RunnerDestinationId,
        client: SpritesClient,
        config: SpritesConfig,
        sprite: Sprite,
        app_server: Option<SpritesAppServerDeployment>,
    ) -> Self {
        Self {
            destination_id,
            client,
            config,
            sprite,
            app_server,
        }
    }

    pub fn from_state(
        state: RunnerSessionState,
        client: SpritesClient,
        config: SpritesConfig,
        sprite: Sprite,
    ) -> Self {
        Self {
            destination_id: state.destination_id,
            client,
            config,
            sprite,
            app_server: serde_json::from_value(
                state
                    .metadata
                    .get("remote_app_server")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            )
            .ok(),
        }
    }
}

#[async_trait::async_trait]
impl RemoteRunnerSession for SpritesRunnerSession {
    fn state(&self) -> RunnerSessionState {
        let mut metadata = serde_json::json!({
            "base_url": self.config.base_url,
            "sprite_name": self.sprite.name,
            "sprite_url": self.sprite.url,
            "working_dir": self.config.working_dir,
            "cleanup": self.config.cleanup,
        });
        if let Some(app_server) = &self.app_server {
            metadata["remote_app_server"] =
                serde_json::to_value(app_server).unwrap_or_else(|_| serde_json::Value::Null);
        }
        RunnerSessionState {
            provider_id: PROVIDER_ID.to_string(),
            session_id: self.sprite.name.clone(),
            destination_id: self.destination_id.clone(),
            snapshot: None,
            metadata,
        }
    }

    async fn run_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        self.client.run_command(&self.sprite.name, request).await
    }

    async fn cancel_command(&self, command_id: &RunnerCommandId) -> anyhow::Result<bool> {
        self.client
            .cancel_command(&self.sprite.name, command_id)
            .await
    }

    async fn read_file(
        &self,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        self.client.read_file(&self.sprite.name, request).await
    }

    async fn write_file(&self, request: RunnerFileWriteRequest) -> anyhow::Result<()> {
        self.client.write_file(&self.sprite.name, request).await
    }

    async fn expose_port(&self, request: RunnerPortRequest) -> anyhow::Result<RunnerPortResult> {
        self.client.expose_port(&self.sprite, request).await
    }

    async fn export_artifact(
        &self,
        request: RunnerArtifactExportRequest,
    ) -> anyhow::Result<RunnerArtifactExportResult> {
        self.client
            .export_artifact(&self.sprite.name, request)
            .await
    }

    async fn snapshot(&self) -> anyhow::Result<Option<RunnerSnapshotRef>> {
        self.client.snapshot(&self.sprite.name).await
    }

    async fn close(&self) -> anyhow::Result<()> {
        if self.config.cleanup == CleanupMode::DeleteOnClose {
            self.client.delete_sprite(&self.sprite.name).await?;
        }
        Ok(())
    }
}
