use std::time::Duration;

use roder_api::remote_runner::{RemoteRunnerProvider, RemoteRunnerSession};
use roder_ext_runner_blaxel::{
    BlaxelRunnerProvider, CleanupMode, Redacted,
    forks::{DeleteReceipt, ForkReadiness, ForkRequest, ForkSessionOptions, Outcome},
};
use serde_json::{Value, json};

#[path = "support/forks.rs"]
mod support;
use support::{Server, response, runtime, source};

fn request() -> ForkRequest {
    ForkRequest::new(
        "child-one",
        "VEX_FORK_GENERATION",
        Redacted("fixture-generation".into()),
        Duration::from_secs(600),
    )
    .unwrap()
}

fn options() -> ForkSessionOptions {
    ForkSessionOptions {
        destination_id: "child-run".into(),
        working_dir: "/home/user/roder".into(),
        cleanup: CleanupMode::DeleteOnClose,
    }
}

fn child() -> Value {
    let mut child = source();
    child["metadata"]["name"] = json!("child-one");
    child["metadata"]["url"] = json!("https://child-one.bl.run");
    child["spec"]["runtime"]["envs"] = json!([
        {"name":"VEX_FORK_GENERATION","value":"fixture-generation","secret":false},
        {"name":"APP_CREDENTIAL","value":"private-application-secret","secret":true}]);
    child["spec"]["lifecycle"] =
        json!({"expirationPolicies":[{"type":"ttl-max-age","value":"10m","action":"delete"}]});
    child
}

#[tokio::test]
async fn source_pins_are_read_without_secret_values_or_workload_side_effects() {
    let server = Server::start(vec![response(200, source())]).await;
    assert_eq!(
        server.client().qualify_source("source-one").await.unwrap(),
        runtime()
    );
    let requests = server.requests().await;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /v0/sandboxes/source-one "));
    assert!(!requests[0].contains("show_secrets"));
}

#[tokio::test]
async fn adoption_only_reads_the_child_and_drops_inherited_identity_and_secrets() {
    let server = Server::start(vec![response(200, child())]).await;
    let session = server
        .client()
        .adopt_child(&request(), &runtime(), &options())
        .await
        .unwrap()
        .unwrap();
    let state = session.state();
    assert_eq!(state.session_id, "child-one");
    assert_eq!(state.destination_id, "child-run");
    assert_eq!(state.metadata["fork_adopted"], true);
    assert_eq!(state.metadata["workspace"], "snapshot-workspace");
    assert!(state.metadata.get("external_id").is_none());
    let serialized = serde_json::to_string(&state).unwrap();
    assert!(!serialized.contains("inherited-parent"));
    assert!(!serialized.contains("private-application-secret"));
    assert!(!serialized.contains("fixture-generation"));
    assert!(!serialized.contains("fixture-credential"));
    let requests = server.requests().await;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /v0/sandboxes/child-one?show_secrets=true "));
    assert!(
        BlaxelRunnerProvider::default()
            .rejoin_session(state)
            .await
            .is_err()
    );
    assert_eq!(server.requests().await.len(), 1);
}

#[tokio::test]
async fn standby_adoption_does_not_wake_or_refresh_inherited_processes() {
    let mut child = child();
    child["state"] = json!("STANDBY");
    let server = Server::start(vec![response(200, child)]).await;
    let session = server
        .client()
        .adopt_child(&request(), &runtime(), &options())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.state().metadata["paused"], true);
    assert_eq!(server.requests().await.len(), 1);
}

#[tokio::test]
async fn missing_and_partially_deployed_children_do_not_become_sessions() {
    let mut deploying = child();
    deploying["status"] = json!("DEPLOYING");
    let mut no_url = child();
    no_url["metadata"].as_object_mut().unwrap().remove("url");
    let mut empty_url = child();
    empty_url["metadata"]["url"] = json!("");
    for (body, ready) in [
        (None, ForkReadiness::Missing),
        (Some(deploying), ForkReadiness::Pending),
        (Some(no_url), ForkReadiness::Pending),
        (Some(empty_url), ForkReadiness::Pending),
    ] {
        let reply = || match &body {
            None => response(404, json!({})),
            Some(body) => response(200, body.clone()),
        };
        let server = Server::start(vec![reply(), reply()]).await;
        assert_eq!(
            server
                .client()
                .inspect_child(&request(), &runtime())
                .await
                .unwrap(),
            ready
        );
        assert!(
            server
                .client()
                .adopt_child(&request(), &runtime(), &options())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            server
                .requests()
                .await
                .iter()
                .all(|request| request.starts_with("GET "))
        );
    }
}

