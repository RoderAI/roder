//! `RemoteAppClient` controller tests (roadmap phase 67, Stage 2): a full
//! thread/turn flow over loopback TLS/mTLS against a real app-server with
//! the offline fake provider, plus reconnect behavior.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use roder_api::events::RoderEvent;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::thread::ThreadStoreFactory;
use roder_app_server::agent_node::{AgentNodeOptions, serve_agent_node};
use roder_app_server::client::{AppClient, AppEventReceiver};
use roder_app_server::{
    AppServer, AppServerFeatureConfig, RemoteAppClient, RemoteNodeConnection,
};
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use roder_protocol::{
    JsonRpcRequest, ThreadStartParams, ThreadStartResult, TurnStartParams, TurnStartResult,
    WorkspaceCreateParams, WorkspaceCreateResult, WorkspaceRootInput,
};

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "roder-remote-client-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app_server(label: &str) -> Arc<AppServer> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: temp_dir(&format!("{label}-threads")),
    }));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let feature_config = AppServerFeatureConfig::default().with_workspace_registry_path(
        temp_dir(&format!("{label}-registry")).join("workspaces.json"),
    );
    Arc::new(AppServer::with_feature_config(runtime, feature_config))
}

async fn request<T: serde::de::DeserializeOwned>(
    client: &RemoteAppClient,
    method: &str,
    params: serde_json::Value,
) -> T {
    let response = client
        .request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params: Some(params),
        })
        .await;
    assert!(
        response.error.is_none(),
        "RPC error for {method}: {:?}",
        response.error
    );
    serde_json::from_value(response.result.unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_controller_drives_a_full_turn_with_events_over_mtls() {
    let node = serve_agent_node(
        app_server("turn"),
        AgentNodeOptions {
            listen: "127.0.0.1:0".to_string(),
            node_name: "turn-node".to_string(),
            state_dir: temp_dir("turn-state"),
            workspace: None,
        },
    )
    .await
    .unwrap();
    let controller = roder_app_server::agent_node::generate_identity("controller").unwrap();
    node.handle
        .trust
        .enroll(&controller.fingerprint, "test")
        .unwrap();

    let client = RemoteAppClient::connect(RemoteNodeConnection {
        address: node.handle.listen_addr.to_string(),
        server_fingerprint: node.handle.fingerprint.clone(),
        controller_identity: controller,
        pairing_token: None,
    })
    .await
    .unwrap();

    // Event subscription is part of the same authenticated connection.
    let mut events = client.subscribe_events();

    let workspace_dir = temp_dir("turn-workspace");
    let workspace: WorkspaceCreateResult = request(
        &client,
        "workspace/create",
        serde_json::to_value(WorkspaceCreateParams {
            name: None,
            roots: vec![WorkspaceRootInput {
                path: workspace_dir.display().to_string(),
                name: None,
            }],
            default_root_path: Some(workspace_dir.display().to_string()),
        })
        .unwrap(),
    )
    .await;
    let started: ThreadStartResult = request(
        &client,
        "thread/start",
        serde_json::to_value(ThreadStartParams {
            selection: None,
            workspace_id: workspace.workspace.id.clone(),
            root_id: Some(workspace.workspace.default_root_id.clone()),
            model: Some("mock".to_string()),
            model_provider: Some("mock".to_string()),
            reasoning: None,
            cwd: None,
            tool_allowlist: None,
            developer_instructions: None,
            external_tools: None,
            runner: None,
            ephemeral: false,
        })
        .unwrap(),
    )
    .await;

    let turn: TurnStartResult = request(
        &client,
        "turn/start",
        serde_json::to_value(TurnStartParams {
            thread_id: started.thread.id.clone(),
            input: Vec::new(),
            prompt: Some("hello remote node".to_string()),
            model_provider: None,
            model: None,
            reasoning: None,
            policy_mode: None,
            task_ledger_required: false,
        })
        .unwrap(),
    )
    .await;

    // The runtime's event envelopes stream to the controller.
    let completed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let envelope = events.recv().await.unwrap();
            if let RoderEvent::TurnCompleted(done) = envelope.event
                && done.turn_id == turn.turn_id
            {
                break;
            }
        }
    })
    .await;
    assert!(completed.is_ok(), "TurnCompleted must reach the controller");

    node.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnect_fails_pending_requests_explicitly_then_recovers() {
    let state_dir = temp_dir("reconnect-state");
    let node = serve_agent_node(
        app_server("reconnect"),
        AgentNodeOptions {
            listen: "127.0.0.1:0".to_string(),
            node_name: "reconnect-node".to_string(),
            state_dir: state_dir.clone(),
            workspace: None,
        },
    )
    .await
    .unwrap();
    let address = node.handle.listen_addr.to_string();
    let fingerprint = node.handle.fingerprint.clone();
    let controller = roder_app_server::agent_node::generate_identity("controller").unwrap();
    node.handle
        .trust
        .enroll(&controller.fingerprint, "test")
        .unwrap();

    let client = RemoteAppClient::connect(RemoteNodeConnection {
        address: address.clone(),
        server_fingerprint: fingerprint.clone(),
        controller_identity: controller,
        pairing_token: None,
    })
    .await
    .unwrap();
    let response = client
        .request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("first")),
            method: "thread/list".to_string(),
            params: Some(serde_json::json!({})),
        })
        .await;
    assert!(response.error.is_none());

    // Stop the node: in-flight/next requests fail explicitly, never hang
    // or replay.
    node.stop().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let response = client
        .request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("offline")),
            method: "thread/list".to_string(),
            params: Some(serde_json::json!({})),
        })
        .await;
    let error = response.error.expect("offline request fails explicitly");
    assert!(
        error.message.contains("offline") || error.message.contains("connection"),
        "{error:?}"
    );
    assert_eq!(response.id, Some(serde_json::json!("offline")));

    // Restart a node with the same identity/trust state on the same port;
    // the client reconnects with mTLS only (no token replay).
    let restarted = serve_agent_node(
        app_server("reconnect-2"),
        AgentNodeOptions {
            listen: address.clone(),
            node_name: "reconnect-node".to_string(),
            state_dir,
            workspace: None,
        },
    )
    .await
    .unwrap();

    let recovered = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let response = client
                .request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("recovered")),
                    method: "thread/list".to_string(),
                    params: Some(serde_json::json!({})),
                })
                .await;
            if response.error.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await;
    assert!(recovered.is_ok(), "client must reconnect after node restart");

    restarted.stop().await.unwrap();
}
