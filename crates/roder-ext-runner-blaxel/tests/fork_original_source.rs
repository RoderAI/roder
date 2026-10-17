use roder_ext_runner_blaxel::{BlaxelWorkspace, Redacted, forks::BlaxelForkClient};
use serde_json::{Value, json};

#[path = "support/forks.rs"]
mod support;
use support::{Server, response, runtime, source};

#[tokio::test]
async fn original_source_observation_is_one_scoped_read_without_wake_or_secret_export() {
    for state in ["RUNNING", "STANDBY"] {
        let mut document = source();
        document["state"] = json!(state);
        document["spec"]["envs"] = json!({"PRIVATE_CANARY": "never-export-this-provider-document"});
        let server = Server::start(vec![response(200, document)]).await;
        let profile = BlaxelWorkspace::with_base_url(
            Redacted("fixture-credential".into()),
            "snapshot-workspace",
            &server.url,
        )
        .unwrap();
        let client = BlaxelForkClient::from_workspace(&profile).unwrap();
        let observed = client
            .observe_original_source("source-one", "inherited-parent")
            .await
            .unwrap();
        assert_eq!(observed.is_running(), state == "RUNNING");
        assert_eq!(observed.name(), "source-one");
        assert_eq!(observed.external_id(), "inherited-parent");
        assert_eq!(observed.workspace(), "snapshot-workspace");
        assert_eq!(observed.endpoint_url(), "https://source-one.bl.run");
        assert_eq!(observed.control_base_url(), server.url);
        assert_eq!(observed.runtime(), &runtime());
        let requests = server.requests().await;
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("GET /v0/sandboxes/source-one HTTP/1.1"));
        assert!(requests[0].contains("authorization: Bearer fixture-credential"));
        assert!(requests[0].contains("x-blaxel-workspace: snapshot-workspace"));
        assert!(!requests[0].contains("show_secrets"));
    }
}

#[tokio::test]
async fn original_source_observation_refuses_foreign_missing_unready_or_unsupported_resources() {
    let changes: Vec<(&str, Value)> = vec![
        ("/metadata/name", json!("foreign")),
        ("/metadata/workspace", json!("default-workspace")),
        ("/metadata/externalId", json!("foreign")),
        ("/metadata/externalId", Value::Null),
        ("/metadata/url", Value::Null),
        ("/metadata/url", json!("https://foreign.test")),
        ("/metadata/url", json!("https://source-one.bl.run/path")),
        ("/metadata/url", json!("https://secret@source-one.bl.run")),
        ("/status", json!("DEPLOYING")),
        ("/status", json!("FAILED")),
        ("/state", json!("PAUSED")),
        ("/nodeGeneration", json!("mk2")),
        ("/spec/runtime/image", json!("")),
        ("/spec/runtime/memory", json!(0)),
    ];
    for (pointer, value) in changes {
        let mut document = source();
        *document.pointer_mut(pointer).unwrap() = value;
        let server = Server::start(vec![response(200, document)]).await;
        let error = server
            .client()
            .observe_original_source("source-one", "inherited-parent")
            .await
            .err()
            .unwrap();
        assert!(!error.to_string().contains("secret@"));
        assert_eq!(server.requests().await.len(), 1);
    }
    for attachment in [
        json!({"network":{"egress":{"mode":"dedicated"}}}),
        json!({"vpc":"private-network"}),
        json!({"volumes":[{"name":"shared-data"}]}),
    ] {
        let mut document = source();
        document["spec"]
            .as_object_mut()
            .unwrap()
            .extend(attachment.as_object().unwrap().clone());
        let server = Server::start(vec![response(200, document)]).await;
        assert!(
            server
                .client()
                .observe_original_source("source-one", "inherited-parent")
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn original_source_observation_does_not_retry_or_follow_redirects() {
    for status in [302, 403, 404, 429, 500] {
        let server = Server::start(vec![response(
            status,
            json!({"secret":"private-provider-error"}),
        )])
        .await;
        let error = server
            .client()
            .observe_original_source("source-one", "inherited-parent")
            .await
            .err()
            .unwrap();
        assert!(!error.to_string().contains("private-provider-error"));
        assert_eq!(server.requests().await.len(), 1);
    }
}
