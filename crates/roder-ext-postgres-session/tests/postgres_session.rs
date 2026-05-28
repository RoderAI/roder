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

#[tokio::test]
#[ignore = "requires RODER_POSTGRES_SESSION_TEST_URL"]
async fn postgres_store_connects_and_migrates_when_env_is_present() {
    let Ok(url) = std::env::var("RODER_POSTGRES_SESSION_TEST_URL") else {
        eprintln!("RODER_POSTGRES_SESSION_TEST_URL not set; skipping live PostgreSQL session test");
        return;
    };
    let config =
        PostgresSessionConfig::new(url, format!("tenant-{}", uuid::Uuid::new_v4())).unwrap();
    let _store = PostgresSessionStore::connect(&config).await.unwrap();
}
