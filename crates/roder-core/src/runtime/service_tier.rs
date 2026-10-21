use super::*;

impl Runtime {
    /// Persist a thread's default inference service tier. None restores the
    /// provider default. An active turn retains its captured settings.
    pub async fn set_thread_service_tier(
        &self,
        thread_id: &ThreadId,
        service_tier: Option<String>,
    ) -> anyhow::Result<Option<String>> {
        let store = self
            .thread_store
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("thread service tiers require a thread store"))?;
        let mut metadata = store
            .load_thread_metadata(thread_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
        let service_tier = service_tier.map(|tier| tier.trim().to_string());
        anyhow::ensure!(
            service_tier.as_ref().is_none_or(|tier| !tier.is_empty()),
            "service tier must be non-empty, or null to restore the provider default"
        );
        metadata.service_tier = service_tier.clone();
        metadata.updated_at = OffsetDateTime::now_utc();
        store.update_thread_metadata(metadata).await?;
        Ok(service_tier)
    }
}
