mod fake_server;

use std::sync::Mutex;

use roder_api::extension::{ProvidedService, RoderExtension};
use roder_api::remote_runner::{
    RemoteRunnerProvider, RunnerCommandRequest, RunnerDestination, RunnerFileReadRequest,
    RunnerFileWriteRequest, RunnerManifest, RunnerManifestEntry,
};
use roder_ext_runner_sprites::config::{
    BASE_URL_ENV, DEFAULT_APP_SERVER_TOKEN_ENV, RODER_BASE_URL_ENV, RODER_TOKEN_ENV, TOKEN_ENV,
};
use roder_ext_runner_sprites::{
    CleanupMode, PROVIDER_ID, SpritesConfig, SpritesRunnerExtension, SpritesRunnerProvider,
};

use fake_server::FakeSpritesServer;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn clear_env() {
    unsafe {
        std::env::remove_var(RODER_TOKEN_ENV);
        std::env::remove_var(TOKEN_ENV);
        std::env::remove_var(RODER_BASE_URL_ENV);
        std::env::remove_var(BASE_URL_ENV);
        std::env::remove_var(DEFAULT_APP_SERVER_TOKEN_ENV);
    }
}

#[test]
fn manifest_registration_exposes_sprites_runner() {
    let manifest = SpritesRunnerExtension.manifest();
    assert_eq!(manifest.id, "roder-ext-runner-sprites");
    assert_eq!(
        manifest.provides,
        vec![ProvidedService::RemoteRunnerProvider(
            PROVIDER_ID.to_string()
        )]
    );
    assert!(
        manifest
            .required_capabilities
            .iter()
            .any(|capability| capability.id == "network.http")
    );
}

#[test]
fn config_precedence_and_redaction() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    unsafe {
        std::env::set_var(TOKEN_ENV, "base-token");
        std::env::set_var(RODER_TOKEN_ENV, "roder-token");
        std::env::set_var(BASE_URL_ENV, "https://base.example/");
        std::env::set_var(RODER_BASE_URL_ENV, "https://roder.example/");
    }
    let destination = RunnerDestination {
        id: "sprites-dev".to_string(),
        provider_id: PROVIDER_ID.to_string(),
        config: serde_json::json!({
            "base_url": "https://config.example",
            "sprite_name": "existing-sprite",
            "cleanup": "keep",
            "network_policy": {"default": "deny", "token": "do-not-print"}
        }),
        default_manifest: RunnerManifest::default(),
    };
    let config = SpritesConfig::from_destination(&destination).unwrap();
    assert_eq!(config.token, "roder-token");
    assert_eq!(config.base_url, "https://roder.example");
    assert_eq!(config.sprite_name.as_deref(), Some("existing-sprite"));
    let debug = format!("{config:?}");
    assert!(!debug.contains("roder-token"));
    assert!(!debug.contains("do-not-print"));
    clear_env();
}

