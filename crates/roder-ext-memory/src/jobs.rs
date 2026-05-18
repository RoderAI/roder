use roder_api::memory::{MemoryJobLease, MemoryProviderSelection, MemoryScope};
use time::{Duration, OffsetDateTime};

pub fn reembed_job(
    scope: Option<MemoryScope>,
    provider: MemoryProviderSelection,
    lease_seconds: i64,
) -> MemoryJobLease {
    MemoryJobLease {
        id: format!("reembed-{}", uuid::Uuid::new_v4()),
        scope_id: scope
            .map(|scope| scope.stable_id())
            .unwrap_or_else(|| "all".to_string()),
        provider_id: provider.provider_id,
        model: provider.model,
        leased_until: OffsetDateTime::now_utc() + Duration::seconds(lease_seconds.max(1)),
        attempts: 0,
        metadata: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_create_reembed_lease() {
        let lease = reembed_job(
            Some(MemoryScope::Global),
            MemoryProviderSelection {
                provider_id: "fake".to_string(),
                model: "fake-vector-32".to_string(),
            },
            30,
        );
        assert_eq!(lease.scope_id, "global");
    }
}
