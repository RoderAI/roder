use std::sync::Arc;

use crate::cancellation::CANCELLATION_DIR;
use crate::{
    BlaxelClient, BlaxelConfig, BlaxelRunnerSession, BlaxelWorkspace, PROVIDER_ID, TOKEN_ENV,
    USER_AGENT, WORKSPACE_ENV, client, config, egress, sanitize_name, wait_until_ready,
};
use async_trait::async_trait;
use roder_api::remote_runner::{
    RemoteRunnerProvider, RemoteRunnerProviderId, RemoteRunnerSession, RunnerCapabilities,
    RunnerDestination, RunnerMountCapabilities, RunnerSessionState,
};

const READINESS_ATTEMPTS: u32 = 20;

#[derive(Debug)]
pub struct BlaxelRunnerProvider {
    http: reqwest::Client,
    workspace: Option<BlaxelWorkspace>,
}

impl Default for BlaxelRunnerProvider {
    fn default() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(
                client::HTTP_REQUEST_TIMEOUT_SECONDS,
            ))
            .build()
            .expect("build Blaxel HTTP client");
        Self {
            http,
            workspace: None,
        }
    }
}

impl BlaxelRunnerProvider {
    /// Keep source creation, rejoin and cleanup on one operator-selected account.
    /// The default provider remains independently configured for other workloads.
    pub fn for_workspace(workspace: BlaxelWorkspace) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(
                client::HTTP_REQUEST_TIMEOUT_SECONDS,
            ))
            .build()?;
        Ok(Self {
            http,
            workspace: Some(workspace),
        })
    }

    fn destination_config(&self, destination: &RunnerDestination) -> anyhow::Result<BlaxelConfig> {
        match &self.workspace {
            Some(profile) => BlaxelConfig::from_destination_in_workspace(destination, profile),
            None => BlaxelConfig::from_destination(destination),
        }
    }

    fn state_config(&self, state: &RunnerSessionState) -> anyhow::Result<BlaxelConfig> {
        match &self.workspace {
            Some(profile) => BlaxelConfig::from_state_in_workspace(state, profile),
            None => BlaxelConfig::from_state(state),
        }
    }

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
            cancellation: true,
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
        if self.workspace.is_some() {
            return None;
        }
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
        let config = self.destination_config(destination)?;
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
        let mut config = self.destination_config(&destination)?;
        let external_id = config
            .external_id
            .clone()
            .unwrap_or_else(|| destination.id.clone());
        config.external_id = Some(external_id.clone());
        let name = config.sandbox_name.clone().unwrap_or_else(|| {
            sanitize_name(&format!("{}-{}", config.sandbox_name_prefix, external_id))
        });
        config.sandbox_name = Some(name.clone());

        let client = self.client(&config);
        let sandbox = client.create_sandbox(&name, &config).await?;
        egress::verify(&sandbox, &config)?;
        let sandbox = if let Some(lifecycle) = &config.lifecycle {
            // `createIfNotExist=true` may have returned a persistent sandbox;
            // reconcile mutable lifecycle policy without replacing it.
            client
                .update_sandbox_lifecycle(sandbox.name(), lifecycle)
                .await?
        } else {
            sandbox
        };
        let (sandbox, endpoint) = wait_until_ready(&client, sandbox, READINESS_ATTEMPTS).await?;
        if let Some(profile) = &self.workspace {
            profile.validate_endpoint(&endpoint)?;
        }
        client.make_dir(&endpoint, &config.working_dir).await.ok();
        client.make_dir(&endpoint, CANCELLATION_DIR).await?;

        let session = Arc::new(
            BlaxelRunnerSession::new(
                client,
                &config,
                destination.id,
                sandbox.metadata.name,
                Some(external_id),
                endpoint,
                false,
            )
            .with_explicit_workspace(self.workspace.is_some()),
        );
        session.refresh_standby_grace().await?;
        Ok(session)
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
        anyhow::ensure!(
            state.metadata.get("fork_adopted").is_none(),
            "fork sessions require explicit adoption with current ownership and runtime pins"
        );
        let config = self.state_config(&state)?;
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

        egress::verify(&sandbox, &config)?;
        let sandbox = if let Some(lifecycle) = &config.lifecycle {
            // Mutable lifecycle policy is intentionally reapplied on every
            // rejoin; this preserves the sandbox generation and its checkout.
            client
                .update_sandbox_lifecycle(sandbox.name(), lifecycle)
                .await?
        } else {
            sandbox
        };
        let external_id = sandbox
            .metadata
            .external_id
            .clone()
            .or(config.external_id.clone());
        let (sandbox, endpoint) = wait_until_ready(&client, sandbox, READINESS_ATTEMPTS).await?;
        if let Some(profile) = &self.workspace {
            profile.validate_endpoint(&endpoint)?;
        }
        client.make_dir(&endpoint, CANCELLATION_DIR).await?;
        let session = Arc::new(
            BlaxelRunnerSession::new(
                client,
                &config,
                state.destination_id,
                sandbox.metadata.name,
                external_id,
                endpoint,
                false,
            )
            .with_explicit_workspace(self.workspace.is_some()),
        );
        session.refresh_standby_grace().await?;
        Ok(session)
    }
}
