mod fake_server;

use std::sync::Mutex;

use roder_api::extension::{ProvidedService, RoderExtension};
use roder_api::remote_runner::{
    RemoteRunnerProvider, RunnerCommandRequest, RunnerDestination, RunnerFileReadRequest,
    RunnerFileWriteRequest, RunnerManifest, RunnerPortRequest, RunnerSessionState,
};
use roder_ext_runner_blaxel::config::{
    BASE_URL_ENV, BL_TOKEN_ENV, RODER_BASE_URL_ENV, RODER_TOKEN_ENV, RODER_WORKSPACE_ENV, TOKEN_ENV,
    WORKSPACE_ENV,
};
use roder_ext_runner_blaxel::{
    BlaxelConfig, BlaxelRunnerExtension, BlaxelRunnerProvider, PROVIDER_ID, sanitize_name,
};

use fake_server::FakeBlaxelServer;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn clear_env() {
    unsafe {
        for var in [
            RODER_TOKEN_ENV,
            TOKEN_ENV,
            BL_TOKEN_ENV,
            RODER_WORKSPACE_ENV,
            WORKSPACE_ENV,
            "BLAXEL_WORKSPACE",
            RODER_BASE_URL_ENV,
            BASE_URL_ENV,
        ] {
            std::env::remove_var(var);
        }
    }
}

#[test]
fn manifest_registration_exposes_blaxel_runner() {
    let manifest = BlaxelRunnerExtension.manifest();
    assert_eq!(manifest.id, "roder-ext-runner-blaxel");
    assert_eq!(
        manifest.provides,
        vec![ProvidedService::RemoteRunnerProvider(PROVIDER_ID.to_string())]
    );
    assert!(
        manifest
            .required_capabilities
            .iter()
            .any(|capability| capability.id == "network.http")
    );
}

#[test]
fn provider_advertises_pause_and_detach_capabilities() {
    let capabilities = BlaxelRunnerProvider::default().capabilities();
    assert!(capabilities.pausable);
    assert!(capabilities.detachable);
    assert!(capabilities.command_exec);
    assert!(capabilities.port_preview);
}

#[test]
fn config_precedence_and_redaction() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    unsafe {
        std::env::set_var(TOKEN_ENV, "base-token");
        std::env::set_var(RODER_TOKEN_ENV, "roder-token");
        std::env::set_var(WORKSPACE_ENV, "env-workspace");
        std::env::set_var(BASE_URL_ENV, "https://base.example/");
        std::env::set_var(RODER_BASE_URL_ENV, "https://roder.example/");
    }
    let destination = RunnerDestination {
        id: "blaxel-dev".to_string(),
        provider_id: PROVIDER_ID.to_string(),
        config: serde_json::json!({
            "base_url": "https://config.example",
            "sandbox_name": "existing-sandbox",
            "cleanup": "keep"
        }),
        default_manifest: RunnerManifest::default(),
    };
    let config = BlaxelConfig::from_destination(&destination).unwrap();
    assert_eq!(config.token.as_str(), "roder-token");
    assert_eq!(config.base_url, "https://roder.example");
    assert_eq!(config.workspace.as_deref(), Some("env-workspace"));
    assert_eq!(config.sandbox_name.as_deref(), Some("existing-sandbox"));
    let debug = format!("{config:?}");
    assert!(!debug.contains("roder-token"), "token leaked: {debug}");
    assert!(debug.contains("<redacted>"));
    clear_env();
}

#[test]
fn sanitize_name_enforces_blaxel_rules() {
    assert_eq!(sanitize_name("roder-thread-ABC_123"), "roder-thread-abc-123");
    assert_eq!(sanitize_name("//weird**id//"), "weird-id");
    assert!(sanitize_name(&"x".repeat(80)).len() <= 49);
}

