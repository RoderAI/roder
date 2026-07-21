use std::sync::Arc;

use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::lifecycle::{
    TurnCleanupState, TurnLifecycleReason, TurnLifecycleRecord, TurnLifecycleState,
};
use roder_api::thread::{ThreadStore, ThreadStoreFactory};
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig};
use roder_ext_postgres_session::{
    PostgresSessionConfig, PostgresSessionStore, redact_database_url,
};

struct SharedPostgresStoreFactory {
    store: Arc<PostgresSessionStore>,
}

impl ThreadStoreFactory for SharedPostgresStoreFactory {
    fn id(&self) -> roder_api::extension::ThreadStoreId {
        self.store.id()
    }

    fn create(&self) -> Arc<dyn ThreadStore> {
        self.store.clone()
    }
}

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

#[tokio::test]
#[ignore = "requires RODER_POSTGRES_SESSION_TEST_URL"]
async fn postgres_store_persists_lifecycle_and_recovers_after_runtime_restart() {
    let Ok(url) = std::env::var("RODER_POSTGRES_SESSION_TEST_URL") else {
        eprintln!(
            "RODER_POSTGRES_SESSION_TEST_URL not set; skipping live PostgreSQL lifecycle test"
        );
        return;
    };
    let config =
        PostgresSessionConfig::new(url, format!("tenant-lifecycle-{}", uuid::Uuid::new_v4()))
            .unwrap();
    let store = Arc::new(PostgresSessionStore::connect(&config).await.unwrap());
    let factory = Arc::new(SharedPostgresStoreFactory {
        store: store.clone(),
    });

    let mut initial_builder = ExtensionRegistryBuilder::new();
    initial_builder.inference_engine(Arc::new(FakeInferenceEngine));
    initial_builder.thread_store_factory(factory.clone());
    let initial =
        Arc::new(Runtime::new(initial_builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let thread_id = initial
        .create_thread(Some("Interrupted before PostgreSQL restart".to_string()))
        .await
        .unwrap()
        .thread_id;
    let record = TurnLifecycleRecord::new(
        thread_id.clone(),
        "unfinished-postgres-turn".to_string(),
        TurnLifecycleState::InterruptRequested,
        TurnCleanupState::Requested,
        Some(TurnLifecycleReason::Shutdown),
        time::OffsetDateTime::UNIX_EPOCH,
    );
    store
        .append_extension_state(&thread_id, &record.extension_state().unwrap())
        .await
        .unwrap();

    let persisted = store.load_thread(&thread_id).await.unwrap().unwrap();
    assert!(persisted.extension_states.iter().any(|state| {
        TurnLifecycleRecord::from_extension_state(state)
            .unwrap()
            .is_some_and(|decoded| decoded == record)
    }));
    drop(initial);

    let mut recovery_builder = ExtensionRegistryBuilder::new();
    recovery_builder.inference_engine(Arc::new(FakeInferenceEngine));
    recovery_builder.thread_store_factory(factory);
    let recovery =
        Runtime::new(recovery_builder.build().unwrap(), RuntimeConfig::default()).unwrap();
    recovery.load_thread(&thread_id).await.unwrap().unwrap();

    let lifecycle = recovery.turn_lifecycle_snapshot(&thread_id).await.unwrap();
    assert!(lifecycle.records.iter().any(|record| {
        record.turn_id == "unfinished-postgres-turn"
            && record.state == TurnLifecycleState::RecoveryNeeded
            && record.reason == Some(TurnLifecycleReason::RuntimeRestart)
    }));
}
