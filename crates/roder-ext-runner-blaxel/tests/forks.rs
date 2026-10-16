use std::time::Duration;
use tokio::net::TcpListener;

use roder_ext_runner_blaxel::{
    Redacted,
    forks::{BlaxelForkClient, DeleteReceipt, ForkRequest, Outcome, Snapshot},
};
use serde_json::{Value, json};
#[path = "support/forks.rs"]
mod support;
use support::{Server, response, runtime, source};

fn snapshot(status: &str) -> Value {
    json!({"id":"opaque-id","name":"checkpoint-one","workspace":"snapshot-workspace","status":status,
        "source":{"name":"source-one","kind":"sandbox","deleted":false},
        "spec":{"image":"sandbox/verify:immutable","memory":2048,"region":"us-was-1","generation":"mk3","ports":[{"target":4319,"protocol":"HTTP"}]}})
}

fn request() -> ForkRequest {
    ForkRequest::new(
        "child-one",
        "VEX_FORK_GENERATION",
        Redacted("fixture-generation".into()),
        Duration::from_secs(600),
    )
    .unwrap()
}

#[tokio::test]
async fn snapshot_create_lookup_and_cleanup_preserve_explicit_workspace_identity() {
    let server = Server::start(vec![
        response(200, source()),
        response(201, snapshot("creating")),
        response(200, snapshot("ready")),
        "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".into(),
        response(404, json!({})),
        response(404, json!({})),
    ])
    .await;
    let client = server.client();
    let created = client
        .create_snapshot("source-one", "checkpoint-one", &runtime())
        .await
        .unwrap();
    assert!(!created.is_ready());
    assert!(
        client
            .get_snapshot(&created.id)
            .await
            .unwrap()
            .unwrap()
            .is_ready()
    );
    assert_eq!(
        client.delete_snapshot(&created.id).await.unwrap(),
        DeleteReceipt::Accepted
    );
    assert!(client.get_snapshot(&created.id).await.unwrap().is_none());
    assert_eq!(
        client.delete_snapshot(&created.id).await.unwrap(),
        DeleteReceipt::Absent
    );
    let requests = server.requests().await;
    assert_eq!(requests.len(), 6);
    for request in &requests {
        let headers = request.split("\r\n\r\n").next().unwrap().to_lowercase();
        assert!(headers.contains("authorization: bearer fixture-credential"));
        assert!(headers.contains("x-blaxel-workspace: snapshot-workspace"));
        assert!(headers.contains("blaxel-version: 2026-04-28"));
    }
    let body: Value = serde_json::from_str(requests[1].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        body,
        json!({"name":"checkpoint-one","source":{"name":"source-one"}})
    );
    assert!(requests[3].starts_with("DELETE /v0/snapshots/opaque-id "));
}

#[tokio::test]
async fn live_and_snapshot_forks_send_new_lifetime_and_marker_without_external_id() {
    let server = Server::start(vec![
        response(200, source()),
        response(200, json!({"name":"child-one","type":"sandbox"})),
        response(200, snapshot("ready")),
        response(200, source()),
        response(
            200,
            json!({"name":"child-one","type":"sandbox","snapshotId":"opaque-id"}),
        ),
    ])
    .await;
    let client = server.client();
    client
        .fork_live("source-one", &request(), &runtime())
        .await
        .unwrap();
    let snapshot: Snapshot = serde_json::from_value(snapshot("ready")).unwrap();
    client
        .fork_snapshot(&snapshot, &request(), &runtime())
        .await
        .unwrap();
    let requests = server.requests().await;
    assert!(requests[1].starts_with("POST /v0/sandboxes/source-one/fork "));
    assert!(requests[4].starts_with("POST /v0/snapshots/opaque-id/fork "));
    for request in requests
        .into_iter()
        .filter(|request| request.starts_with("POST"))
    {
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["targetName"], "child-one");
        assert_eq!(body["envs"][0]["value"], "fixture-generation");
        assert_eq!(body["lifecycle"]["expirationPolicies"][0]["value"], "600s");
        assert!(body.get("externalId").is_none());
    }
}

