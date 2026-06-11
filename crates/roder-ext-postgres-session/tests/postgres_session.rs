use roder_ext_postgres_session::{
    PostgresSessionConfig, PostgresSessionStore, redact_database_url,
};

#[test]
fn redacts_password_bearing_database_url() {
    assert_eq!(
        redact_database_url("postgres://roder:secret@localhost:5432/roder"),
        "postgres://roder:<redacted>@localhost:5432/roder"
    );
}

#[test]
fn config_requires_tenant_and_url() {
    let err = PostgresSessionConfig::new("", "tenant-a")
        .unwrap_err()
        .to_string();
    assert!(err.contains("database URL"));
    let err = PostgresSessionConfig::new("postgres://localhost/db", "")
        .unwrap_err()
        .to_string();
    assert!(err.contains("tenant id"));
}

#[test]
fn tenant_ids_validate_for_shared_pool_handles() {
    use roder_ext_postgres_session::validate_tenant_id;
    assert_eq!(validate_tenant_id(" tenant-a ").unwrap(), "tenant-a");
    assert!(validate_tenant_id("").is_err());
    assert!(validate_tenant_id("a/b").is_err());
}

#[tokio::test]
#[ignore = "requires RODER_POSTGRES_SESSION_TEST_URL"]
async fn postgres_store_connects_and_migrates_when_env_is_present() {
    let Ok(url) = std::env::var("RODER_POSTGRES_SESSION_TEST_URL") else {
        eprintln!("RODER_POSTGRES_SESSION_TEST_URL not set; skipping live PostgreSQL session test");
        return;
    };
    let config =
        PostgresSessionConfig::new(url, format!("tenant-{}", uuid::Uuid::new_v4())).unwrap();
    let store = PostgresSessionStore::connect(&config).await.unwrap();
    // Shared-pool tenant handles bind a different tenant over the same pool.
    let other = store.for_tenant("tenant-other").unwrap();
    assert_eq!(other.tenant_id(), "tenant-other");
    assert_ne!(store.tenant_id(), other.tenant_id());
}
