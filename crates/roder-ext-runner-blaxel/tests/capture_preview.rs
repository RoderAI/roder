use std::time::Duration;

use roder_ext_runner_blaxel::forks::{CapturePreviewIntent, DeleteReceipt, Outcome};
use serde_json::{Value, json};
use tokio::time::Instant;

#[path = "support/forks.rs"]
mod support;
use support::{Server, response, runtime, source};

const NAME: &str = "verify-capture-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}
fn document() -> Value {
    let mut value = source();
    value["spec"]["runtime"]["ports"][0]["target"] = json!(4330);
    value
}
fn intent(server: &Server) -> CapturePreviewIntent {
    let mut expected = runtime();
    expected.ports[0].target = 4330;
    server
        .client()
        .capture_preview_intent("source-one", "inherited-parent", &expected, &"a".repeat(64))
        .unwrap()
}
fn preview() -> Value {
    json!({"metadata":{"name":NAME,"workspace":"snapshot-workspace","resourceName":"source-one","resourceType":"Sandbox"},
        "spec":{"port":4330,"public":true,"url":"https://owned.us-was-1.preview.bl.run","ttl":"10m","requestHeaders":{},"responseHeaders":{}},"status":"DEPLOYED"})
}
fn preparation() -> Vec<String> {
    vec![
        response(200, document()),
        response(404, Value::Null),
        response(200, document()),
    ]
}

#[tokio::test]
async fn retained_intent_precedes_single_non_forced_create_with_empty_header_overrides() {
    let mut replies = preparation();
    replies.push(response(200, preview()));
    let server = Server::start(replies).await;
    let client = server.client();
    let intent = intent(&server);
    let retained = serde_json::to_string(&intent).unwrap();
    assert!(!retained.contains("fixture-credential"));
    assert_eq!(intent.preview_name(), NAME);
    assert!(server.requests().await.is_empty());
    let restored: CapturePreviewIntent = serde_json::from_str(&retained).unwrap();
    let permit = client
        .prepare_capture_preview(&restored, deadline())
        .await
        .unwrap();
    let pin = client.create_capture_preview(permit).await.unwrap();
    assert_eq!(serde_json::to_value(pin).unwrap()["preview_name"], NAME);
    let requests = server.requests().await;
    assert_eq!(requests.len(), 4);
    assert!(requests[1].starts_with(&format!(
        "GET /v0/sandboxes/source-one/previews/{NAME} HTTP/1.1"
    )));
    assert!(requests[3].starts_with("POST /v0/sandboxes/source-one/previews HTTP/1.1"));
    assert!(!requests[3].contains("force="));
    let body: Value = serde_json::from_str(requests[3].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        body,
        json!({"metadata":{"name":NAME},"spec":{"port":4330,"public":true,"ttl":"10m","requestHeaders":{},"responseHeaders":{}}})
    );
}

#[tokio::test]
async fn lost_create_ack_is_reconciled_by_exact_get_without_another_post() {
    let mut replies = preparation();
    replies.extend([
        String::new(),
        response(200, document()),
        response(200, preview()),
    ]);
    let server = Server::start(replies).await;
    let client = server.client();
    let intent = intent(&server);
    let permit = client
        .prepare_capture_preview(&intent, deadline())
        .await
        .unwrap();
    let error = client.create_capture_preview(permit).await.err().unwrap();
    assert_eq!(error.outcome, Outcome::Unknown);
    let pin = client
        .reconcile_capture_preview(&intent, deadline())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_value(pin).unwrap()["preview_name"], NAME);
    let requests = server.requests().await;
    assert_eq!(
        requests.iter().filter(|v| v.starts_with("POST ")).count(),
        1
    );
    assert!(requests[5].starts_with(&format!(
        "GET /v0/sandboxes/source-one/previews/{NAME} HTTP/1.1"
    )));
}