#[tokio::test]
async fn fake_lifecycle_command_files_checkpoint_and_cleanup_match_contract() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let server = FakeSpritesServer::start(9).await;
    let provider = SpritesRunnerProvider::default();
    let session = provider
        .create_session(RunnerDestination {
            id: "sprites-dev".to_string(),
            provider_id: PROVIDER_ID.to_string(),
            config: serde_json::json!({
                "token": "test-token",
                "base_url": server.base_url(),
                "sprite_name_prefix": "roder-test",
                "network_policy": {"default": "deny"},
                "cleanup": "delete-on-close"
            }),
            default_manifest: RunnerManifest::default(),
        })
        .await
        .unwrap();
    assert_eq!(session.state().provider_id, PROVIDER_ID);
    assert_eq!(session.state().session_id, "roder-test");

    let command = session
        .run_command(RunnerCommandRequest {
            command_id: "cmd-1".to_string(),
            program: "python3".to_string(),
            args: vec!["-c".to_string(), "print(2+2)".to_string()],
            cwd: None,
            env: vec![("RUST_LOG".to_string(), "info".to_string())],
            timeout_ms: None,
        })
        .await
        .unwrap();
    assert_eq!(command.stdout, "4\n");

    session
        .write_file(RunnerFileWriteRequest {
            path: "hello.txt".into(),
            contents: b"hello".to_vec(),
        })
        .await
        .unwrap();
    let file = session
        .read_file(RunnerFileReadRequest {
            path: "hello.txt".into(),
        })
        .await
        .unwrap();
    assert_eq!(file.contents, b"hello");
    assert_eq!(session.snapshot().await.unwrap().unwrap().snapshot_id, "v1");
    assert!(session.cancel_command(&"cmd-1".to_string()).await.unwrap());
    session.close().await.unwrap();

    let requests = server.requests().join("\n---\n");
    assert!(requests.contains("POST /v1/sprites "));
    assert!(requests.contains("PUT /v1/sprites/roder-test/policy/network "));
    assert!(requests.contains("path=.roder-runner-ready"));
    assert!(requests.contains("POST /v1/sprites/roder-test/exec?"));
    assert!(requests.contains("cmd=python3"));
    assert!(requests.contains("cmd=-c"));
    assert!(requests.contains("env=RUST_LOG%3Dinfo"));
    assert!(requests.contains("PUT /v1/sprites/roder-test/fs/write?"));
    assert!(requests.contains("GET /v1/sprites/roder-test/fs/read?"));
    assert!(requests.contains("POST /v1/sprites/roder-test/checkpoint "));
    assert!(requests.contains("DELETE /v1/sprites/roder-test "));
    clear_env();
}

#[tokio::test]
async fn manifest_directory_materialization_forks_repo_tree() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let source = temp_dir("sprites-repo-source");
    std::fs::create_dir_all(source.join("src")).unwrap();
    std::fs::create_dir_all(source.join(".git")).unwrap();
    std::fs::create_dir_all(source.join("target/debug")).unwrap();
    std::fs::create_dir_all(source.join("roadmap/assets/demo/frames")).unwrap();
    std::fs::write(source.join("Cargo.toml"), b"[package]\nname = \"demo\"\n").unwrap();
    std::fs::write(
        source.join("src/lib.rs"),
        b"pub fn answer() -> i32 { 42 }\n",
    )
    .unwrap();
    std::fs::write(source.join(".git/config"), b"skip me").unwrap();
    std::fs::write(source.join("target/debug/build.log"), b"skip me").unwrap();
    std::fs::write(
        source.join("roadmap/assets/demo/frames/trace.png"),
        b"generated image",
    )
    .unwrap();

    let server = FakeSpritesServer::start(5).await;
    let provider = SpritesRunnerProvider::default();
    let session = provider
        .create_session(RunnerDestination {
            id: "sprites-dev".to_string(),
            provider_id: PROVIDER_ID.to_string(),
            config: serde_json::json!({
                "token": "test-token",
                "base_url": server.base_url(),
                "sprite_name_prefix": "roder-test",
                "cleanup": "delete-on-close"
            }),
            default_manifest: RunnerManifest {
                entries: vec![RunnerManifestEntry {
                    source: source.clone(),
                    target: "repo".into(),
                    writable: true,
                }],
                mounts: Vec::new(),
            },
        })
        .await
        .unwrap();
    session.close().await.unwrap();

    let requests = server.requests().join("\n---\n");
    assert!(requests.contains("path=.roder%2Fmanifests%2F"));
    assert!(requests.contains("tar+-xf") || requests.contains("tar%20-xf"));
    assert!(requests.contains("Cargo.toml"));
    assert!(requests.contains("src/lib.rs"));
    assert!(!requests.contains(".git/config"));
    assert!(!requests.contains("target/debug/build.log"));
    assert!(!requests.contains("roadmap/assets/demo/frames/trace.png"));

    let _ = std::fs::remove_dir_all(source);
    clear_env();
}

#[test]
fn path_scope_blocks_absolute_and_parent_paths() {
    assert!(roder_ext_runner_sprites::normalize_workspace_path("/tmp/x".as_ref()).is_err());
    assert!(roder_ext_runner_sprites::normalize_workspace_path("../x".as_ref()).is_err());
    assert_eq!(
        roder_ext_runner_sprites::normalize_workspace_path("src/lib.rs".as_ref()).unwrap(),
        "src/lib.rs"
    );
}