#[tokio::test]
async fn terminal_or_unknown_status_is_never_adopted_even_with_a_valid_endpoint() {
    for status in [
        "FAILED",
        "TERMINATED",
        "DELETING",
        "DELETED",
        "DEACTIVATED",
        "ARCHIVING",
        "ARCHIVED",
        "unexpected",
    ] {
        let mut child = child();
        child["status"] = json!(status);
        let server = Server::start(vec![response(200, child)]).await;
        assert!(
            server
                .client()
                .adopt_child(&request(), &runtime(), &options())
                .await
                .is_err()
        );
        assert_eq!(server.requests().await.len(), 1);
    }
}

#[tokio::test]
async fn marker_is_required_once_and_inherited_external_id_cannot_replace_it() {
    for envs in [
        json!([]),
        json!([{"name":"VEX_FORK_GENERATION","value":"old-generation","secret":false}]),
        json!([{"name":"VEX_FORK_GENERATION","value":"********","secret":false}]),
        json!([{"name":"VEX_FORK_GENERATION","value":"fixture-generation","secret":true}]),
        json!([{"name":"VEX_FORK_GENERATION","value":"fixture-generation","secret":false},{"name":"VEX_FORK_GENERATION","value":"fixture-generation","secret":false}]),
    ] {
        let mut child = child();
        child["spec"]["runtime"]["envs"] = envs;
        let server = Server::start(vec![response(200, child.clone()), response(200, child)]).await;
        assert!(
            server
                .client()
                .adopt_child(&request(), &runtime(), &options())
                .await
                .is_err()
        );
        assert!(server.client().delete_child(&request()).await.is_err());
        assert!(
            server
                .requests()
                .await
                .iter()
                .all(|request| request.starts_with("GET "))
        );
    }
}

