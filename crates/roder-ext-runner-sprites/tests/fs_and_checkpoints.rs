//! Offline contract tests for the richer Sprites filesystem operations and
//! checkpoint list/restore (roadmap phase 88, Tasks 5–6). All requests hit
//! the fake HTTP server; no live `api.sprites.dev` access.

mod fake_server;

use std::path::Path;

use roder_api::remote_runner::{RemoteRunnerProvider, RunnerDestination, RunnerManifest};
use roder_ext_runner_sprites::{
    PROVIDER_ID, SpritesClient, SpritesConfig, SpritesRunnerProvider,
};

use fake_server::FakeSpritesServer;

/// Ambient Sprites env vars (from live smokes) take precedence over config;
/// clear them so the fake-server token/base-url are used.
fn clear_sprites_env() {
    use roder_ext_runner_sprites::config::{
        BASE_URL_ENV, RODER_BASE_URL_ENV, RODER_TOKEN_ENV, TOKEN_ENV,
    };
    unsafe {
        std::env::remove_var(RODER_TOKEN_ENV);
        std::env::remove_var(TOKEN_ENV);
        std::env::remove_var(RODER_BASE_URL_ENV);
        std::env::remove_var(BASE_URL_ENV);
    }
}

fn config_for(base_url: &str) -> SpritesConfig {
    SpritesConfig::from_destination(&RunnerDestination {
        id: "sprites-dev".to_string(),
        provider_id: PROVIDER_ID.to_string(),
        config: serde_json::json!({
            "token": "test-token",
            "base_url": base_url,
            "sprite_name": "roder-test",
            "cleanup": "keep"
        }),
        default_manifest: RunnerManifest::default(),
    })
    .unwrap()
}

#[tokio::test]
async fn filesystem_operations_map_to_fs_endpoints_with_scoped_paths() {
    clear_sprites_env();
    let server = FakeSpritesServer::start(6).await;
    let client = SpritesClient::new(reqwest::Client::new(), config_for(&server.base_url()));

    let entries = client
        .list_dir("roder-test", Path::new("src"))
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "hello.txt");
    assert!(!entries[0].dir);
    assert_eq!(entries[0].mode.as_deref(), Some("0644"));
    assert!(entries[1].dir);

    client
        .delete_path("roder-test", Path::new("old.txt"))
        .await
        .unwrap();
    client
        .rename_path("roder-test", Path::new("a.txt"), Path::new("b.txt"))
        .await
        .unwrap();
    client
        .copy_path("roder-test", Path::new("b.txt"), Path::new("c.txt"))
        .await
        .unwrap();
    client
        .chmod_path("roder-test", Path::new("run.sh"), "755")
        .await
        .unwrap();

    // Path traversal and bad modes are rejected before any request is sent.
    assert!(
        client
            .delete_path("roder-test", Path::new("../escape"))
            .await
            .is_err()
    );
    assert!(
        client
            .chmod_path("roder-test", Path::new("run.sh"), "rwxr-xr-x")
            .await
            .is_err()
    );

    // chmod is the 6th and final wire request; traversal/mode rejections
    // never reached the server.
    let requests = server.requests();
    let firsts: Vec<String> = requests
        .iter()
        .map(|request| request.lines().next().unwrap_or_default().to_string())
        .collect();
    assert!(firsts[0].starts_with("GET /v1/sprites/roder-test/fs/list?path=src&"));
    assert!(firsts[1].starts_with("DELETE /v1/sprites/roder-test/fs/delete?path=old.txt&"));
    assert!(firsts[2].starts_with("POST /v1/sprites/roder-test/fs/rename?path=a.txt&to=b.txt&"));
    assert!(firsts[3].starts_with("POST /v1/sprites/roder-test/fs/copy?path=b.txt&to=c.txt&"));
    assert!(firsts[4].starts_with("POST /v1/sprites/roder-test/fs/chmod?path=run.sh&mode=755&"));
    // The exec route is never used by filesystem operations.
    assert_eq!(
        firsts
            .iter()
            .filter(|first| first.contains("/fs/"))
            .count(),
        5
    );
}

#[tokio::test]
async fn checkpoint_list_and_restore_follow_ndjson_contract() {
    clear_sprites_env();
    let server = FakeSpritesServer::start(3).await;
    let client = SpritesClient::new(reqwest::Client::new(), config_for(&server.base_url()));

    let checkpoints = client.list_checkpoints("roder-test").await.unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0].id, "v1");

    client.restore_checkpoint("roder-test", "v1").await.unwrap();

    // NDJSON error frames fail closed with the server-provided reason.
    let error = client
        .restore_checkpoint("roder-test", "missing")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("checkpoint missing not found"), "{error}");

    // Empty checkpoint ids never reach the wire.
    assert!(client.restore_checkpoint("roder-test", " ").await.is_err());

    let firsts: Vec<String> = server
        .requests()
        .iter()
        .map(|request| request.lines().next().unwrap_or_default().to_string())
        .collect();
    assert!(firsts[0].starts_with("GET /v1/sprites/roder-test/checkpoints "));
    assert!(firsts[1].starts_with("POST /v1/sprites/roder-test/checkpoints/v1/restore "));
    assert!(firsts[2].starts_with("POST /v1/sprites/roder-test/checkpoints/missing/restore "));
}

#[tokio::test]
async fn create_session_restores_configured_checkpoint_before_workspace_setup() {
    clear_sprites_env();
    let server = FakeSpritesServer::start(4).await;
    let provider = SpritesRunnerProvider::default();

    let session = provider
        .create_session(RunnerDestination {
            id: "sprites-dev".to_string(),
            provider_id: PROVIDER_ID.to_string(),
            config: serde_json::json!({
                "token": "test-token",
                "base_url": server.base_url(),
                "sprite_name_prefix": "roder-test",
                "restore_checkpoint_id": "v1",
                "cleanup": "delete-on-close"
            }),
            default_manifest: RunnerManifest::default(),
        })
        .await
        .unwrap();
    assert_eq!(session.state().provider_id, PROVIDER_ID);

    let firsts: Vec<String> = server
        .requests()
        .iter()
        .map(|request| request.lines().next().unwrap_or_default().to_string())
        .collect();
    let restore_index = firsts
        .iter()
        .position(|first| first.starts_with("POST /v1/sprites/roder-test/checkpoints/v1/restore"))
        .expect("restore call issued");
    let create_index = firsts
        .iter()
        .position(|first| first.starts_with("POST /v1/sprites "))
        .expect("sprite created");
    let workspace_index = firsts
        .iter()
        .position(|first| first.starts_with("PUT /v1/sprites/roder-test/fs/write"))
        .expect("working dir prepared");
    assert!(
        create_index < restore_index && restore_index < workspace_index,
        "restore must run after creation and before workspace setup: {firsts:?}"
    );
}