#[tokio::test]
async fn unknown_response_is_not_replayed_and_errors_do_not_leak_provider_bodies() {
    for reply in [
        String::new(),
        response(503, json!({"secret":"provider-secret"})),
        response(200, json!({"name":"foreign-child","type":"sandbox"})),
        response(
            200,
            json!({"name":"child-one","type":"sandbox","snapshotId":"untracked"}),
        ),
    ] {
        let server = Server::start(vec![response(200, source()), reply]).await;
        let error = server
            .client()
            .fork_live("source-one", &request(), &runtime())
            .await
            .unwrap_err();
        assert_eq!(error.outcome, Outcome::Unknown);
        let displayed = format!("{error:?} {error}");
        assert!(!displayed.contains("provider-secret"));
        assert!(!displayed.contains("fixture-credential"));
        assert_eq!(server.requests().await.len(), 2);
    }
}

#[tokio::test]
async fn conflicts_missing_sources_and_throttling_are_not_adopted_or_retried() {
    for status in [403, 404, 409, 429] {
        let reply = response(status, json!({"secret":"provider-secret"}))
            .replace("Content-Type:", "Retry-After: 7\r\nContent-Type:");
        let server = Server::start(vec![response(200, source()), reply]).await;
        let error = server
            .client()
            .fork_live("source-one", &request(), &runtime())
            .await
            .unwrap_err();
        assert_eq!(error.outcome, Outcome::Rejected);
        assert_eq!(error.status, Some(status));
        assert_eq!(error.retry_after, Some(Duration::from_secs(7)));
        assert_eq!(server.requests().await.len(), 2);
    }
}

#[tokio::test]
async fn redirects_never_forward_credentials() {
    let receiver = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = Server::start(vec![response(200, source()), format!("HTTP/1.1 307 Redirect\r\nLocation: http://{}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",receiver.local_addr().unwrap())]).await;
    let error = server
        .client()
        .fork_live("source-one", &request(), &runtime())
        .await
        .unwrap_err();
    assert_eq!(error.status, Some(307));
    assert_eq!(error.outcome, Outcome::Unknown);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), receiver.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn snapshot_lookup_and_create_reject_foreign_receipts() {
    let mut foreign = snapshot("ready");
    foreign["workspace"] = json!("other-workspace");
    let server = Server::start(vec![
        response(200, foreign.clone()),
        response(200, source()),
        response(201, foreign),
    ])
    .await;
    assert!(server.client().get_snapshot("opaque-id").await.is_err());
    let error = server
        .client()
        .create_snapshot("source-one", "checkpoint-one", &runtime())
        .await
        .unwrap_err();
    assert_eq!(error.outcome, Outcome::Unknown);
}