#[tokio::test]
async fn source_capture_and_live_fork_recheck_configuration_before_mutating() {
    for pointer in [
        "/spec/runtime/image",
        "/spec/runtime/memory",
        "/spec/region",
        "/nodeGeneration",
        "/spec/runtime/ports/0/target",
    ] {
        let mut document = source();
        let value = match pointer {
            "/spec/runtime/memory" => json!(4096),
            "/spec/runtime/ports/0/target" => json!(4320),
            "/nodeGeneration" => json!("mk3.1"),
            _ => json!("changed"),
        };
        *document.pointer_mut(pointer).unwrap() = value;
        let server = Server::start(vec![
            response(200, document.clone()),
            response(200, document),
        ])
        .await;
        assert!(
            server
                .client()
                .create_snapshot("source-one", "checkpoint-one", &runtime())
                .await
                .is_err()
        );
        assert!(
            server
                .client()
                .fork_live("source-one", &request(), &runtime())
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
}

#[tokio::test]
async fn unsupported_attachments_are_rejected_before_capture_and_adoption() {
    for (key, value) in [
        ("volumes", json!([{"name":"shared-data"}])),
        ("mounts", json!(["/data"])),
        ("vpc", json!("private-vpc")),
        (
            "network",
            json!({"egress":{"mode":"dedicated","gateway":"static-ip"}}),
        ),
        ("network", json!({"proxy":{"routing":[]}})),
        ("network", json!({"subnet":"private"})),
        ("network", json!({"egress":{"mode":"shared","policies":[]}})),
    ] {
        let mut source = source();
        source["spec"][key] = value.clone();
        let mut child = child();
        child["spec"][key] = value;
        let server = Server::start(vec![response(200, source), response(200, child)]).await;
        assert!(
            server
                .client()
                .create_snapshot("source-one", "checkpoint-one", &runtime())
                .await
                .is_err()
        );
        assert!(
            server
                .client()
                .adopt_child(&request(), &runtime(), &options())
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
}

#[tokio::test]
async fn adoption_rechecks_runtime_ports_lifetime_and_workspace() {
    for (pointer, value) in [
        ("/spec/runtime/image", json!("foreign-image")),
        ("/spec/runtime/memory", json!(4096)),
        ("/spec/region", json!("us-pdx-1")),
        ("/nodeGeneration", json!("mk3.1")),
        ("/spec/runtime/ports/0/target", json!(4320)),
        ("/metadata/workspace", json!("other-workspace")),
        ("/metadata/name", json!("source-one")),
        ("/spec/lifecycle/expirationPolicies/0/value", json!("7d")),
        (
            "/spec/lifecycle/expirationPolicies/0/type",
            json!("ttl-idle"),
        ),
        ("/spec/lifecycle/expirationPolicies/0/action", json!("keep")),
    ] {
        let mut child = child();
        *child.pointer_mut(pointer).unwrap() = value;
        let server = Server::start(vec![response(200, child)]).await;
        assert!(
            server
                .client()
                .adopt_child(&request(), &runtime(), &options())
                .await
                .is_err()
        );
        assert_eq!(server.requests().await.len(), 1);
    }
}

#[tokio::test]
async fn untrusted_endpoints_are_rejected_without_forwarding_credentials() {
    for endpoint in [
        "http://child-one.bl.run",
        "https://child-one.bl.run.evil.example",
        "https://child-one.bl.run/process",
        "https://user:password@child-one.bl.run",
        "https://child-one.bl.run?redirect=1",
        "https://child-one.bl.run#fragment",
        "https://child-one.bl.run:8443",
    ] {
        let mut child = child();
        child["metadata"]["url"] = json!(endpoint);
        let server = Server::start(vec![response(200, child)]).await;
        assert!(
            server
                .client()
                .adopt_child(&request(), &runtime(), &options())
                .await
                .is_err()
        );
        assert_eq!(server.requests().await.len(), 1);
    }
}

#[tokio::test]
async fn adoption_rechecks_marker_after_an_earlier_ready_observation() {
    let mut replaced = child();
    replaced["spec"]["runtime"]["envs"][0]["value"] = json!("replacement-generation");
    let server = Server::start(vec![response(200, child()), response(200, replaced)]).await;
    assert_eq!(
        server
            .client()
            .inspect_child(&request(), &runtime())
            .await
            .unwrap(),
        ForkReadiness::Ready
    );
    assert!(
        server
            .client()
            .adopt_child(&request(), &runtime(), &options())
            .await
            .is_err()
    );
    assert_eq!(server.requests().await.len(), 2);
}

#[tokio::test]
async fn session_close_checks_current_generation_then_deletes_only_the_child() {
    let server = Server::start(vec![
        response(200, child()),
        response(200, child()),
        "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".into(),
    ])
    .await;
    let session = server
        .client()
        .adopt_child(&request(), &runtime(), &options())
        .await
        .unwrap()
        .unwrap();
    session.close().await.unwrap();
    let requests = server.requests().await;
    assert_eq!(requests.len(), 3);
    assert!(requests[1].starts_with("GET /v0/sandboxes/child-one?show_secrets=true "));
    assert!(requests[2].starts_with("DELETE /v0/sandboxes/child-one "));
    assert!(
        requests
            .iter()
            .all(|request| !request.contains("/source-one"))
    );
}

#[tokio::test]
async fn replaced_generation_cannot_be_deleted_by_an_old_session() {
    let mut replaced = child();
    replaced["spec"]["runtime"]["envs"][0]["value"] = json!("replacement-generation");
    let server = Server::start(vec![response(200, child()), response(200, replaced)]).await;
    let session = server
        .client()
        .adopt_child(&request(), &runtime(), &options())
        .await
        .unwrap()
        .unwrap();
    assert!(session.close().await.is_err());
    assert!(
        server
            .requests()
            .await
            .iter()
            .all(|request| request.starts_with("GET "))
    );
}

#[tokio::test]
async fn cleanup_acceptance_absence_and_unknown_are_distinct_even_when_not_ready() {
    let mut failed = child();
    failed["status"] = json!("FAILED");
    failed["spec"]["runtime"]["image"] = json!("changed");
    let server = Server::start(vec![
        response(200, failed),
        response(202, json!({})),
        response(404, json!({})),
    ])
    .await;
    assert_eq!(
        server.client().delete_child(&request()).await.unwrap(),
        DeleteReceipt::Accepted
    );
    assert_eq!(
        server.client().delete_child(&request()).await.unwrap(),
        DeleteReceipt::Absent
    );
    let server = Server::start(vec![response(200, child()), String::new()]).await;
    assert_eq!(
        server
            .client()
            .delete_child(&request())
            .await
            .unwrap_err()
            .outcome,
        Outcome::Unknown
    );
    assert_eq!(server.requests().await.len(), 2);
}
