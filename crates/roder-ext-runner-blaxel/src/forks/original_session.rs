//! Adopt a surviving original VM without the generic provision/rejoin lifecycle.
use super::{BlaxelForkClient, ForkError, OriginalSourceObservation};
use crate::{BlaxelClient, BlaxelConfig, BlaxelRunnerSession, CleanupMode};

/// Local settings only; provider/runtime/account pins come from checked reads.
/// Cleanup remains caller-owned: adoption neither claims nor deletes the VM.
pub struct OriginalSessionOptions {
    pub destination_id: String,
    pub working_dir: String,
}

impl BlaxelForkClient {
    /// Reobserve the original sandbox and compare it with a prior scoped read.
    /// Only an unchanged RUNNING source may be adopted. This method sends one
    /// non-secret GET; it never creates, wakes, reconfigures or starts a process.
    /// The caller must still check source authority and native controller identity
    /// before private delivery. This is not a browser/control or cleanup grant.
    pub async fn adopt_original_source(
        &self,
        expected: &OriginalSourceObservation,
        options: &OriginalSessionOptions,
    ) -> Result<BlaxelRunnerSession, ForkError> {
        if expected.workspace() != self.workspace
            || expected.control_base_url() != self.base_url
            || !expected.is_running()
            || options.destination_id.is_empty()
            || options.destination_id.len() > 512
            || options.destination_id.chars().any(char::is_control)
            || !options.working_dir.starts_with('/')
            || options.working_dir.len() > 4096
            || options.working_dir.chars().any(char::is_control)
        {
            return Err(ForkError::rejected("original session scope unavailable"));
        }
        let current = self
            .observe_original_source(expected.name(), expected.external_id())
            .await?;
        if !current.is_running()
            || current.endpoint_url() != expected.endpoint_url()
            || !expected.runtime().matches(current.runtime())
        {
            return Err(ForkError::rejected(
                "original source changed before adoption",
            ));
        }
        let pins = current.runtime();
        let config = BlaxelConfig {
            token: self.token.clone(),
            workspace: Some(self.workspace.clone()),
            base_url: self.base_url.clone(),
            sandbox_name: Some(current.name().into()),
            external_id: Some(current.external_id().into()),
            sandbox_name_prefix: "original".into(),
            image: pins.image.clone(),
            memory_mb: pins.memory_mb,
            ports: pins.ports.clone(),
            region: Some(pins.region.clone()),
            egress_gateway: None,
            egress_workspace: None,
            ttl: None,
            standby_after_seconds: None,
            lifecycle: None,
            working_dir: options.working_dir.clone(),
            cleanup: CleanupMode::Keep,
        };
        Ok(BlaxelRunnerSession::new(
            BlaxelClient::new(self.http.clone(), &config),
            &config,
            options.destination_id.clone(),
            current.name().into(),
            Some(current.external_id().into()),
            current.endpoint_url().trim_end_matches('/').into(),
            false,
        )
        .with_explicit_workspace(true))
    }
}