#[tokio::test]
async fn reconciliation_checks_all_catalog_pages_and_rejects_duplicates() {
    let page = |data: Value, more, cursor| {
        response(
            200,
            json!({"data":data,"meta":{"hasMore":more,"nextCursor":cursor}}),
        )
    };
    let server = Server::start(vec![
        page(json!([]), true, Some("opaque/cursor")),
        page(json!([snapshot("ready")]), false, None),
    ])
    .await;
    assert!(
        server
            .client()
            .find_snapshot("source-one", "checkpoint-one")
            .await
            .unwrap()
            .unwrap()
            .is_ready()
    );
    assert!(server.requests().await[1].contains("cursor=opaque%2Fcursor"));
    let server = Server::start(vec![page(
        json!([snapshot("ready"), snapshot("ready")]),
        false,
        None,
    )])
    .await;
    assert!(
        server
            .client()
            .find_snapshot("source-one", "checkpoint-one")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn invalid_or_unready_inputs_do_not_dispatch() {
    let server = Server::start(vec![]).await;
    let client = server.client();
    for status in ["creating", "failed", "deleted", "deleting"] {
        let snapshot: Snapshot = serde_json::from_value(snapshot(status)).unwrap();
        assert_eq!(
            client
                .fork_snapshot(&snapshot, &request(), &runtime())
                .await
                .unwrap_err()
                .outcome,
            Outcome::Rejected
        );
    }
    assert!(
        client
            .fork_live("child-one", &request(), &runtime())
            .await
            .is_err()
    );
    assert!(
        client
            .create_snapshot("../source", "checkpoint-one", &runtime())
            .await
            .is_err()
    );
    assert!(client.get_snapshot("..").await.is_err());
    assert!(server.requests().await.is_empty());
}

#[test]
fn profiles_and_request_debug_keep_credentials_and_marker_values_private() {
    assert!(!format!("{:?}", request()).contains("fixture-generation"));
    for endpoint in [
        "https://foreign.example/v0",
        "https://api.blaxel.ai/v0?key=secret",
        "https://user:password@api.blaxel.ai/v0",
        "http://api.blaxel.ai/v0",
    ] {
        assert!(
            BlaxelForkClient::with_base_url(
                Redacted("fixture-key".into()),
                "snapshot-workspace",
                endpoint
            )
            .is_err()
        );
    }
    assert!(BlaxelForkClient::new(Redacted(String::new()), "snapshot-workspace").is_err());
    assert!(BlaxelForkClient::new(Redacted("fixture-key".into()), "bad\r\nworkspace").is_err());
    assert!(
        ForkRequest::new(
            "child-one",
            "MARKER",
            Redacted("marker".into()),
            Duration::ZERO
        )
        .is_err()
    );
}

#[tokio::test]
async fn reconciliation_uses_workspace_catalog_after_source_deletion() {
    let mut deleted = snapshot("ready");
    deleted["source"]["deleted"] = json!(true);
    let mut other_source = deleted.clone();
    other_source["source"]["name"] = json!("other-source");
    other_source["id"] = json!("other-id");
    let server = Server::start(vec![response(
        200,
        json!({"data":[other_source,deleted],"meta":{"hasMore":false}}),
    )])
    .await;
    assert!(
        server
            .client()
            .find_snapshot("source-one", "checkpoint-one")
            .await
            .unwrap()
            .unwrap()
            .source
            .deleted
    );
    assert!(server.requests().await[0].starts_with("GET /v0/snapshots?limit=100 "));
}

#[tokio::test]
async fn repeated_cursor_and_partial_catalog_never_claim_snapshot_absence() {
    for replies in [
        vec![
            response(
                200,
                json!({"data":[],"meta":{"hasMore":true,"nextCursor":"same"}}),
            ),
            response(
                200,
                json!({"data":[],"meta":{"hasMore":true,"nextCursor":"same"}}),
            ),
        ],
        vec![response(404, json!({}))],
        vec![response(200, json!({"data":[],"meta":{"hasMore":true}}))],
    ] {
        let server = Server::start(replies).await;
        assert!(
            server
                .client()
                .find_snapshot("source-one", "checkpoint-one")
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn oversized_and_invalid_mutation_receipts_are_unknown() {
    let oversized = response(200, json!({"padding":"x".repeat(1024*1024)}));
    for reply in [
        oversized,
        response(200, json!({})),
        "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\n???".into(),
    ] {
        let server = Server::start(vec![response(200, source()), reply]).await;
        assert_eq!(
            server
                .client()
                .create_snapshot("source-one", "checkpoint-one", &runtime())
                .await
                .unwrap_err()
                .outcome,
            Outcome::Unknown
        );
        assert_eq!(server.requests().await.len(), 2);
    }
}

#[tokio::test]
async fn deletion_timeout_retains_unknown_outcome_and_does_not_claim_absence() {
    let server = Server::start(vec![String::new()]).await;
    let error = server
        .client()
        .delete_snapshot("opaque-id")
        .await
        .unwrap_err();
    assert_eq!(error.outcome, Outcome::Unknown);
    assert_eq!(server.requests().await.len(), 1);
}

#[tokio::test]
async fn unexpected_snapshot_handle_is_retained_without_logging_it() {
    let server = Server::start(vec![
        response(200, source()),
        response(
            200,
            json!({"name":"child-one","type":"sandbox","snapshotId":"unexpected-provider-handle"}),
        ),
    ])
    .await;
    let error = server
        .client()
        .fork_live("source-one", &request(), &runtime())
        .await
        .unwrap_err();
    assert_eq!(
        error.fork_receipt().unwrap().snapshot_id.as_deref(),
        Some("unexpected-provider-handle")
    );
    assert!(!format!("{error:?} {error}").contains("unexpected-provider-handle"));
}
