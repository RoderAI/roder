//! `[hosted]` service configuration (roadmap phase 72, Task 5).
//!
//! Configures the hosted multi-tenant gateway: listener, tenant registry,
//! static auth keys (always env references — raw secrets are rejected),
//! runtime profile, rate limits, and audit. Local single-user Roder
//! ignores this block entirely. Env overrides use `RODER_HOSTED_*` names.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostedConfig {
    /// Gateway listen address (`host:port`). TLS terminates at the load
    /// balancer; never expose the plain listener directly.
    #[serde(default = "default_listen")]
    pub listen: String,
    /// Public URL clients use (for docs/hooks), e.g. `wss://roder.example.com`.
    #[serde(default)]
    pub public_url: Option<String>,
    /// Root directory for per-tenant runtime data.
    pub data_root: String,
    /// Allow tenants to execute against host-local workspaces. Hosted
    /// default is false: execution requires a configured runner
    /// destination. Single-box/test installs may opt in.
    #[serde(default)]
    pub allow_local_workspaces: bool,
    /// Tenant runtimes idle longer than this many seconds are evicted.
    #[serde(default = "default_idle_ttl_secs")]
    pub idle_ttl_secs: u64,
    #[serde(default)]
    pub tenants: Vec<HostedTenantConfig>,
    /// Static credentials; secrets are env references, never inline.
    #[serde(default)]
    pub static_keys: Vec<HostedStaticKeyConfig>,
    #[serde(default)]
    pub rate_limit: HostedRateLimitConfig,
    /// JSONL audit log path; defaults under the data root.
    #[serde(default)]
    pub audit_log: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostedTenantConfig {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostedStaticKeyConfig {
    /// Env var holding the `rk_test_*` key. Raw keys in config are
    /// rejected at validation.
    pub token_env: String,
    pub tenant: String,
    pub user: String,
    /// `member`, `tenant_admin`, or `system_admin`.
    #[serde(default = "default_role")]
    pub role: String,
    /// Subset of `read`, `write`, `admin`.
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostedRateLimitConfig {
    #[serde(default = "default_burst")]
    pub burst: u32,
    #[serde(default = "default_per_second")]
    pub per_second: f64,
    #[serde(default = "default_max_request_bytes")]
    pub max_request_bytes: usize,
}

impl Default for HostedRateLimitConfig {
    fn default() -> Self {
        Self {
            burst: default_burst(),
            per_second: default_per_second(),
            max_request_bytes: default_max_request_bytes(),
        }
    }
}

fn default_listen() -> String {
    "127.0.0.1:7900".to_string()
}
fn default_idle_ttl_secs() -> u64 {
    900
}
fn default_role() -> String {
    "member".to_string()
}
fn default_scopes() -> Vec<String> {
    vec!["read".to_string(), "write".to_string()]
}
fn default_burst() -> u32 {
    60
}
fn default_per_second() -> f64 {
    10.0
}
fn default_max_request_bytes() -> usize {
    1024 * 1024
}

impl HostedConfig {
    /// Applies `RODER_HOSTED_*` env overrides and validates.
    pub fn resolved(mut self) -> anyhow::Result<Self> {
        if let Ok(listen) = std::env::var("RODER_HOSTED_LISTEN")
            && !listen.trim().is_empty()
        {
            self.listen = listen;
        }
        if let Ok(data_root) = std::env::var("RODER_HOSTED_DATA_ROOT")
            && !data_root.trim().is_empty()
        {
            self.data_root = data_root;
        }
        if let Ok(allow) = std::env::var("RODER_HOSTED_ALLOW_LOCAL_WORKSPACES") {
            self.allow_local_workspaces = allow == "1" || allow.eq_ignore_ascii_case("true");
        }
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.data_root.trim().is_empty(), "hosted.data_root is required");
        anyhow::ensure!(!self.tenants.is_empty(), "at least one [[hosted.tenants]] is required");
        for key in &self.static_keys {
            anyhow::ensure!(
                !key.token_env.starts_with("rk_"),
                "hosted.static_keys.token_env must name an env var, not a raw key; \
                 raw secrets are never accepted in config"
            );
            anyhow::ensure!(
                self.tenants.iter().any(|tenant| tenant.id == key.tenant),
                "static key references unknown tenant {:?}",
                key.tenant
            );
            anyhow::ensure!(
                matches!(key.role.as_str(), "member" | "tenant_admin" | "system_admin"),
                "unknown hosted role {:?}",
                key.role
            );
            for scope in &key.scopes {
                anyhow::ensure!(
                    matches!(scope.as_str(), "read" | "write" | "admin"),
                    "unknown hosted scope {scope:?}"
                );
            }
        }
        Ok(())
    }

    /// Redacted summary for logs: never echoes env values.
    pub fn redacted_summary(&self) -> String {
        format!(
            "hosted gateway on {} ({} tenant(s), {} static key env ref(s), local workspaces {})",
            self.listen,
            self.tenants.len(),
            self.static_keys.len(),
            if self.allow_local_workspaces { "ALLOWED" } else { "disabled" },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HostedConfig {
        toml::from_str(
            r#"
            listen = "0.0.0.0:7900"
            data_root = "/var/lib/roder-hosted"

            [[tenants]]
            id = "acme"
            display_name = "Acme"

            [[static_keys]]
            token_env = "RODER_HOSTED_KEY_ACME_ADMIN"
            tenant = "acme"
            user = "ops"
            role = "tenant_admin"
            scopes = ["read", "write", "admin"]
            "#,
        )
        .unwrap()
    }

    #[test]
    fn hosted_config_parses_validates_and_redacts() {
        let config = sample();
        config.validate().unwrap();
        assert!(!config.allow_local_workspaces, "hosted default is locked down");
        assert_eq!(config.rate_limit.burst, 60);
        let summary = config.redacted_summary();
        assert!(summary.contains("1 tenant(s)"));
        assert!(!summary.contains("RODER_HOSTED_KEY"), "no env names in summaries");
    }

    #[test]
    fn hosted_config_rejects_raw_keys_and_unknown_references() {
        let mut config = sample();
        config.static_keys[0].token_env = "rk_test_raw_key_inline".to_string();
        assert!(config.validate().unwrap_err().to_string().contains("never accepted"));

        let mut config = sample();
        config.static_keys[0].tenant = "ghost".to_string();
        assert!(config.validate().unwrap_err().to_string().contains("unknown tenant"));

        let mut config = sample();
        config.static_keys[0].scopes = vec!["root".to_string()];
        assert!(config.validate().unwrap_err().to_string().contains("unknown hosted scope"));
    }
}
