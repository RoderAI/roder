//! Hosted credential validation and method authorization (phase 72,
//! Task 2). Fully offline.

use roder_api::identity::{
    AuthorizationDecision, HostedRole, HostedScope, PrincipalContext, TenantContext,
};
use roder_app_server::hosted::auth::PrincipalSeed;
use roder_app_server::hosted::{HostedAuthError, HostedAuthenticator, TenantRegistry, authorize_method};
use time::OffsetDateTime;

fn registry_with(tenant_id: &str) -> TenantRegistry {
    let tenants = TenantRegistry::default();
    tenants.insert(TenantContext {
        tenant_id: tenant_id.to_string(),
        display_name: None,
    });
    tenants
}

fn seed(tenant: &str, role: HostedRole, scopes: Vec<HostedScope>) -> PrincipalSeed {
    PrincipalSeed {
        tenant_id: tenant.to_string(),
        principal: PrincipalContext::User {
            user_id: "user-1".to_string(),
            display_name: None,
        },
        role,
        scopes,
    }
}

#[test]
fn static_keys_and_service_account_keys_authenticate() {
    let auth = HostedAuthenticator::default();
    let tenants = registry_with("tenant-a");
    let now = OffsetDateTime::UNIX_EPOCH;

    auth.register_static_key(
        "rk_test_alpha_0001",
        seed("tenant-a", HostedRole::Member, vec![HostedScope::Read]),
    )
    .unwrap();
    let context = auth
        .authenticate("rk_test_alpha_0001", &tenants, now)
        .unwrap();
    assert_eq!(context.tenant.tenant_id, "tenant-a");
    // Credential ids never contain the raw key.
    assert!(!context.credential_id.unwrap().contains("alpha"));

    let key = auth.mint_service_account_key(
        seed(
            "tenant-a",
            HostedRole::Member,
            vec![HostedScope::Read, HostedScope::Write],
        ),
        Some(now + time::Duration::hours(1)),
    );
    let context = auth.authenticate(&key.token, &tenants, now).unwrap();
    assert!(matches!(
        context.principal,
        PrincipalContext::User { .. }
    ));
    assert_eq!(context.credential_id.as_deref(), Some(&*format!("sa:{}", key.key_id)));
}

#[test]
fn invalid_expired_and_revoked_credentials_fail() {
    let auth = HostedAuthenticator::default();
    let tenants = registry_with("tenant-a");
    let now = OffsetDateTime::UNIX_EPOCH;

    assert_eq!(
        auth.authenticate("rk_test_unknown_0001", &tenants, now),
        Err(HostedAuthError::Invalid)
    );
    assert_eq!(
        auth.authenticate("totally-not-a-key", &tenants, now),
        Err(HostedAuthError::Invalid)
    );

    let key = auth.mint_service_account_key(
        seed("tenant-a", HostedRole::Member, vec![HostedScope::Read]),
        Some(now + time::Duration::minutes(5)),
    );
    // Wrong secret with a valid key id.
    let wrong = format!("{}.deadbeef", key.token.rsplit_once('.').unwrap().0);
    assert_eq!(auth.authenticate(&wrong, &tenants, now), Err(HostedAuthError::Invalid));
    // Expired.
    assert_eq!(
        auth.authenticate(&key.token, &tenants, now + time::Duration::minutes(6)),
        Err(HostedAuthError::Expired)
    );
    // Revoked.
    assert!(auth.revoke_service_account_key(&key.key_id));
    assert_eq!(auth.authenticate(&key.token, &tenants, now), Err(HostedAuthError::Revoked));

    // Unknown tenant fails even with a valid credential.
    auth.register_static_key(
        "rk_test_ghost_0001",
        seed("tenant-ghost", HostedRole::Member, vec![HostedScope::Read]),
    )
    .unwrap();
    assert_eq!(
        auth.authenticate("rk_test_ghost_0001", &tenants, now),
        Err(HostedAuthError::UnknownTenant)
    );
}

#[test]
fn authorization_maps_scopes_roles_and_methods() {
    let auth = HostedAuthenticator::default();
    let tenants = registry_with("tenant-a");
    let now = OffsetDateTime::UNIX_EPOCH;

    auth.register_static_key(
        "rk_test_reader_001",
        seed("tenant-a", HostedRole::Member, vec![HostedScope::Read]),
    )
    .unwrap();
    auth.register_static_key(
        "rk_test_writer_001",
        seed(
            "tenant-a",
            HostedRole::Member,
            vec![HostedScope::Read, HostedScope::Write],
        ),
    )
    .unwrap();
    auth.register_static_key(
        "rk_test_tadmin_001",
        seed("tenant-a", HostedRole::TenantAdmin, vec![HostedScope::Admin]),
    )
    .unwrap();
    auth.register_static_key(
        "rk_test_sadmin_001",
        seed("tenant-a", HostedRole::SystemAdmin, vec![HostedScope::Admin]),
    )
    .unwrap();

    let reader = auth.authenticate("rk_test_reader_001", &tenants, now).unwrap();
    let writer = auth.authenticate("rk_test_writer_001", &tenants, now).unwrap();
    let tenant_admin = auth.authenticate("rk_test_tadmin_001", &tenants, now).unwrap();
    let system_admin = auth.authenticate("rk_test_sadmin_001", &tenants, now).unwrap();

    // Everyone may ask who they are.
    assert!(authorize_method(&reader, "hosted/whoami").is_allowed());
    // Read-only methods need read; mutating methods need write.
    assert!(authorize_method(&reader, "thread/list").is_allowed());
    assert!(!authorize_method(&reader, "thread/start").is_allowed());
    assert!(authorize_method(&writer, "thread/start").is_allowed());
    // Tenant administration needs admin scope + tenant-admin role.
    assert!(!authorize_method(&writer, "hosted/service_accounts/create").is_allowed());
    assert!(authorize_method(&tenant_admin, "hosted/service_accounts/create").is_allowed());
    // Cross-tenant listing is system-admin only.
    assert!(!authorize_method(&tenant_admin, "hosted/tenants/list").is_allowed());
    assert!(authorize_method(&system_admin, "hosted/tenants/list").is_allowed());
    // Unknown methods are denied fail-closed.
    assert_eq!(
        authorize_method(&writer, "made/up_method"),
        AuthorizationDecision::deny("unknown_method")
    );
}
