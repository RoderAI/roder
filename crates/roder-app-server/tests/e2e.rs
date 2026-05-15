use roder_app_server::{AppServer, LocalAppClient};
use roder_core::{Runtime, fake_provider::FakeInferenceEngine};
use roder_protocol::{
    CreateSessionResult, ExtensionsListResult, JsonRpcRequest, ProvidersListResult,
    SessionsListResult, StartTurnParams, StartTurnResult, SystemStatusResult,
};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_app_server_e2e() {
    let engine = Arc::new(FakeInferenceEngine);
    let runtime = Arc::new(Runtime::from_engine(engine).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let status: SystemStatusResult = request(&client, "system/status", None).await;
    assert_eq!(status.provider, "fake-provider");
    assert_eq!(status.model, "fake-model");

    let extensions: ExtensionsListResult = request(&client, "extensions/list", None).await;
    assert!(extensions.extensions.is_empty());

    let providers: ProvidersListResult = request(&client, "providers/list", None).await;
    assert_eq!(providers.providers.len(), 1);
    assert_eq!(providers.providers[0].id, "fake-provider");

    let session: CreateSessionResult = request(&client, "sessions/create", None).await;
    assert!(!session.thread_id.is_empty());

    let sessions: SessionsListResult = request(&client, "sessions/list", None).await;
    assert!(sessions.sessions.is_empty());

    let params = StartTurnParams {
        thread_id: session.thread_id.clone(),
        message: "Hello".to_string(),
        provider_override: None,
        model_override: None,
    };
    let started: StartTurnResult = request(
        &client,
        "turns/start",
        Some(serde_json::to_value(params).unwrap()),
    )
    .await;
    assert!(!started.turn_id.is_empty());

    let mut kinds = Vec::new();
    for _ in 0..12 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if envelope.thread_id.as_deref() == Some(&session.thread_id) {
            kinds.push(envelope.kind);
        }
        if kinds.iter().any(|kind| kind == "turn.completed") {
            break;
        }
    }

    assert!(
        kinds.iter().any(|kind| kind == "turn.started"),
        "missing turn.started: {kinds:?}"
    );
    assert!(
        kinds.iter().any(|kind| kind == "inference.started"),
        "missing inference.started: {kinds:?}"
    );
    assert!(
        kinds.iter().any(|kind| kind == "inference.event_received"),
        "missing inference.event_received: {kinds:?}"
    );
    assert!(
        kinds.iter().any(|kind| kind == "turn.completed"),
        "missing turn.completed: {kinds:?}"
    );
}

async fn request<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: Option<serde_json::Value>,
) -> T {
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(method)),
        method: method.to_string(),
        params,
    };
    let res = client.send_request(req).await;
    assert!(
        res.error.is_none(),
        "RPC error for {method}: {:?}",
        res.error
    );
    serde_json::from_value(res.result.unwrap()).unwrap()
}
