use roder_ext_runner_blaxel::forks::CaptureChannelPin;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::Instant;

#[path = "support/forks.rs"]
mod support;
use support::{Server, response, runtime, source};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}
fn document() -> Value {
    let mut value = source();
    value["spec"]["runtime"]["ports"][0]["target"] = json!(4330);
    value
}
fn expected() -> roder_ext_runner_blaxel::forks::ForkRuntime {
    let mut value = runtime();
    value.ports[0].target = 4330;
    value
}
fn preview() -> Value {
    json!({"metadata":{"name":"verify-capture-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"spec":{"port":4330,"public":true,"ttl":"10m","url":"https://owned.us-was-1.preview.bl.run"}})
}

#[tokio::test]
async fn observation_and_recovery_use_exact_authenticated_reads_and_preserve_public_pin() {
    let server = Server::start(vec![
        response(200, document()),
        response(200, preview()),
        response(200, document()),
        response(200, preview()),
    ])
    .await;
    let client = server.client();
    let channel = client
        .observe_capture_channel(
            "source-one",
            "inherited-parent",
            &expected(),
            &"a".repeat(64),
            deadline(),
        )
        .await
        .unwrap();
    let encoded = serde_json::to_string(channel.pin()).unwrap();
    assert!(!encoded.contains("fixture-credential"));
    let pin: CaptureChannelPin = serde_json::from_str(&encoded).unwrap();
    let recovered = client
        .recover_capture_channel(&pin, &expected(), deadline())
        .await
        .unwrap();
    assert!(recovered.pin() == channel.pin());
    let requests = server.requests().await;
    assert_eq!(requests.len(), 4);
    for (index, request) in requests.iter().enumerate() {
        let path = if index % 2 == 0 {
            "/v0/sandboxes/source-one"
        } else {
            "/v0/sandboxes/source-one/previews/verify-capture-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        };
        assert!(request.starts_with(&format!("GET {path} HTTP/1.1\r\n")));
        assert!(!request.contains("show_secrets") && !request.contains("force="));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-credential")
        );
    }
}

#[tokio::test]
async fn unrelated_original_identity_runtime_or_port_refuses_before_preview_lookup() {
    for pointer in [
        "/metadata/externalId",
        "/spec/runtime/memory",
        "/spec/runtime/ports/0/target",
        "/state",
    ] {
        let mut value = document();
        *value.pointer_mut(pointer).unwrap() = match pointer {
            "/metadata/externalId" => json!("foreign-source"),
            "/spec/runtime/memory" => json!(4096),
            "/state" => json!("STANDBY"),
            _ => json!(4320),
        };
        let server = Server::start(vec![response(200, value)]).await;
        assert!(
            server
                .client()
                .observe_capture_channel(
                    "source-one",
                    "inherited-parent",
                    &expected(),
                    &"a".repeat(64),
                    deadline()
                )
                .await
                .is_err()
        );
        assert_eq!(server.requests().await.len(), 1);
    }
}

#[tokio::test]
async fn private_unrelated_or_untrusted_preview_receipts_cannot_construct_channel() {
    for (pointer, replacement) in [
        ("/metadata/name", json!("other-preview")),
        ("/spec/port", json!(8080)),
        ("/spec/public", json!(false)),
        (
            "/spec/url",
            json!("https://private-canary@owned.preview.bl.run"),
        ),
        ("/spec/url", json!("https://evil.test/private-canary")),
    ] {
        let mut value = preview();
        *value.pointer_mut(pointer).unwrap() = replacement;
        let server = Server::start(vec![response(200, document()), response(200, value)]).await;
        let error = server
            .client()
            .observe_capture_channel(
                "source-one",
                "inherited-parent",
                &expected(),
                &"a".repeat(64),
                deadline(),
            )
            .await
            .err()
            .unwrap();
        assert!(!format!("{error:?} {error}").contains("private-canary"));
        assert_eq!(server.requests().await.len(), 2);
    }
}

#[tokio::test]
async fn retained_route_and_workspace_cannot_be_silently_replaced_on_recovery() {
    let server = Server::start(vec![
        response(200, document()),
        response(200, preview()),
        response(200, document()),
        response(200, preview()),
    ])
    .await;
    let client = server.client();
    let channel = client
        .observe_capture_channel(
            "source-one",
            "inherited-parent",
            &expected(),
            &"a".repeat(64),
            deadline(),
        )
        .await
        .unwrap();
    let mut value = serde_json::to_value(channel.pin()).unwrap();
    value["workspace"] = json!("another-workspace");
    let pin: CaptureChannelPin = serde_json::from_value(value.clone()).unwrap();
    assert!(
        client
            .recover_capture_channel(&pin, &expected(), deadline())
            .await
            .is_err()
    );
    assert_eq!(server.requests().await.len(), 2);
    value["workspace"] = json!("snapshot-workspace");
    value["origin"] = json!("https://different.preview.bl.run");
    let pin: CaptureChannelPin = serde_json::from_value(value).unwrap();
    assert!(
        client
            .recover_capture_channel(&pin, &expected(), deadline())
            .await
            .is_err()
    );
    assert_eq!(server.requests().await.len(), 4);
}

#[tokio::test]
async fn invalid_digest_or_expired_deadline_performs_no_control_plane_request() {
    let server = Server::start(vec![]).await;
    let client = server.client();
    assert!(
        client
            .observe_capture_channel(
                "source-one",
                "inherited-parent",
                &expected(),
                "private-canary",
                deadline()
            )
            .await
            .is_err()
    );
    assert!(
        client
            .observe_capture_channel(
                "source-one",
                "inherited-parent",
                &expected(),
                &"a".repeat(64),
                Instant::now()
            )
            .await
            .is_err()
    );
    assert!(server.requests().await.is_empty());
}
