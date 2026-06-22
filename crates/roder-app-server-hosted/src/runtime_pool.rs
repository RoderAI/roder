//! Per-tenant runtime pool (roadmap phase 72, Task 3).
//!
//! Hosted tenancy in Roder is **runtime-per-tenant**: each tenant gets its
//! own `Runtime` + `AppServer` built by a host-supplied factory that roots
//! every store under the tenant's private data directory (thread stores,
//! artifact stores, memories, automation databases) or a tenant-scoped
//! handle of a shared pool (PostgreSQL). Tenant ids come exclusively from
//! the authenticated request context — request payloads never pick a
//! tenant. Isolation is therefore by construction: tenant B's app-server
//! cannot list, read, mutate, or emit notifications for state it has never
//! seen, and automation leases live in per-tenant databases that other
//! tenants cannot reach.
//!
//! The pool also owns idle eviction: tenants with no in-flight gateway
//! requests and no active turns are dropped after `idle_ttl`, releasing
//! their runtime without interrupting active work.

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use roder_app_server::AppServer;

/// How a hosted deployment builds and guards tenant runtimes.
#[derive(Clone)]
pub struct HostedRuntimeProfile {
    /// Root under which each tenant gets a private data directory.
    pub data_root: PathBuf,
    /**
     * Hosted servers must not let tenants execute against arbitrary host
     * paths: when false (the hosted default), `workspace/create` and any
     * `thread/start` naming a `cwd` are denied at the gateway and
     * workspace execution requires a configured runner destination.
     * Tests and single-box installs may opt into local workspaces.
     */
    pub allow_local_workspaces: bool,
    /// Tenant runtimes idle longer than this (no in-flight requests, no
    /// active turns) are evicted.
    pub idle_ttl: Duration,
}

impl Default for HostedRuntimeProfile {
    fn default() -> Self {
        Self {
            data_root: std::env::temp_dir().join("roder-hosted"),
            allow_local_workspaces: false,
            idle_ttl: Duration::from_secs(15 * 60),
        }
    }
}

/// Builds a tenant's app-server given `(tenant_id, tenant_data_dir)`.
pub type TenantAppServerFactory = Arc<
    dyn Fn(String, PathBuf) -> Pin<Box<dyn Future<Output = anyhow::Result<Arc<AppServer>>> + Send>>
        + Send
        + Sync,
>;

struct TenantEntry {
    server: Arc<AppServer>,
    in_flight: Arc<AtomicUsize>,
    last_used: Instant,
}

pub struct HostedRuntimePool {
    profile: HostedRuntimeProfile,
    factory: TenantAppServerFactory,
    tenants: Mutex<HashMap<String, TenantEntry>>,
}

/// RAII guard counting an in-flight request against a tenant runtime.
pub struct TenantLease {
    pub server: Arc<AppServer>,
    in_flight: Arc<AtomicUsize>,
}

impl Drop for TenantLease {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

impl HostedRuntimePool {
    pub fn new(profile: HostedRuntimeProfile, factory: TenantAppServerFactory) -> Self {
        Self {
            profile,
            factory,
            tenants: Mutex::new(HashMap::new()),
        }
    }

    pub fn profile(&self) -> &HostedRuntimeProfile {
        &self.profile
    }

    /// Resolves (lazily creating) the tenant's app-server and counts an
    /// in-flight request against it until the lease drops.
    pub async fn lease(&self, tenant_id: &str) -> anyhow::Result<TenantLease> {
        let mut tenants = self.tenants.lock().await;
        if !tenants.contains_key(tenant_id) {
            let data_dir = self.profile.data_root.join(sanitize_tenant_dir(tenant_id));
            std::fs::create_dir_all(&data_dir)?;
            let server = (self.factory)(tenant_id.to_string(), data_dir).await?;
            tenants.insert(
                tenant_id.to_string(),
                TenantEntry {
                    server,
                    in_flight: Arc::new(AtomicUsize::new(0)),
                    last_used: Instant::now(),
                },
            );
        }
        let entry = tenants.get_mut(tenant_id).expect("just inserted");
        entry.last_used = Instant::now();
        entry.in_flight.fetch_add(1, Ordering::SeqCst);
        Ok(TenantLease {
            server: entry.server.clone(),
            in_flight: entry.in_flight.clone(),
        })
    }

    /// Number of live tenant runtimes.
    pub async fn len(&self) -> usize {
        self.tenants.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.tenants.lock().await.is_empty()
    }

    /**
     * Evicts tenants idle past `idle_ttl` with no in-flight requests and
     * no active turns; returns evicted tenant ids. Active work is never
     * interrupted: a tenant with a running turn stays resident regardless
     * of how long ago its last request finished.
     */
    pub async fn evict_idle(&self) -> Vec<String> {
        let mut evicted = Vec::new();
        let mut tenants = self.tenants.lock().await;
        let mut keep = HashMap::new();
        for (tenant_id, entry) in tenants.drain() {
            let idle = entry.last_used.elapsed() >= self.profile.idle_ttl;
            let busy = entry.in_flight.load(Ordering::SeqCst) > 0
                || entry.server.runtime.active_turn_count().await > 0;
            if idle && !busy {
                evicted.push(tenant_id);
            } else {
                keep.insert(tenant_id, entry);
            }
        }
        *tenants = keep;
        evicted
    }

    /// Graceful shutdown: waits (bounded) for active turns to finish, then
    /// drops all tenant runtimes.
    pub async fn shutdown(&self, max_wait: Duration) {
        let deadline = Instant::now() + max_wait;
        loop {
            let busy = {
                let tenants = self.tenants.lock().await;
                let mut busy = false;
                for entry in tenants.values() {
                    if entry.in_flight.load(Ordering::SeqCst) > 0
                        || entry.server.runtime.active_turn_count().await > 0
                    {
                        busy = true;
                        break;
                    }
                }
                busy
            };
            if !busy || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.tenants.lock().await.clear();
    }
}

/// Tenant ids map onto directory names defensively (they originate from
/// operator-configured registries, but path separators must never appear).
fn sanitize_tenant_dir(tenant_id: &str) -> String {
    tenant_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_dirs_are_path_safe() {
        assert_eq!(sanitize_tenant_dir("tenant-a"), "tenant-a");
        assert_eq!(sanitize_tenant_dir("../evil/../x"), "___evil____x");
    }
}
