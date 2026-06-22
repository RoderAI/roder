//! Hosted credential validation.
//!
//! Two credential families ship now, both validated before any JSON-RPC
//! dispatch:
//!
//! - **Static test keys** (`rk_test_*`): fixture credentials for tests and
//!   local hosted-mode development; mapped explicitly to a principal.
//! - **Service-account API keys** (`rk_sa_<key-id>.<secret>`): only a
//!   SHA-256 hash of the secret is stored; records carry expiry and a
//!   revocation flag.
//!
//! External-IdP JWT/JWKS validation is a deliberate follow-up: it needs a
//! JWT dependency decision and an IdP to test against, and the gateway
//! seam (`HostedAuthenticator::authenticate`) is where it slots in.

use std::collections::BTreeMap;
use std::sync::RwLock;

use roder_api::identity::{
    HostedRequestContext, HostedRole, HostedScope, PrincipalContext, TenantContext, TenantId,
};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostedAuthError {
    /// Unknown, malformed, or wrong-secret credential.
    Invalid,
    Expired,
    Revoked,
    UnknownTenant,
}

impl std::fmt::Display for HostedAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            HostedAuthError::Invalid => "invalid credential",
            HostedAuthError::Expired => "credential expired",
            HostedAuthError::Revoked => "credential revoked",
            HostedAuthError::UnknownTenant => "unknown tenant",
        };
        f.write_str(text)
    }
}

/// Identity seeded behind a credential.
#[derive(Debug, Clone)]
pub struct PrincipalSeed {
    pub tenant_id: TenantId,
    pub principal: PrincipalContext,
    pub role: HostedRole,
    pub scopes: Vec<HostedScope>,
}

struct ServiceAccountRecord {
    seed: PrincipalSeed,
    secret_sha256: String,
    expires_at: Option<OffsetDateTime>,
    revoked: bool,
}

/// A freshly minted service-account key. The secret is shown once and only
/// its hash is retained.
#[derive(Debug, Clone)]
pub struct ServiceAccountKey {
    pub key_id: String,
    /// Full bearer token (`rk_sa_<key-id>.<secret>`).
    pub token: String,
}

#[derive(Default)]
pub struct HostedAuthenticator {
    static_keys: RwLock<BTreeMap<String, PrincipalSeed>>,
    service_accounts: RwLock<BTreeMap<String, ServiceAccountRecord>>,
}

fn sha256_hex(input: &str) -> String {
    Sha256::digest(input.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl HostedAuthenticator {
    /// Registers a static test key (must start with `rk_test_`).
    pub fn register_static_key(&self, token: &str, seed: PrincipalSeed) -> anyhow::Result<()> {
        anyhow::ensure!(
            token.starts_with("rk_test_") && token.len() >= 16,
            "static keys must start with rk_test_ and be at least 16 chars"
        );
        self.static_keys
            .write()
            .unwrap()
            .insert(token.to_string(), seed);
        Ok(())
    }

    /// Mints a service-account key; only the secret hash is stored.
    pub fn mint_service_account_key(
        &self,
        seed: PrincipalSeed,
        expires_at: Option<OffsetDateTime>,
    ) -> ServiceAccountKey {
        let key_id = uuid::Uuid::new_v4().simple().to_string();
        let secret = uuid::Uuid::new_v4().simple().to_string();
        let token = format!("rk_sa_{key_id}.{secret}");
        self.service_accounts.write().unwrap().insert(
            key_id.clone(),
            ServiceAccountRecord {
                seed,
                secret_sha256: sha256_hex(&secret),
                expires_at,
                revoked: false,
            },
        );
        ServiceAccountKey { key_id, token }
    }

    /// Revokes a service-account key by key id.
    pub fn revoke_service_account_key(&self, key_id: &str) -> bool {
        match self.service_accounts.write().unwrap().get_mut(key_id) {
            Some(record) => {
                record.revoked = true;
                true
            }
            None => false,
        }
    }

    /// Validates a bearer token into a resolved request context.
    pub fn authenticate(
        &self,
        token: &str,
        tenants: &super::tenant::TenantRegistry,
        now: OffsetDateTime,
    ) -> Result<HostedRequestContext, HostedAuthError> {
        let (seed, credential_id) = if token.starts_with("rk_test_") {
            let keys = self.static_keys.read().unwrap();
            let seed = keys.get(token).ok_or(HostedAuthError::Invalid)?.clone();
            // Static keys audit-correlate by a hash prefix, never the key.
            let credential = format!("static:{}", &sha256_hex(token)[..12]);
            (seed, credential)
        } else if let Some(rest) = token.strip_prefix("rk_sa_") {
            let (key_id, secret) = rest.split_once('.').ok_or(HostedAuthError::Invalid)?;
            let accounts = self.service_accounts.read().unwrap();
            let record = accounts.get(key_id).ok_or(HostedAuthError::Invalid)?;
            if record.secret_sha256 != sha256_hex(secret) {
                return Err(HostedAuthError::Invalid);
            }
            if record.revoked {
                return Err(HostedAuthError::Revoked);
            }
            if record.expires_at.is_some_and(|expiry| now >= expiry) {
                return Err(HostedAuthError::Expired);
            }
            (record.seed.clone(), format!("sa:{key_id}"))
        } else {
            return Err(HostedAuthError::Invalid);
        };

        let tenant: TenantContext = tenants
            .get(&seed.tenant_id)
            .ok_or(HostedAuthError::UnknownTenant)?;
        Ok(HostedRequestContext {
            tenant,
            principal: seed.principal,
            role: seed.role,
            scopes: seed.scopes,
            credential_id: Some(credential_id),
            authenticated_at: now,
        })
    }
}
