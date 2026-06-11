//! Agent-node security tests (roadmap phase 67, Stage 1): loopback
//! TLS/mTLS only, fake runtime, no network beyond 127.0.0.1.

use std::path::PathBuf;
use std::sync::Arc;

use roder_api::extension::ExtensionRegistryBuilder;
use roder_app_server::agent_node::{AgentNodeOptions, serve_agent_node};
use roder_app_server::{AppServer, RemoteAppClient, RemoteNodeConnection};
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig};
use roder_protocol::JsonRpcRequest;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("roder-agent-node-{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app_server() -> Arc<AppServer> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    Arc::new(AppServer::new(runtime))
}

async fn start_node(label: &str) -> (roder_app_server::agent_node::AgentNodeController, PathBuf) {
    let state_dir = temp_dir(label);
    let controller = serve_agent_node(
        app_server(),
        AgentNodeOptions {
            listen: "127.0.0.1:0".to_string(),
            node_name: format!("test-node-{label}"),
            state_dir: state_dir.clone(),
            workspace: Some("/srv/workspace".to_string()),
        },
    )
    .await
    .unwrap();
    (controller, state_dir)
}

fn initialize_request() -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(1)),
        method: "initialize".to_string(),
        params: Some(serde_json::json!({})),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn enrolled_mtls_controller_connects_and_unenrolled_certs_fail() {
    let (node, _state) = start_node("mtls").await;
    let address = node.handle.listen_addr.to_string();

    // Out-of-band enrollment (operator pins the controller fingerprint).
    let controller = roder_app_server::agent_node::generate_identity("controller").unwrap();
    node.handle
        .trust
        .enroll(&controller.fingerprint, "test-controller")
        .unwrap();

    let client = RemoteAppClient::connect(RemoteNodeConnection {
        address: address.clone(),
        server_fingerprint: node.handle.fingerprint.clone(),
        controller_identity: controller,
        pairing_token: None,
    })
    .await
    .expect("enrolled mTLS controller connects");
    let response = client.request(initialize_request()).await;
    assert!(response.error.is_none(), "{:?}", response.error);
    let node_metadata = &response.result.unwrap()["node"];
    assert_eq!(node_metadata["nodeId"], node.handle.node_id.as_str());
    assert_eq!(node_metadata["authMode"], "mtls");
    assert_eq!(node_metadata["protocolVersion"], "roder.agent-node.v1");

    // An unenrolled certificate without a pairing token is rejected before
    // any request is handled.
    let stranger = roder_app_server::agent_node::generate_identity("stranger").unwrap();
    let denied = RemoteAppClient::connect(RemoteNodeConnection {
        address: address.clone(),
        server_fingerprint: node.handle.fingerprint.clone(),
        controller_identity: stranger,
        pairing_token: None,
    })
    .await;
    assert!(denied.is_err(), "unenrolled certificate must be rejected");

    // A controller that does not trust the node's fingerprint refuses to
    // connect (server identity pinning).
    let pinned_wrong = roder_app_server::agent_node::generate_identity("controller2").unwrap();
    let denied = RemoteAppClient::connect(RemoteNodeConnection {
        address,
        server_fingerprint: "0".repeat(64),
        controller_identity: pinned_wrong,
        pairing_token: None,
    })
    .await;
    assert!(denied.is_err(), "wrong server fingerprint must fail TLS trust");

    node.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_tokens_enroll_once_and_reject_reuse_expiry_and_query_strings() {
    let (node, _state) = start_node("pairing").await;
    let address = node.handle.listen_addr.to_string();

    // Valid token enrolls the connection's certificate.
    let (token, _preview) = node.handle.tokens.mint(time::Duration::minutes(5));
    let controller = roder_app_server::agent_node::generate_identity("paired").unwrap();
    let fingerprint = controller.fingerprint.clone();
    let client = RemoteAppClient::connect(RemoteNodeConnection {
        address: address.clone(),
        server_fingerprint: node.handle.fingerprint.clone(),
        controller_identity: controller.clone(),
        pairing_token: Some(token.clone()),
    })
    .await
    .expect("pairing token enrolls the controller");
    let response = client.request(initialize_request()).await;
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(
        response.result.unwrap()["node"]["authMode"],
        "pairing-token-enrolled"
    );
    assert!(node.handle.trust.is_trusted(&fingerprint));

    // The same certificate reconnects without any token (mTLS only).
    let reconnect = RemoteAppClient::connect(RemoteNodeConnection {
        address: address.clone(),
        server_fingerprint: node.handle.fingerprint.clone(),
        controller_identity: controller,
        pairing_token: None,
    })
    .await;
    assert!(reconnect.is_ok(), "enrolled cert reconnects without token");

    // Token reuse with a different certificate fails.
    let second = roder_app_server::agent_node::generate_identity("second").unwrap();
    let denied = RemoteAppClient::connect(RemoteNodeConnection {
        address: address.clone(),
        server_fingerprint: node.handle.fingerprint.clone(),
        controller_identity: second.clone(),
        pairing_token: Some(token),
    })
    .await;
    assert!(denied.is_err(), "single-use token must not enroll twice");

    // Expired tokens fail.
    let (expired, _) = node.handle.tokens.mint(time::Duration::seconds(-1));
    let denied = RemoteAppClient::connect(RemoteNodeConnection {
        address: address.clone(),
        server_fingerprint: node.handle.fingerprint.clone(),
        controller_identity: second.clone(),
        pairing_token: Some(expired),
    })
    .await;
    assert!(denied.is_err(), "expired token must be rejected");

    // Wrong tokens fail.
    let denied = RemoteAppClient::connect(RemoteNodeConnection {
        address: address.clone(),
        server_fingerprint: node.handle.fingerprint.clone(),
        controller_identity: second.clone(),
        pairing_token: Some("not-a-real-token".to_string()),
    })
    .await;
    assert!(denied.is_err(), "wrong token must be rejected");

    // Query-string tokens are always rejected, even valid ones.
    let (query_token, _) = node.handle.tokens.mint(time::Duration::minutes(5));
    let tls = roder_app_server::agent_node::client_tls_config(
        &node.handle.fingerprint,
        Some(&second),
    )
    .unwrap();
    let tcp = tokio::net::TcpStream::connect(&address).await.unwrap();
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(tls));
    let server_name = rustls::pki_types::ServerName::try_from("localhost".to_string()).unwrap();
    let tls_stream = connector.connect(server_name, tcp).await.unwrap();
    let request = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
        format!("wss://{address}/control?token={query_token}"),
    )
    .unwrap();
    let denied = tokio_tungstenite::client_async(request, tls_stream).await;
    assert!(denied.is_err(), "query-string tokens are forbidden");
    // ... and the token was not burned by the rejected attempt being read.
    assert!(
        node.handle.tokens.redeem(&query_token).is_ok(),
        "query-delivered token must never be read or redeemed"
    );

    node.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn revoked_controllers_are_rejected() {
    let (node, _state) = start_node("revoke").await;
    let address = node.handle.listen_addr.to_string();
    let controller = roder_app_server::agent_node::generate_identity("revocable").unwrap();
    node.handle
        .trust
        .enroll(&controller.fingerprint, "soon-revoked")
        .unwrap();

    let connected = RemoteAppClient::connect(RemoteNodeConnection {
        address: address.clone(),
        server_fingerprint: node.handle.fingerprint.clone(),
        controller_identity: controller.clone(),
        pairing_token: None,
    })
    .await;
    assert!(connected.is_ok());

    node.handle.trust.revoke(&controller.fingerprint).unwrap();
    let denied = RemoteAppClient::connect(RemoteNodeConnection {
        address,
        server_fingerprint: node.handle.fingerprint.clone(),
        controller_identity: controller,
        pairing_token: None,
    })
    .await;
    assert!(denied.is_err(), "revoked controller must be rejected");

    node.stop().await.unwrap();
}
