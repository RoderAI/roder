use super::*;
use crate::{BlaxelRunnerProvider, BlaxelWorkspace, Redacted};
use roder_api::remote_runner::{RemoteRunnerProvider, RunnerDestination};
use serde_json::json;
#[path = "../../tests/fake_server.rs"]
mod fake_server;
#[path = "../../tests/support/forks.rs"]
#[allow(dead_code)]
mod support;

fn destination() -> RunnerDestination {
    RunnerDestination {
        id: "durable-source".into(),
        provider_id: "blaxel".into(),
        default_manifest: Default::default(),
        config: json!({"sandbox_name":"source-one","external_id":"source-execution-one","cleanup":"detach-on-close"}),
    }
}
fn request() -> RunnerCommandRequest {
    RunnerCommandRequest {
        command_id: "source-resume-123".into(),
        program: "echo".into(),
        args: vec!["private-canary".into()],
        cwd: None,
        env: vec![],
        timeout_ms: Some(1000),
    }
}

#[tokio::test]
async fn durable_command_identity_survives_a_fresh_provider_session_without_replay() {
    let server = fake_server::FakeBlaxelServer::start_with_token("snapshot-key").await;
    let profile = || {
        BlaxelWorkspace::with_base_url(
            Redacted("snapshot-key".into()),
            "snapshot-workspace",
            &format!("{}/v0", server.base_url()),
        )
        .unwrap()
    };
    let provider = BlaxelRunnerProvider::for_workspace(profile()).unwrap();
    let session = provider.create_session(destination()).await.unwrap();
    let state = session.state();
    session.run_durable_command(request()).await.unwrap();
    let before = server.requests().len();
    assert!(session.run_durable_command(request()).await.is_err());
    assert_eq!(server.requests().len(), before);
    let fresh = BlaxelRunnerProvider::for_workspace(profile())
        .unwrap()
        .rejoin_session(state)
        .await
        .unwrap();
    let before = server.requests().len();
    let observed = fresh
        .observe_durable_command(&request().command_id)
        .await
        .unwrap();
    assert_eq!(
        observed,
        RunnerCommandObservation::Exited { exit_code: Some(0) }
    );
    assert!(!format!("{observed:?}").contains("private-canary"));
    let reads = &server.requests()[before..];
    assert_eq!(reads.len(), 1);
    assert!(reads[0].starts_with(&format!(
        "GET /process/{} ",
        durable_process_name(&request().command_id).unwrap()
    )));
    let commands: Vec<serde_json::Value> = server
        .requests()
        .iter()
        .filter(|r| r.starts_with("POST /process "))
        .map(|r| serde_json::from_str(r.split("\r\n\r\n").nth(1).unwrap()).unwrap())
        .collect();
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0]["name"],
        durable_process_name(&request().command_id).unwrap()
    );
}

#[tokio::test]
async fn durable_status_is_bounded_scoped_and_projects_no_logs() {
    let name = durable_process_name("source-resume-123").unwrap();
    for (status, body, expected) in [
        (
            200,
            json!({"name":name,"status":"running","exitCode":null,"stdout":"private-canary"}),
            Some(RunnerCommandObservation::Running),
        ),
        (
            200,
            json!({"name":name,"status":"running","exitCode":0}),
            Some(RunnerCommandObservation::Unknown),
        ),
        (
            200,
            json!({"name":name,"status":"pending","exitCode":null}),
            Some(RunnerCommandObservation::Unknown),
        ),
        (
            200,
            json!({"name":name,"status":"failed","exitCode":1}),
            Some(RunnerCommandObservation::Exited { exit_code: Some(1) }),
        ),
        (
            200,
            json!({"name":name,"status":"killed","exitCode":null}),
            Some(RunnerCommandObservation::Exited { exit_code: None }),
        ),
        (200, json!({"name":"foreign","status":"running"}), None),
        (200, json!({"name":name}), None),
        (
            200,
            json!({"name":name,"status":"running","stdout":"x".repeat(65537)}),
            None,
        ),
        (
            404,
            json!({"private":"canary"}),
            Some(RunnerCommandObservation::NotObserved),
        ),
        (302, json!({"private":"canary"}), None),
        (403, json!({"private":"canary"}), None),
        (503, json!({"private":"canary"}), None),
    ] {
        let server = support::Server::start(vec![support::response(status, body)]).await;
        let profile = BlaxelWorkspace::with_base_url(
            Redacted("fixture-credential".into()),
            "snapshot-workspace",
            &server.url,
        )
        .unwrap();
        let config = BlaxelConfig::from_destination_in_workspace(&destination(), &profile).unwrap();
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .unwrap();
        let client = BlaxelClient::new(http, &config);
        let endpoint = server.url.trim_end_matches("/v0").to_owned();
        let session = BlaxelRunnerSession::new(
            client,
            &config,
            "durable-source".into(),
            "source-one".into(),
            Some("source-execution-one".into()),
            endpoint,
            false,
        )
        .with_explicit_workspace(true);
        let result = session
            .observe_durable_command(&"source-resume-123".into())
            .await;
        assert!(!format!("{result:?}").contains("canary"));
        match expected {
            Some(value) => assert_eq!(result.unwrap(), value),
            None => assert!(result.is_err()),
        }
        let requests = server.requests().await;
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with(&format!("GET /process/{name} ")));
        assert!(requests[0].contains("authorization: Bearer fixture-credential"));
    }
}

#[test]
fn durable_name_is_stable_bounded_and_rejects_untrusted_identifiers() {
    let name = durable_process_name("source-resume-123").unwrap();
    assert_eq!(name, durable_process_name("source-resume-123").unwrap());
    assert_ne!(name, durable_process_name("source-resume-124").unwrap());
    assert!(name.len() <= 63 && !name.contains("source-resume"));
    for invalid in ["", "../path", "new\nline", &"x".repeat(257)] {
        assert!(durable_process_name(invalid).is_err());
    }
}
