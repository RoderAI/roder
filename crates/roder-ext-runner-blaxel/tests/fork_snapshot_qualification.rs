use std::time::Duration;

use roder_ext_runner_blaxel::{
    Redacted,
    forks::{ForkRequest, Outcome, Snapshot},
};
use serde_json::{Value, json};

#[path = "support/forks.rs"]
mod support;
use support::{Server, response, runtime, source};

fn snapshot() -> Value {
    json!({"id":"opaque-id","name":"checkpoint-one","workspace":"snapshot-workspace","status":"ready",
        "source":{"name":"source-one","kind":"sandbox","deleted":true},
        "spec":{"image":"sandbox/verify:immutable","memory":2048,"region":"us-was-1","generation":"mk3",
            "ports":[{"target":4319,"protocol":"HTTP"}]}})
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
async fn snapshot_fork_works_without_source_record_when_captured_configuration_matches() {
    let receipt: Snapshot = serde_json::from_value(snapshot()).unwrap();
    let server = Server::start(vec![
        response(200, snapshot()),
        response(
            200,
            json!({"name":"child-one","type":"sandbox","snapshotId":"opaque-id"}),
        ),
    ])
    .await;
    server
        .client()
        .fork_snapshot(&receipt, &request(), &runtime())
        .await
        .unwrap();
    let requests = server.requests().await;
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /v0/snapshots/opaque-id "));
    assert!(requests[1].starts_with("POST /v0/snapshots/opaque-id/fork "));
    assert!(
        requests
            .iter()
            .all(|request| !request.contains("/sandboxes/"))
    );
}

#[tokio::test]
async fn snapshot_fork_can_inspect_configuration_on_a_terminated_source_tombstone() {
    let mut snapshot = snapshot();
    snapshot["source"]["deleted"] = json!(false);
    let receipt: Snapshot = serde_json::from_value(snapshot.clone()).unwrap();
    let mut terminated = source();
    terminated["status"] = json!("TERMINATED");
    let server = Server::start(vec![
        response(200, snapshot),
        response(200, terminated),
        response(
            200,
            json!({"name":"child-one","type":"sandbox","snapshotId":"opaque-id"}),
        ),
    ])
    .await;
    server
        .client()
        .fork_snapshot(&receipt, &request(), &runtime())
        .await
        .unwrap();
    assert_eq!(server.requests().await.len(), 3);
}

#[tokio::test]
async fn snapshot_fork_tolerates_source_absence_after_its_snapshot_read() {
    let mut snapshot = snapshot();
    snapshot["source"]["deleted"] = json!(false);
    let receipt: Snapshot = serde_json::from_value(snapshot.clone()).unwrap();
    let server = Server::start(vec![
        response(200, snapshot),
        response(404, json!({})),
        response(
            200,
            json!({"name":"child-one","type":"sandbox","snapshotId":"opaque-id"}),
        ),
    ])
    .await;
    server
        .client()
        .fork_snapshot(&receipt, &request(), &runtime())
        .await
        .unwrap();
    assert_eq!(server.requests().await.len(), 3);
}

#[tokio::test]
async fn source_survival_cannot_silently_introduce_dedicated_egress() {
    let mut snapshot = snapshot();
    snapshot["source"]["deleted"] = json!(false);
    let receipt: Snapshot = serde_json::from_value(snapshot.clone()).unwrap();
    let mut source = source();
    source["spec"]["network"] = json!({"egress":{"mode":"dedicated","gateway":"static-ip"}});
    let server = Server::start(vec![response(200, snapshot), response(200, source)]).await;
    assert!(
        server
            .client()
            .fork_snapshot(&receipt, &request(), &runtime())
            .await
            .is_err()
    );
    assert!(
        server
            .requests()
            .await
            .iter()
            .all(|request| request.starts_with("GET "))
    );
}

#[tokio::test]
async fn fresh_snapshot_read_rejects_missing_unready_or_changed_snapshots() {
    let receipt: Snapshot = serde_json::from_value(snapshot()).unwrap();
    for (pointer, value) in [
        ("/status", json!("creating")),
        ("/status", json!("deleted")),
        ("/source/name", json!("other-source")),
        ("/name", json!("other-checkpoint")),
        ("/spec/image", json!("other-image")),
        ("/spec/memory", json!(4096)),
        ("/spec/region", json!("us-pdx-1")),
        ("/spec/generation", json!("mk2")),
        ("/spec/ports/0/target", json!(4320)),
        ("/spec", json!(null)),
    ] {
        let mut changed = snapshot();
        *changed.pointer_mut(pointer).unwrap() = value;
        let server = Server::start(vec![response(200, changed)]).await;
        assert_eq!(
            server
                .client()
                .fork_snapshot(&receipt, &request(), &runtime())
                .await
                .unwrap_err()
                .outcome,
            Outcome::Rejected
        );
        assert_eq!(server.requests().await.len(), 1);
    }
    let server = Server::start(vec![response(404, json!({}))]).await;
    assert!(
        server
            .client()
            .fork_snapshot(&receipt, &request(), &runtime())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn snapshot_volumes_are_not_accepted_after_source_deletion() {
    let receipt: Snapshot = serde_json::from_value(snapshot()).unwrap();
    let mut snapshot = snapshot();
    snapshot["spec"]["volumes"] = json!([{"name":"shared-data","mountPath":"/data"}]);
    let server = Server::start(vec![response(200, snapshot)]).await;
    assert!(
        server
            .client()
            .fork_snapshot(&receipt, &request(), &runtime())
            .await
            .is_err()
    );
    assert_eq!(server.requests().await.len(), 1);
}

#[tokio::test]
async fn duplicated_snapshot_ports_cannot_hide_a_missing_expected_port() {
    let mut expected = runtime();
    let mut extra = expected.ports[0].clone();
    extra.target = 4320;
    expected.ports.push(extra);
    let receipt: Snapshot = serde_json::from_value(snapshot()).unwrap();
    let mut duplicate = snapshot();
    duplicate["spec"]["ports"] = json!([
        {"target":4319,"protocol":"HTTP"},{"target":4319,"protocol":"HTTP"}]);
    let server = Server::start(vec![response(200, duplicate)]).await;
    assert!(
        server
            .client()
            .fork_snapshot(&receipt, &request(), &expected)
            .await
            .is_err()
    );
    assert_eq!(server.requests().await.len(), 1);
}