#[tokio::test]
async fn full_lifecycle_pause_resume_detach_rejoin_and_cleanup() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    // The credential is sourced from the environment (as a destination
    // `secret_env` mapping would inject it), never persisted in state.
    unsafe {
        std::env::set_var(TOKEN_ENV, "test-token");
    }
    let server = FakeBlaxelServer::start().await;
    let provider = BlaxelRunnerProvider::default();
    let destination = RunnerDestination {
        id: "blaxel-dev".to_string(),
        provider_id: PROVIDER_ID.to_string(),
        config: serde_json::json!({
            "workspace": "test-workspace",
            "base_url": server.base_url(),
            "sandbox_name_prefix": "roder",
            "working_dir": "/home/user/roder",
            "cleanup": "delete-on-close"
        }),
        default_manifest: RunnerManifest::default(),
    };

    let session = provider.create_session(destination).await.unwrap();
    let state = session.state();
    assert_eq!(state.provider_id, PROVIDER_ID);
    assert_eq!(state.session_id, "roder-blaxel-dev");
    assert_eq!(
        state.metadata["working_dir"].as_str(),
        Some("/home/user/roder")
    );

    // Command construction joins program + args into a shell command.
    let command = session
        .run_command(RunnerCommandRequest {
            command_id: "cmd-1".to_string(),
            program: "echo".to_string(),
            args: vec!["hello".to_string(), "world".to_string()],
            cwd: None,
            env: vec![("RUST_LOG".to_string(), "info".to_string())],
        })
        .await
        .unwrap();
    assert_eq!(command.stdout, "hello world\n");
    assert_eq!(command.exit_code, Some(0));

    // Write then read a file round-trips through the per-sandbox filesystem API.
    session
        .write_file(RunnerFileWriteRequest {
            path: "notes.txt".into(),
            contents: b"hello blaxel".to_vec(),
        })
        .await
        .unwrap();
    let file = session
        .read_file(RunnerFileReadRequest {
            path: "notes.txt".into(),
        })
        .await
        .unwrap();
    assert_eq!(file.contents, b"hello blaxel");

    // Port preview returns a preview URL.
    let port = session
        .expose_port(RunnerPortRequest {
            port: 3000,
            label: None,
        })
        .await
        .unwrap();
    assert_eq!(port.port, 3000);
    assert!(port.url.unwrap().contains("preview.example"));

    // Pause marks standby intent; resume wakes the sandbox.
    let paused = session.pause().await.unwrap();
    assert_eq!(paused.metadata["paused"].as_bool(), Some(true));
    let resumed = session.resume().await.unwrap();
    assert_eq!(resumed.metadata["paused"].as_bool(), Some(false));

    // Detach returns durable, rejoinable state with no secrets.
    let detached = session.detach().await.unwrap();
    let detached_debug = serde_json::to_string(&detached).unwrap();
    assert!(!detached_debug.contains("test-token"));
    assert_eq!(detached.metadata["sandbox_name"].as_str(), Some("roder-blaxel-dev"));

    // Rejoin reuses the same sandbox without creating a new one.
    let rejoined = provider.rejoin_session(detached).await.unwrap();
    assert_eq!(rejoined.state().session_id, "roder-blaxel-dev");

    // Cleanup deletes the sandbox (delete-on-close).
    rejoined.close().await.unwrap();

    // Auth header present, token never serialized into a request body.
    let requests = server.requests();
    assert!(
        requests
            .iter()
            .any(|req| req.to_ascii_lowercase().contains("authorization: bearer test-token"))
    );
    assert!(requests.iter().all(|req| {
        let body = req.split("\r\n\r\n").nth(1).unwrap_or_default();
        !body.contains("test-token")
    }));
    clear_env();
}

#[tokio::test]
async fn rejoin_recovers_via_external_id_when_name_is_lost() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    unsafe {
        std::env::set_var(TOKEN_ENV, "test-token");
    }
    let server = FakeBlaxelServer::start().await;
    let provider = BlaxelRunnerProvider::default();
    // Seed a sandbox so the external-id mapping exists on the server.
    let created = provider
        .create_session(RunnerDestination {
            id: "blaxel-dev".to_string(),
            provider_id: PROVIDER_ID.to_string(),
            config: serde_json::json!({
                "base_url": server.base_url(),
                "sandbox_name_prefix": "roder",
                "working_dir": "/home/user/roder"
            }),
            default_manifest: RunnerManifest::default(),
        })
        .await
        .unwrap();
    drop(created);

    // Rejoin from state whose sandbox_name no longer resolves, but whose
    // external_id still maps to the live sandbox.
    let state = RunnerSessionState {
        provider_id: PROVIDER_ID.to_string(),
        session_id: "missing-name".to_string(),
        destination_id: "blaxel-dev".to_string(),
        snapshot: None,
        metadata: serde_json::json!({
            "base_url": server.base_url(),
            "sandbox_name": "missing-name",
            "external_id": "blaxel-dev",
            "working_dir": "/home/user/roder",
            "cleanup": "detach-on-close"
        }),
    };
    let rejoined = provider.rejoin_session(state).await.unwrap();
    assert_eq!(rejoined.state().session_id, "roder-blaxel-dev");
    clear_env();
}
