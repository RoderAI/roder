mod fake_server;
#[path = "support/forks.rs"]
#[allow(dead_code)]
mod support;

use fake_server::FakeBlaxelServer;
use roder_api::remote_runner::{RemoteRunnerProvider, RunnerDestination};
use roder_ext_runner_blaxel::{BlaxelConfig, BlaxelRunnerProvider, BlaxelWorkspace, Redacted};
use serde_json::json;

fn destination() -> RunnerDestination {
    RunnerDestination {
        id: "source-run".into(),
        provider_id: "blaxel".into(),
        default_manifest: Default::default(),
        config: json!({"sandbox_name":"source-one", "external_id":"source-execution-one",
            "cleanup":"delete-on-close", "token":"untrusted-destination-key"}),
    }
}

#[test]
fn profiles_reject_missing_credentials_invalid_scope_and_untrusted_endpoints() {
    for workspace in [
        "",
        "Uppercase",
        "has space",
        "-start",
        "end-",
        &"a".repeat(64),
    ] {
        assert!(BlaxelWorkspace::new(Redacted("test-key".into()), workspace).is_err());
    }
    for credential in ["", " ", "test\r\nheader"] {
        assert!(BlaxelWorkspace::new(Redacted(credential.into()), "snapshot-workspace").is_err());
    }
    for url in [
        "https://example.test/v0",
        "https://secret@api.blaxel.ai/v0",
        "http://api.blaxel.ai/v0",
        "https://api.blaxel.ai/v0?secret=value",
        "https://api.blaxel.ai/wrong",
        "http://localhost/v0#secret",
    ] {
        let error =
            BlaxelWorkspace::with_base_url(Redacted("test-key".into()), "snapshot-workspace", url)
                .unwrap_err();
        assert!(!error.to_string().contains("secret"));
    }
}

