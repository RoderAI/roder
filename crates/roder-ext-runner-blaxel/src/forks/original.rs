use super::{BlaxelForkClient, ForkError, ForkRuntime};

/// One fresh, read-only control-plane observation of an original source.
/// This does not attest to browser identity, scrub, or backend admission. It
/// intentionally cannot expose the provider document or its environment.
pub struct OriginalSourceObservation {
    name: String,
    workspace: String,
    external_id: String,
    endpoint_url: String,
    control_base_url: String,
    runtime: ForkRuntime,
    running: bool,
}

impl OriginalSourceObservation {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn workspace(&self) -> &str {
        &self.workspace
    }
    pub fn external_id(&self) -> &str {
        &self.external_id
    }
    pub fn endpoint_url(&self) -> &str {
        &self.endpoint_url
    }
    pub fn control_base_url(&self) -> &str {
        &self.control_base_url
    }
    pub fn is_running(&self) -> bool {
        self.running
    }
    pub fn runtime(&self) -> &ForkRuntime {
        &self.runtime
    }
}

impl BlaxelForkClient {
    /// Observe one exact original source without waking it, requesting secrets,
    /// or mutating provider state. Callers must compare their session's endpoint
    /// and runtime pins and bound freshness immediately before private delivery.
    pub async fn observe_original_source(
        &self,
        source: &str,
        external_id: &str,
    ) -> Result<OriginalSourceObservation, ForkError> {
        if external_id.is_empty()
            || external_id.len() > 256
            || external_id.chars().any(char::is_control)
        {
            return Err(ForkError::rejected("invalid original source identity"));
        }
        let document = self
            .sandbox_document(source, false)
            .await?
            .ok_or_else(|| ForkError::rejected("original source is absent"))?;
        if !document.ready()? || document.metadata.external_id.as_deref() != Some(external_id) {
            return Err(ForkError::rejected("original source identity unavailable"));
        }
        let running = document.running();
        let runtime = document.runtime()?;
        let endpoint_url = document
            .metadata
            .url
            .ok_or_else(|| ForkError::rejected("original source endpoint unavailable"))?;
        crate::workspace::validate_endpoint(&self.base_url, &endpoint_url)
            .map_err(|_| ForkError::rejected("original source endpoint unavailable"))?;
        Ok(OriginalSourceObservation {
            name: document.metadata.name,
            workspace: document.metadata.workspace,
            external_id: external_id.into(),
            endpoint_url,
            control_base_url: self.base_url.clone(),
            runtime,
            running,
        })
    }
}
