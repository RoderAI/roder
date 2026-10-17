use roder_api::remote_runner::{RemoteRunnerSession, RunnerCommandObservation};
use roder_ext_runner_blaxel::{
    Redacted,
    forks::{BlaxelForkClient, OriginalSessionOptions},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[path = "support/forks.rs"]
mod support;
use support::{Server, response, source};
fn options() -> OriginalSessionOptions {
    OriginalSessionOptions {
        destination_id: "original-worker".into(),
        working_dir: "/workspace".into(),
    }
}

#[tokio::test]
async fn original_adoption_and_fresh_durable_observation_only_read_the_existing_vm() {
    let command = "retained-source-resume";
    let hash = Sha256::digest(command.as_bytes());
    let name = format!(
        "roder-durable-{}",
        hash[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let server = Server::start_with(|url| {
        let mut document = source();
        document["metadata"]["url"] = json!(url.trim_end_matches("/v0"));
        document["spec"]["runtime"]["envs"] = json!([{"name":"PRIVATE_CANARY","value":"do-not-export"}]);
        vec![response(200, document.clone()), response(200, document),
            response(200, json!({"name":name,"status":"running","exitCode":null,"stdout":"private-process-log"}))]
    }).await;
    let client = server.client();
    let observed = client
        .observe_original_source("source-one", "inherited-parent")
        .await
        .unwrap();
    let session = client
        .adopt_original_source(&observed, &options())
        .await
        .unwrap();
    let state = session.state();
    assert_eq!(state.provider_id, "blaxel");
    assert_eq!(state.session_id, "source-one");
    assert_eq!(state.destination_id, "original-worker");
    assert!(state.snapshot.is_none());
    assert_eq!(state.metadata["external_id"], "inherited-parent");
    assert_eq!(state.metadata["workspace"], "snapshot-workspace");
    assert_eq!(state.metadata["explicit_workspace"], true);
    assert_eq!(state.metadata["paused"], false);
    assert_eq!(state.metadata["cleanup"], "keep");
    assert_eq!(state.metadata["image"], "sandbox/verify:immutable");
    assert_eq!(state.metadata["memory"], 2048);
    assert!(state.metadata.get("fork_adopted").is_none());
    assert!(
        !serde_json::to_string(&state)
            .unwrap()
            .contains("do-not-export")
    );
    assert_eq!(
        session
            .observe_durable_command(&command.into())
            .await
            .unwrap(),
        RunnerCommandObservation::Running
    );
    session.close().await.unwrap(); // Local keep-close cannot delete or reconfigure the source.
    let requests = server.requests().await;
    assert_eq!(requests.len(), 3);
    assert!(
        requests[..2]
            .iter()
            .all(|r| r.starts_with("GET /v0/sandboxes/source-one "))
    );
    assert!(requests[2].starts_with(&format!("GET /process/{name} ")));
    assert!(
        requests
            .iter()
            .all(|r| r.contains("authorization: Bearer fixture-credential")
                && !r.contains("show_secrets"))
    );
    assert!(
        requests[..2]
            .iter()
            .all(|r| r.contains("x-blaxel-workspace: snapshot-workspace"))
    );
}

#[tokio::test]
async fn original_adoption_refuses_changed_runtime_endpoint_state_and_identity_without_mutation() {
    let changes: Vec<(&str, Value)> = vec![
        ("/metadata/name", json!("foreign")),
        ("/metadata/workspace", json!("default-workspace")),
        ("/metadata/externalId", json!("foreign")),
        ("/metadata/url", json!("https://other.bl.run")),
        ("/spec/runtime/image", json!("sandbox/changed:immutable")),
        ("/spec/runtime/memory", json!(4096)),
        ("/spec/region", json!("us-east-1")),
        ("/nodeGeneration", json!("mk3.1")),
        ("/spec/runtime/ports", json!([])),
        ("/state", json!("STANDBY")),
        ("/state", json!("PAUSED")),
        ("/status", json!("DEPLOYING")),
    ];
    for (pointer, value) in changes {
        let mut changed = source();
        *changed.pointer_mut(pointer).unwrap() = value;
        let server = Server::start(vec![response(200, source()), response(200, changed)]).await;
        let client = server.client();
        let observed = client
            .observe_original_source("source-one", "inherited-parent")
            .await
            .unwrap();
        assert!(
            client
                .adopt_original_source(&observed, &options())
                .await
                .is_err(),
            "{pointer}"
        );
        let requests = server.requests().await;
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|r| r.starts_with("GET ") && !r.contains("show_secrets"))
        );
    }
}

#[tokio::test]
async fn original_adoption_never_wakes_prior_standby_or_uses_a_foreign_client_or_invalid_local_options()
 {
    for mode in 0..7 {
        let mut document = source();
        if mode == 0 {
            document["state"] = json!("STANDBY");
        }
        let server = Server::start(vec![response(200, document)]).await;
        let client = server.client();
        let observed = client
            .observe_original_source("source-one", "inherited-parent")
            .await
            .unwrap();
        let mut options = options();
        let adopter = match mode {
            1 => BlaxelForkClient::with_base_url(
                Redacted("other-key".into()),
                "different-workspace",
                &server.url,
            )
            .unwrap(),
            2 => BlaxelForkClient::with_base_url(
                Redacted("other-key".into()),
                "snapshot-workspace",
                "http://127.0.0.1:1/v0",
            )
            .unwrap(),
            _ => server.client(),
        };
        match mode {
            3 => options.destination_id.clear(),
            4 => options.working_dir = "relative".into(),
            5 => options.destination_id = "bad\nline".into(),
            6 => options.working_dir = "/bad\0path".into(),
            _ => (),
        }
        assert!(
            adopter
                .adopt_original_source(&observed, &options)
                .await
                .is_err()
        );
        assert_eq!(server.requests().await.len(), 1);
    }
}

#[tokio::test]
async fn original_adoption_never_retries_or_creates_after_missing_unknown_or_failed_read() {
    for status in [302, 403, 404, 429, 500] {
        let server = Server::start(vec![
            response(200, source()),
            response(status, json!({"private":"provider-canary"})),
        ])
        .await;
        let client = server.client();
        let observed = client
            .observe_original_source("source-one", "inherited-parent")
            .await
            .unwrap();
        let error = client
            .adopt_original_source(&observed, &options())
            .await
            .err()
            .unwrap();
        assert!(!error.to_string().contains("provider-canary"));
        let requests = server.requests().await;
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|r| r.starts_with("GET ")));
    }
}