#[test]
fn resumed_state_preserves_cleanup_mode() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    unsafe {
        std::env::set_var(RODER_TOKEN_ENV, "resume-token");
    }
    let config = SpritesConfig::from_state(&roder_api::remote_runner::RunnerSessionState {
        provider_id: PROVIDER_ID.to_string(),
        session_id: "roder-core-live-test".to_string(),
        destination_id: "sprites-live-runtime".to_string(),
        snapshot: None,
        metadata: serde_json::json!({
            "base_url": "https://api.sprites.dev",
            "sprite_name": "roder-core-live-test",
            "working_dir": "/home/sprite/roder-core-live",
            "cleanup": "delete-on-close"
        }),
    })
    .unwrap();

    assert_eq!(config.cleanup, CleanupMode::DeleteOnClose);
    clear_env();
}

#[tokio::test]
async fn app_server_bootstrap_installs_binary_and_starts_service() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    unsafe {
        std::env::set_var(DEFAULT_APP_SERVER_TOKEN_ENV, "remote-secret");
    }
    let server = FakeSpritesServer::start(10).await;
    let binary_path = temp_file("remote-roder-bin");
    std::fs::write(&binary_path, b"#!/bin/sh\n").unwrap();
    let provider = SpritesRunnerProvider::default();
    let session = provider
        .create_session(RunnerDestination {
            id: "sprites-dev".to_string(),
            provider_id: PROVIDER_ID.to_string(),
            config: serde_json::json!({
                "token": "test-token",
                "base_url": server.base_url(),
                "sprite_name_prefix": "roder-test",
                "cleanup": "delete-on-close",
                "app_server": {
                    "enabled": true,
                    "local_binary_path": binary_path.display().to_string(),
                    "auth_token_env": DEFAULT_APP_SERVER_TOKEN_ENV,
                    "workspace_path": "repo",
                    "env_passthrough": []
                }
            }),
            default_manifest: RunnerManifest::default(),
        })
        .await
        .unwrap();

    let state = session.state();
    let app_server = &state.metadata["remote_app_server"];
    assert_eq!(app_server["service_name"], "roder-app-server");
    assert_eq!(app_server["port"], 17373);
    assert_eq!(app_server["status"], "running");
    assert_eq!(app_server["token_env"], DEFAULT_APP_SERVER_TOKEN_ENV);
    assert_eq!(app_server["workspace_path"], "repo");
    assert_eq!(
        app_server["health_url"],
        "https://roder-test.sprites.app/readyz"
    );
    assert_eq!(app_server["connect_url"], "wss://roder-test.sprites.app");
    assert_eq!(app_server["websocket_url"], "wss://roder-test.sprites.app");
    assert_eq!(
        app_server["auth_schemes"],
        serde_json::json!(["authorization_bearer", "websocket_subprotocol_bearer"])
    );
    assert_eq!(
        app_server["subprotocols"],
        serde_json::json!([
            "roder.remote.v1",
            format!("bearer.env:{DEFAULT_APP_SERVER_TOKEN_ENV}")
        ])
    );
    assert!(!state.metadata.to_string().contains("remote-secret"));
    session.close().await.unwrap();

    let requests = server.requests().join("\n---\n");
    assert!(requests.contains("PUT /v1/sprites/roder-test/fs/write?"));
    assert!(requests.contains("path=.roder%2Fbin%2Froder"));
    assert!(requests.contains("path=repo%2F.roder-workspace-ready"));
    assert!(requests.contains("cmd=sh"));
    assert!(requests.contains("PUT /v1/sprites/roder-test/services/roder-app-server "));
    assert!(requests.contains(r#""cmd":"/home/sprite/roder/.roder/bin/roder""#));
    assert!(requests.contains(r#""dir":"/home/sprite/roder/repo""#));
    assert!(
        requests.contains("ws%3A%2F%2F0.0.0.0%3A17373") || requests.contains("ws://0.0.0.0:17373")
    );
    assert!(requests.contains("GET /v1/sprites/roder-test/services/roder-app-server "));

    let _ = std::fs::remove_file(binary_path);
    clear_env();
}

fn temp_file(prefix: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let path = temp_file(prefix);
    std::fs::create_dir_all(&path).unwrap();
    path
}