#[tokio::test]
async fn create_conflict_and_server_failure_leave_no_automatic_retry() {
    for code in [409, 500] {
        let mut replies = preparation();
        replies.push(response(code, json!({"private":"private-canary"})));
        let server = Server::start(replies).await;
        let client = server.client();
        let permit = client
            .prepare_capture_preview(&intent(&server), deadline())
            .await
            .unwrap();
        let error = client.create_capture_preview(permit).await.err().unwrap();
        assert_eq!(error.outcome, Outcome::Unknown);
        assert!(!format!("{error:?} {error}").contains("private-canary"));
        assert_eq!(
            server
                .requests()
                .await
                .iter()
                .filter(|v| v.starts_with("POST "))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn preexisting_route_and_source_drift_prevent_creation() {
    let server = Server::start(vec![response(200, document()), response(200, preview())]).await;
    assert!(
        server
            .client()
            .prepare_capture_preview(&intent(&server), deadline())
            .await
            .is_err()
    );
    assert_eq!(server.requests().await.len(), 2);
    let mut drifted = document();
    drifted["metadata"]["externalId"] = json!("foreign-source");
    let server = Server::start(vec![
        response(200, document()),
        response(404, Value::Null),
        response(200, drifted),
    ])
    .await;
    let client = server.client();
    let permit = client
        .prepare_capture_preview(&intent(&server), deadline())
        .await
        .unwrap();
    assert!(client.create_capture_preview(permit).await.is_err());
    assert!(
        server
            .requests()
            .await
            .iter()
            .all(|v| v.starts_with("GET "))
    );
}

#[tokio::test]
async fn intent_and_permit_cannot_move_to_another_workspace_or_control_plane() {
    let server = Server::start(vec![response(200, document()), response(404, Value::Null)]).await;
    let client = server.client();
    let original = intent(&server);
    let mut value = serde_json::to_value(&original).unwrap();
    value["workspace"] = json!("other-workspace");
    let changed: CapturePreviewIntent = serde_json::from_value(value).unwrap();
    assert!(
        client
            .prepare_capture_preview(&changed, deadline())
            .await
            .is_err()
    );
    assert!(server.requests().await.is_empty());
    let permit = client
        .prepare_capture_preview(&original, deadline())
        .await
        .unwrap();
    let other = Server::start(vec![]).await;
    assert!(other.client().create_capture_preview(permit).await.is_err());
    assert!(other.requests().await.is_empty());
}

#[tokio::test]
async fn injected_headers_custom_routes_and_malformed_metadata_refuse_observation() {
    for (pointer, value) in [
        (
            "/spec/requestHeaders",
            json!({"Authorization":"private-canary"}),
        ),
        (
            "/spec/responseHeaders",
            json!({"X-Private":"private-canary"}),
        ),
        ("/spec/customDomain", json!("private-canary.test")),
        ("/spec/prefixUrl", json!("private-canary")),
        ("/metadata/resourceName", json!("other-source")),
        ("/metadata/resourceType", json!(3)),
        ("/status", json!(true)),
        ("/status", json!("DELETING")),
        ("/spec/ttl", json!("24h")),
        ("/spec/ttl", Value::Null),
    ] {
        let mut reply = preview();
        // customDomain/prefixUrl are omitted from the normal fixture.
        if let Some(slot) = reply.pointer_mut(pointer) {
            *slot = value;
        } else {
            reply["spec"][pointer.strip_prefix("/spec/").unwrap()] = value;
        }
        let server = Server::start(vec![response(200, document()), response(200, reply)]).await;
        let error = server
            .client()
            .reconcile_capture_preview(&intent(&server), deadline())
            .await
            .err()
            .unwrap();
        assert!(!format!("{error:?} {error}").contains("private-canary"));
        assert!(
            server
                .requests()
                .await
                .iter()
                .all(|v| v.starts_with("GET "))
        );
    }
}

#[tokio::test]
async fn deletion_acceptance_requires_separate_absence_read_and_never_uses_stopped_api() {
    let mut standby = document();
    standby["state"] = json!("STANDBY");
    let server = Server::start(vec![
        response(200, standby.clone()),
        response(200, preview()),
        response(200, standby.clone()),
        response(200, preview()),
        response(204, Value::Null),
        response(200, standby),
        response(404, Value::Null),
    ])
    .await;
    let client = server.client();
    let intent = intent(&server);
    let pin = client
        .reconcile_capture_preview(&intent, deadline())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        client
            .delete_capture_preview(&intent, &pin, deadline())
            .await
            .unwrap(),
        DeleteReceipt::Accepted
    );
    assert!(
        client
            .reconcile_capture_preview(&intent, deadline())
            .await
            .unwrap()
            .is_none()
    );
    let requests = server.requests().await;
    assert_eq!(
        requests.iter().filter(|v| v.starts_with("DELETE ")).count(),
        1
    );
    assert!(requests[4].starts_with(&format!(
        "DELETE /v0/sandboxes/source-one/previews/{NAME} HTTP/1.1"
    )));
    assert!(
        requests
            .iter()
            .all(|v| !v.contains("/process") && !v.contains("force="))
    );
}

#[tokio::test]
async fn changed_route_is_preserved_and_already_absent_route_needs_no_delete() {
    for present in [true, false] {
        let mut changed = preview();
        changed["spec"]["url"] = json!("https://other.us-was-1.preview.bl.run");
        let server = Server::start(vec![
            response(200, document()),
            response(200, preview()),
            response(200, document()),
            if present {
                response(200, changed)
            } else {
                response(404, Value::Null)
            },
        ])
        .await;
        let client = server.client();
        let intent = intent(&server);
        let pin = client
            .reconcile_capture_preview(&intent, deadline())
            .await
            .unwrap()
            .unwrap();
        let result = client
            .delete_capture_preview(&intent, &pin, deadline())
            .await;
        if present {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap(), DeleteReceipt::Absent);
        }
        assert!(
            server
                .requests()
                .await
                .iter()
                .all(|v| v.starts_with("GET "))
        );
    }
}

#[tokio::test]
async fn positive_route_absence_can_be_proved_after_parent_deletion_without_recreating_it() {
    for code in [404, 200, 500] {
        let server = Server::start(vec![response(code, Value::Null)]).await;
        let result = server
            .client()
            .capture_preview_absent(&intent(&server), deadline())
            .await;
        if code == 500 {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap(), code == 404);
        }
        let requests = server.requests().await;
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with(&format!(
            "GET /v0/sandboxes/source-one/previews/{NAME} HTTP/1.1"
        )));
    }
}

#[tokio::test]
async fn malformed_or_injected_create_receipts_are_unknown_without_exposing_their_payload() {
    for (pointer, value) in [
        ("/spec/port", json!(8080)),
        (
            "/spec/requestHeaders",
            json!({"Authorization":"private-canary"}),
        ),
        ("/spec/ttl", Value::Null),
        ("/status", json!("DEPLOYING")),
    ] {
        let mut invalid = preview();
        *invalid.pointer_mut(pointer).unwrap() = value;
        let mut replies = preparation();
        replies.push(response(200, invalid));
        let server = Server::start(replies).await;
        let client = server.client();
        let permit = client
            .prepare_capture_preview(&intent(&server), deadline())
            .await
            .unwrap();
        let error = client.create_capture_preview(permit).await.err().unwrap();
        assert_eq!(error.outcome, Outcome::Unknown);
        assert!(!format!("{error:?} {error}").contains("private-canary"));
        assert_eq!(
            server
                .requests()
                .await
                .iter()
                .filter(|v| v.starts_with("POST "))
                .count(),
            1
        );
    }
}