#[tokio::test]
async fn explicit_account_controls_create_rejoin_and_delete_without_exposing_credentials() {
    let server = FakeBlaxelServer::start_with_token("snapshot-key").await;
    let profile = BlaxelWorkspace::with_base_url(
        Redacted("snapshot-key".into()),
        "snapshot-workspace",
        &format!("{}/v0", server.base_url()),
    )
    .unwrap();
    let provider = BlaxelRunnerProvider::for_workspace(profile.clone()).unwrap();
    assert!(provider.setup_hint().is_none());
    assert!(!format!("{provider:?} {profile:?}").contains("snapshot-key"));
    let config = BlaxelConfig::from_destination_in_workspace(&destination(), &profile).unwrap();
    assert_eq!(config.token.as_str(), "snapshot-key");
    let session = provider.create_session(destination()).await.unwrap();
    let state = session.state();
    assert_eq!(state.metadata["workspace"], "snapshot-workspace");
    assert_eq!(state.metadata["explicit_workspace"], true);
    assert!(
        !serde_json::to_string(&state)
            .unwrap()
            .contains("snapshot-key")
    );
    assert!(
        !serde_json::to_string(&state)
            .unwrap()
            .contains("untrusted-destination-key")
    );
    let before = server.requests().len();
    assert!(
        BlaxelRunnerProvider::default()
            .rejoin_session(state.clone())
            .await
            .is_err()
    );
    let wrong = BlaxelRunnerProvider::for_workspace(
        BlaxelWorkspace::with_base_url(
            Redacted("other-key".into()),
            "other-workspace",
            &format!("{}/v0", server.base_url()),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(wrong.rejoin_session(state.clone()).await.is_err());
    for key in ["explicit_workspace", "workspace", "base_url"] {
        let mut missing = state.clone();
        missing.metadata.as_object_mut().unwrap().remove(key);
        assert!(provider.rejoin_session(missing).await.is_err());
    }
    assert_eq!(before, server.requests().len());
    let session = provider.rejoin_session(state).await.unwrap();
    session.close().await.unwrap();
    let requests = server.requests();
    assert!(
        requests
            .iter()
            .any(|r| r.starts_with("DELETE /v0/sandboxes/source-one "))
    );
    for request in requests {
        let request = request.to_ascii_lowercase();
        assert!(request.contains("authorization: bearer snapshot-key"));
        if request.contains(" /v0/sandboxes") {
            assert!(request.contains("x-blaxel-workspace: snapshot-workspace"));
        }
        assert!(!request.contains("untrusted-destination-key"));
    }
}

#[tokio::test]
async fn destination_scope_and_egress_mismatches_are_rejected_before_allocation() {
    let server = FakeBlaxelServer::start_with_token("snapshot-key").await;
    let provider = BlaxelRunnerProvider::for_workspace(
        BlaxelWorkspace::with_base_url(
            Redacted("snapshot-key".into()),
            "snapshot-workspace",
            &format!("{}/v0", server.base_url()),
        )
        .unwrap(),
    )
    .unwrap();
    for (key, value) in [
        ("workspace", json!("default-workspace")),
        ("workspace", json!(null)),
        ("base_url", json!("http://other.test/v0")),
    ] {
        let mut destination = destination();
        destination.config[key] = value;
        assert!(provider.create_session(destination).await.is_err());
    }
    let mut destination = destination();
    destination.config["egress_gateway"] = json!("static-ip-gateway");
    destination.config["egress_workspace"] = json!("static-ip-workspace");
    destination.config["region"] = json!("us-was-1");
    assert!(provider.create_session(destination).await.is_err());
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn source_creation_does_not_follow_redirects_or_retry_mutations() {
    let target = support::Server::start(vec![]).await;
    let server = support::Server::start(vec![format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", target.url)]).await;
    let provider = BlaxelRunnerProvider::for_workspace(
        BlaxelWorkspace::with_base_url(
            Redacted("snapshot-key".into()),
            "snapshot-workspace",
            &server.url,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(provider.create_session(destination()).await.is_err());
    assert_eq!(server.requests().await.len(), 1);
    assert!(target.requests().await.is_empty());
}

#[test]
fn explicit_profile_ignores_process_defaults() {
    const CHILD: &str = "RODER_TEST_EXPLICIT_WORKSPACE_CHILD";
    if std::env::var(CHILD).is_err() {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "explicit_profile_ignores_process_defaults"])
            .env(CHILD, "1");
        for key in ["RODER_BLAXEL_API_KEY", "BLAXEL_API_KEY", "BL_API_KEY"] {
            command.env(key, "default-static-ip-key");
        }
        for key in ["RODER_BLAXEL_WORKSPACE", "BL_WORKSPACE", "BLAXEL_WORKSPACE"] {
            command.env(key, "default-static-ip-workspace");
        }
        for key in ["RODER_BLAXEL_BASE_URL", "BLAXEL_RUNNER_BASE_URL"] {
            command.env(key, "https://default.invalid/v0");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let profile = BlaxelWorkspace::new(
        Redacted("snapshot-profile-key".into()),
        "snapshot-workspace",
    )
    .unwrap();
    let config = BlaxelConfig::from_destination_in_workspace(&destination(), &profile).unwrap();
    assert_eq!(config.token.as_str(), "snapshot-profile-key");
    assert_eq!(config.workspace.as_deref(), Some("snapshot-workspace"));
    assert_eq!(config.base_url, "https://api.blaxel.ai/v0");
    let default = BlaxelConfig::from_destination(&destination()).unwrap();
    assert_eq!(default.token.as_str(), "default-static-ip-key");
    assert_eq!(
        default.workspace.as_deref(),
        Some("default-static-ip-workspace")
    );
    assert_eq!(default.base_url, "https://default.invalid/v0");
}

#[tokio::test]
async fn separate_profiles_allocate_concurrently_without_changing_each_others_credentials() {
    let first = FakeBlaxelServer::start_with_token("snapshot-one-key").await;
    let second = FakeBlaxelServer::start_with_token("snapshot-two-key").await;
    let provider = |server: &FakeBlaxelServer, workspace: &str, token: &str| {
        BlaxelRunnerProvider::for_workspace(
            BlaxelWorkspace::with_base_url(
                Redacted(token.into()),
                workspace,
                &format!("{}/v0", server.base_url()),
            )
            .unwrap(),
        )
        .unwrap()
    };
    let one = provider(&first, "snapshot-one", "snapshot-one-key");
    let two = provider(&second, "snapshot-two", "snapshot-two-key");
    let (one, two) = tokio::join!(
        one.create_session(destination()),
        two.create_session(destination())
    );
    one.unwrap().close().await.unwrap();
    two.unwrap().close().await.unwrap();
    for (server, workspace, token) in [
        (first, "snapshot-one", "snapshot-one-key"),
        (second, "snapshot-two", "snapshot-two-key"),
    ] {
        for request in server.requests() {
            let request = request.to_ascii_lowercase();
            assert!(request.contains(&format!("authorization: bearer {token}")));
            if request.contains(" /v0/sandboxes") {
                assert!(request.contains(&format!("x-blaxel-workspace: {workspace}")));
            }
        }
    }
}

#[tokio::test]
async fn credentials_can_rotate_without_changing_persisted_account_or_session_identity() {
    let server = FakeBlaxelServer::start_with_token("old-key").await;
    let provider = |token: &str| {
        BlaxelRunnerProvider::for_workspace(
            BlaxelWorkspace::with_base_url(
                Redacted(token.into()),
                "snapshot-workspace",
                &format!("{}/v0", server.base_url()),
            )
            .unwrap(),
        )
        .unwrap()
    };
    let original = provider("old-key")
        .create_session(destination())
        .await
        .unwrap();
    let state = original.state();
    let before = server.requests().len();
    server.set_token("rotated-key");
    let session = provider("rotated-key")
        .rejoin_session(state.clone())
        .await
        .unwrap();
    assert_eq!(
        session.state().metadata["sandbox_name"],
        state.metadata["sandbox_name"]
    );
    session.close().await.unwrap();
    for request in &server.requests()[before..] {
        let request = request.to_ascii_lowercase();
        assert!(request.contains("authorization: bearer rotated-key"));
        assert!(!request.contains("old-key"));
    }
}

#[tokio::test]
async fn source_endpoint_is_validated_before_sending_workspace_credentials() {
    for endpoint in [
        "https://untrusted.invalid",
        "https://secret@source.bl.run",
        "https://source.bl.run?secret=value",
    ] {
        let mut sandbox = support::source();
        sandbox["metadata"]["url"] = json!(endpoint);
        let server = support::Server::start(vec![support::response(200, sandbox)]).await;
        let provider = BlaxelRunnerProvider::for_workspace(
            BlaxelWorkspace::with_base_url(
                Redacted("snapshot-key".into()),
                "snapshot-workspace",
                &server.url,
            )
            .unwrap(),
        )
        .unwrap();
        let error = provider.create_session(destination()).await.err().unwrap();
        assert!(!error.to_string().contains("secret"));
        assert_eq!(server.requests().await.len(), 1);
    }
}
