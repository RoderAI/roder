//! Hosted multi-tenant Roder service distribution (roadmap phase 72,
//! Task 5): assembles the hosted gateway from `HostedConfig` — tenant
//! registry, static-key authenticator (env references only), per-tenant
//! runtime pool, rate limits, and JSONL audit. The TUI is deliberately not
//! part of this distribution; clients connect over the gateway protocol.

use std::sync::Arc;

use roder_api::identity::{HostedRole, HostedScope, PrincipalContext, TenantContext};
use roder_app_server::hosted::auth::PrincipalSeed;
use roder_app_server::hosted::{
    AuditLog, HookDeliveryService, HookStore, HostedAuthenticator, HostedGatewayController,
    HostedGatewayOptions, HostedRuntimePool, HostedRuntimeProfile, RateLimitConfig,
    TenantRegistry, serve_hosted_gateway,
};
use roder_app_server::hosted::runtime_pool::TenantAppServerFactory;
use roder_config::hosted::HostedConfig;

/// Builds the authenticator and tenant registry from config; static-key
/// secrets are resolved from the referenced env vars at startup and never
/// logged.
pub fn auth_from_config(
    config: &HostedConfig,
) -> anyhow::Result<(Arc<HostedAuthenticator>, Arc<TenantRegistry>)> {
    let tenants = Arc::new(TenantRegistry::default());
    for tenant in &config.tenants {
        tenants.insert(TenantContext {
            tenant_id: tenant.id.clone(),
            display_name: tenant.display_name.clone(),
        });
    }
    let authenticator = Arc::new(HostedAuthenticator::default());
    for key in &config.static_keys {
        let Ok(token) = std::env::var(&key.token_env) else {
            anyhow::bail!(
                "hosted static key env var {} is not set; refusing to start with a \
                 partially-configured authenticator",
                key.token_env
            );
        };
        let role = match key.role.as_str() {
            "tenant_admin" => HostedRole::TenantAdmin,
            "system_admin" => HostedRole::SystemAdmin,
            _ => HostedRole::Member,
        };
        let scopes = key
            .scopes
            .iter()
            .map(|scope| match scope.as_str() {
                "admin" => HostedScope::Admin,
                "write" => HostedScope::Write,
                _ => HostedScope::Read,
            })
            .collect();
        authenticator.register_static_key(
            &token,
            PrincipalSeed {
                tenant_id: key.tenant.clone(),
                principal: PrincipalContext::User {
                    user_id: key.user.clone(),
                    display_name: None,
                },
                role,
                scopes,
            },
        )?;
    }
    Ok((authenticator, tenants))
}

/// Default per-tenant runtime factory: the full default extension registry
/// (providers resolve credentials from the process env; hosted runner
/// destinations are configured per deployment) with a private JSONL thread
/// store under the tenant data dir.
pub fn default_tenant_factory() -> TenantAppServerFactory {
    Arc::new(|_tenant_id, data_dir| {
        Box::pin(async move {
            let registry_config = roder_extension_host::DefaultRegistryConfig {
                thread_dir: Some(data_dir.join("threads")),
                ..Default::default()
            };
            let registry = roder_extension_host::build_default_registry(registry_config)?;
            let runtime = Arc::new(roder_core::Runtime::new(
                registry,
                roder_core::RuntimeConfig::default(),
            )?);
            Ok(Arc::new(roder_app_server::AppServer::new(runtime)))
        })
    })
}

/// Launches the hosted gateway from config with the given tenant factory.
pub async fn launch(
    config: HostedConfig,
    factory: TenantAppServerFactory,
) -> anyhow::Result<HostedGatewayController> {
    let config = config.resolved()?;
    let (authenticator, tenants) = auth_from_config(&config)?;
    let data_root = std::path::PathBuf::from(&config.data_root);
    std::fs::create_dir_all(&data_root)?;
    let audit = Arc::new(AuditLog::with_jsonl(
        config
            .audit_log
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| data_root.join("audit.jsonl")),
    ));
    let pool = Arc::new(HostedRuntimePool::new(
        HostedRuntimeProfile {
            data_root,
            allow_local_workspaces: config.allow_local_workspaces,
            idle_ttl: std::time::Duration::from_secs(config.idle_ttl_secs),
        },
        factory,
    ));
    eprintln!("{}", config.redacted_summary());
    serve_hosted_gateway(
        pool,
        HostedGatewayOptions {
            listen: config.listen.clone(),
            authenticator,
            tenants,
            audit,
            limits: RateLimitConfig {
                burst: config.rate_limit.burst,
                per_second: config.rate_limit.per_second,
                max_request_bytes: config.rate_limit.max_request_bytes,
            },
            hooks: Arc::new(HookStore::default()),
            hook_delivery: Arc::new(HookDeliveryService::new(Default::default())),
        },
    )
    .await
}
