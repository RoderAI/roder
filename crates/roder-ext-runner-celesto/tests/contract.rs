mod fake_server;

use std::sync::Mutex;

use roder_api::extension::{ProvidedService, RoderExtension};
use roder_api::remote_runner::{
    RemoteRunnerProvider, RunnerArtifactExportRequest, RunnerCommandRequest, RunnerDestination,
    RunnerFileReadRequest, RunnerFileWriteRequest, RunnerManifest, RunnerPortRequest,
    RunnerSessionState,
};
use roder_ext_runner_celesto::config::{
    BASE_URL_ENV, RODER_BASE_URL_ENV, RODER_TOKEN_ENV, TOKEN_ENV,
};
use roder_ext_runner_celesto::{
    CelestoConfig, CelestoRunnerExtension, CelestoRunnerProvider, CleanupMode, PROVIDER_ID,
};

use fake_server::FakeCelestoServer;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn clear_env() {
    unsafe {
        std::env::remove_var(RODER_TOKEN_ENV);
        std::env::remove_var(TOKEN_ENV);
        std::env::remove_var(RODER_BASE_URL_ENV);
        std::env::remove_var(BASE_URL_ENV);
    }
}

#[test]
fn manifest_registration_exposes_celesto_runner() {
    let manifest = CelestoRunnerExtension.manifest();
    assert_eq!(manifest.id, "roder-ext-runner-celesto");
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
        id: "celesto-dev".to_string(),
        provider_id: PROVIDER_ID.to_string(),
        config: serde_json::json!({
            "base_url": "https://config.example",
            "token": "config-token",
            "computer_id": "comp-existing",
            "cleanup": "keep",
            "template_id": "coding-agent"
        }),
        default_manifest: RunnerManifest::default(),
    };
    let config = CelestoConfig::from_destination(&destination).unwrap();
    assert_eq!(config.token, "roder-token");
    assert_eq!(config.base_url, "https://roder.example");
    assert_eq!(config.computer_id.as_deref(), Some("comp-existing"));
    assert_eq!(config.cleanup, CleanupMode::Keep);
    let debug = format!("{config:?}");
    assert!(!debug.contains("roder-token"));
    assert!(!debug.contains("config-token"));
    clear_env();
}

#[test]
fn missing_token_is_an_error() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let destination = RunnerDestination {
        id: "celesto-dev".to_string(),
        provider_id: PROVIDER_ID.to_string(),
        config: serde_json::json!({}),
        default_manifest: RunnerManifest::default(),
    };
    assert!(CelestoConfig::from_destination(&destination).is_err());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)] // env guard must stay held while the provider reads env
async fn fake_lifecycle_command_files_and_ports_match_contract() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let server = FakeCelestoServer::start(7).await;
    let provider = CelestoRunnerProvider::default();
    let session = provider
        .create_session(RunnerDestination {
            id: "celesto-dev".to_string(),
            provider_id: PROVIDER_ID.to_string(),
            config: serde_json::json!({
                "token": "test-token",
                "base_url": server.base_url(),
                "working_dir": "/root/roder",
                "cleanup": "delete-on-close"
            }),
            default_manifest: RunnerManifest::default(),
        })
        .await
        .unwrap();

    assert_eq!(session.state().provider_id, PROVIDER_ID);
    assert_eq!(session.state().session_id, "comp-123");

    let command = session
        .run_command(RunnerCommandRequest {
            command_id: "cmd-1".to_string(),
            program: "python3".to_string(),
            args: vec!["-c".to_string(), "print(2+2)".to_string()],
            cwd: None,
            env: vec![("RUST_LOG".to_string(), "info".to_string())],
        })
        .await
        .unwrap();
    assert_eq!(command.exit_code, Some(0));
    assert_eq!(command.stdout.trim(), "4");

    let read = session
        .read_file(RunnerFileReadRequest {
            path: "/root/roder/hello.txt".into(),
        })
        .await
        .unwrap();
    assert_eq!(read.contents, b"hello");

    session
        .write_file(RunnerFileWriteRequest {
            path: "/root/roder/out.txt".into(),
            contents: b"data".to_vec(),
        })
        .await
        .unwrap();

    let port = session
        .expose_port(RunnerPortRequest {
            port: 8000,
            label: Some("preview".to_string()),
        })
        .await
        .unwrap();
    assert_eq!(port.port, 8000);
    assert_eq!(port.url.as_deref(), Some("https://comp-123-8000.celesto.app"));

    // Artifact export is intentionally unsupported by this provider.
    assert!(
        session
            .export_artifact(RunnerArtifactExportRequest {
                path: "/root/roder/out.txt".into(),
                recursive: false,
            })
            .await
            .is_err()
    );
    assert!(session.snapshot().await.unwrap().is_none());

    session.close().await.unwrap();

    let requests = server.requests();
    assert!(requests.iter().any(|r| r.starts_with("POST /v1/computers ")));
    assert!(
        requests
            .iter()
            .any(|r| r.starts_with("DELETE /v1/computers/comp-123 "))
    );
    // Auth header is always present.
    assert!(
        requests
            .iter()
            .all(|r| r.to_ascii_lowercase().contains("authorization: bearer test-token"))
    );
    clear_env();
}

#[tokio::test]
#[allow(clippy::await_holding_lock)] // env guard must stay held while the provider reads env
async fn resume_reattaches_to_existing_computer() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let server = FakeCelestoServer::start(1).await;
    unsafe {
        std::env::set_var(RODER_TOKEN_ENV, "test-token");
    }
    let provider = CelestoRunnerProvider::default();
    let state = RunnerSessionState {
        provider_id: PROVIDER_ID.to_string(),
        session_id: "comp-123".to_string(),
        destination_id: "celesto-dev".to_string(),
        snapshot: None,
        metadata: serde_json::json!({
            "base_url": server.base_url(),
            "computer_id": "comp-123",
            "working_dir": "/root/roder",
            "cleanup": "keep"
        }),
    };
    let session = provider.resume_session(state).await.unwrap();
    assert_eq!(session.state().session_id, "comp-123");
    clear_env();
}
