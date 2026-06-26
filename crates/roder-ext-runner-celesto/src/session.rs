use roder_api::remote_runner::{
    RemoteRunnerSession, RunnerCommandRequest, RunnerCommandResult, RunnerDestinationId,
    RunnerFileReadRequest, RunnerFileReadResult, RunnerFileWriteRequest, RunnerPortRequest,
    RunnerPortResult, RunnerSessionState, RunnerSnapshotRef,
};

use crate::client::{CelestoClient, Computer};
use crate::config::{CleanupMode, CelestoConfig, PROVIDER_ID};

#[derive(Debug)]
pub struct CelestoRunnerSession {
    destination_id: RunnerDestinationId,
    client: CelestoClient,
    config: CelestoConfig,
    computer: Computer,
}

impl CelestoRunnerSession {
    pub fn new(
        destination_id: RunnerDestinationId,
        client: CelestoClient,
        config: CelestoConfig,
        computer: Computer,
    ) -> Self {
        Self {
            destination_id,
            client,
            config,
            computer,
        }
    }

    pub fn from_state(
        state: RunnerSessionState,
        client: CelestoClient,
        config: CelestoConfig,
        computer: Computer,
    ) -> Self {
        Self {
            destination_id: state.destination_id,
            client,
            config,
            computer,
        }
    }
}

#[async_trait::async_trait]
impl RemoteRunnerSession for CelestoRunnerSession {
    fn state(&self) -> RunnerSessionState {
        let metadata = serde_json::json!({
            "base_url": self.config.base_url,
            "computer_id": self.computer.id,
            "computer_name": self.computer.name,
            "working_dir": self.config.working_dir,
            "cleanup": self.config.cleanup.as_str(),
            "template_id": self.config.template_id,
        });
        RunnerSessionState {
            provider_id: PROVIDER_ID.to_string(),
            session_id: self.computer.id.clone(),
            destination_id: self.destination_id.clone(),
            snapshot: None,
            metadata,
        }
    }

    async fn run_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        self.client.run_command(&self.computer.id, request).await
    }

    async fn read_file(
        &self,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        self.client.read_file(&self.computer.id, request).await
    }

    async fn write_file(&self, request: RunnerFileWriteRequest) -> anyhow::Result<()> {
        self.client.write_file(&self.computer.id, request).await
    }

    async fn expose_port(&self, request: RunnerPortRequest) -> anyhow::Result<RunnerPortResult> {
        self.client.expose_port(&self.computer.id, request).await
    }

    async fn snapshot(&self) -> anyhow::Result<Option<RunnerSnapshotRef>> {
        // The Celesto Computers API persists state via stop/start rather than
        // exposing explicit snapshots, so there is nothing to capture here.
        Ok(None)
    }

    async fn close(&self) -> anyhow::Result<()> {
        match self.config.cleanup {
            CleanupMode::DeleteOnClose => self.client.delete_computer(&self.computer.id).await,
            CleanupMode::StopOnClose => self.client.stop_computer(&self.computer.id).await,
            CleanupMode::Keep => Ok(()),
        }
    }
}
