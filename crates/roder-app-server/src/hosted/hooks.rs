//! Hosted hook store and dispatch (roadmap phase 72, Task 4).
//!
//! Hooks are tenant-scoped webhook subscriptions over canonical event
//! kinds. Secrets never enter the store: `signing_secret_ref` is an
//! `env:NAME` reference resolved only at delivery time, so listings,
//! audit records, and delivery logs are redacted by construction.

use std::collections::HashMap;
use std::sync::RwLock;

use roder_api::hosted_hooks::{HookId, HostedHookDefinition};
use time::OffsetDateTime;

/// Tenant-scoped hook CRUD. Every operation takes the authenticated
/// tenant id; cross-tenant access is impossible through this API.
#[derive(Default)]
pub struct HookStore {
    hooks: RwLock<HashMap<(String, HookId), HostedHookDefinition>>,
}

impl HookStore {
    pub fn create(
        &self,
        tenant_id: &str,
        mut definition: HostedHookDefinition,
    ) -> anyhow::Result<HostedHookDefinition> {
        anyhow::ensure!(
            definition.url.starts_with("https://") || definition.url.starts_with("http://"),
            "hook url must be an http(s) URL"
        );
        if let Some(secret_ref) = &definition.signing_secret_ref {
            anyhow::ensure!(
                secret_ref.starts_with("env:"),
                "signing secrets are referenced as env:NAME; raw secrets are not accepted"
            );
        }
        // The owning tenant comes from the authenticated context only.
        definition.tenant_id = Some(tenant_id.to_string());
        definition.scope = roder_api::hosted_hooks::HookScope::Tenant;
        let now = OffsetDateTime::now_utc();
        definition.created_at = now;
        definition.updated_at = now;
        self.hooks
            .write()
            .unwrap()
            .insert((tenant_id.to_string(), definition.id.clone()), definition.clone());
        Ok(definition)
    }

    pub fn list(&self, tenant_id: &str) -> Vec<HostedHookDefinition> {
        let mut hooks: Vec<_> = self
            .hooks
            .read()
            .unwrap()
            .iter()
            .filter(|((tenant, _), _)| tenant == tenant_id)
            .map(|(_, hook)| hook.clone())
            .collect();
        hooks.sort_by(|a, b| a.id.cmp(&b.id));
        hooks
    }

    pub fn get(&self, tenant_id: &str, hook_id: &str) -> Option<HostedHookDefinition> {
        self.hooks
            .read()
            .unwrap()
            .get(&(tenant_id.to_string(), hook_id.to_string()))
            .cloned()
    }

    pub fn delete(&self, tenant_id: &str, hook_id: &str) -> bool {
        self.hooks
            .write()
            .unwrap()
            .remove(&(tenant_id.to_string(), hook_id.to_string()))
            .is_some()
    }

    /// Enabled hooks of a tenant matching an event kind.
    pub fn matching(&self, tenant_id: &str, event_kind: &str) -> Vec<HostedHookDefinition> {
        self.list(tenant_id)
            .into_iter()
            .filter(|hook| hook.matches(event_kind))
            .collect()
    }
}

/// Resolves a `env:NAME` signing-secret reference at delivery time.
pub fn resolve_signing_secret(secret_ref: &str) -> anyhow::Result<String> {
    let Some(name) = secret_ref.strip_prefix("env:") else {
        anyhow::bail!("unsupported signing secret reference (expected env:NAME)");
    };
    std::env::var(name).map_err(|_| {
        anyhow::anyhow!("signing secret env var {name} is not set (reference stays redacted)")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::hosted_hooks::HookScope;

    fn definition(id: &str) -> HostedHookDefinition {
        HostedHookDefinition {
            id: id.to_string(),
            scope: HookScope::Tenant,
            tenant_id: None,
            event_kinds: vec!["turn.".to_string()],
            url: "http://127.0.0.1:1/hook".to_string(),
            signing_secret_ref: Some("env:RODER_TEST_HOOK_SECRET".to_string()),
            enabled: true,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn hook_store_is_tenant_scoped() {
        let store = HookStore::default();
        store.create("tenant-a", definition("hook-1")).unwrap();
        store.create("tenant-b", definition("hook-2")).unwrap();

        assert_eq!(store.list("tenant-a").len(), 1);
        assert!(store.get("tenant-b", "hook-1").is_none());
        assert!(!store.delete("tenant-b", "hook-1"));
        assert!(store.delete("tenant-a", "hook-1"));

        // Owning tenant always comes from the caller context.
        let created = store.create("tenant-a", definition("hook-3")).unwrap();
        assert_eq!(created.tenant_id.as_deref(), Some("tenant-a"));
    }

    #[test]
    fn raw_secrets_are_rejected_at_creation() {
        let store = HookStore::default();
        let mut bad = definition("hook-raw");
        bad.signing_secret_ref = Some("super-secret-value".to_string());
        let error = store.create("tenant-a", bad).unwrap_err().to_string();
        assert!(error.contains("env:NAME"), "{error}");
    }
}
