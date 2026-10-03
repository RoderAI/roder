use super::*;

fn requested(id: &str, thread: &str) -> JsonRpcNotification {
    JsonRpcNotification {
        jsonrpc: "2.0".into(),
        method: "thread/toolExecutionRequested".into(),
        params: serde_json::json!({"threadId":thread,"turnId":"turn-1","requestId":id,
                "call":{"id":"call-1","name":"edit_draft","arguments":{"value":"private"}}}),
    }
}

#[tokio::test]
async fn only_bound_connection_receives_or_resolves_execution() {
    let bindings = ExecutorBindings::default();
    let (lease, _) = bindings.bind("owner", "thread-1", false).await.unwrap();
    assert!(bindings.bind("other", "thread-1", false).await.is_err());
    assert!(matches!(
        bindings
            .observe("other", requested("request-1", "thread-1"))
            .await,
        Delivery::Suppress
    ));
    assert!(matches!(
        bindings
            .observe("owner", requested("request-1", "thread-1"))
            .await,
        Delivery::Send(_)
    ));
    let mut resolution = ToolsResolveParams {
        request_id: "request-1".into(),
        output: "ok".into(),
        is_error: false,
        executor: Some(lease.clone()),
        turn_id: Some("wrong-turn".into()),
    };
    assert!(
        bindings
            .authorize_resolution("owner", &resolution)
            .await
            .is_err()
    );
    resolution.turn_id = Some("turn-1".into());
    assert!(
        bindings
            .authorize_resolution("other", &resolution)
            .await
            .is_err()
    );
    assert!(
        bindings
            .authorize_resolution("owner", &resolution)
            .await
            .is_ok()
    );
    assert_eq!(
        bindings
            .read("owner", &lease, "request-1")
            .await
            .unwrap()
            .unwrap()
            .state,
        "pending"
    );
}

#[tokio::test]
async fn takeover_cancels_old_calls_without_transferring_them() {
    let bindings = ExecutorBindings::default();
    let (old, _) = bindings.bind("owner", "thread-1", false).await.unwrap();
    bindings
        .observe("owner", requested("request-1", "thread-1"))
        .await;
    let (new, revoked) = bindings.bind("other", "thread-1", true).await.unwrap();
    assert_eq!(revoked.unwrap().requests, vec!["request-1"]);
    assert!(bindings.unbind("owner", &old).await.is_err());
    assert!(matches!(
        bindings
            .observe("other", requested("request-1", "thread-1"))
            .await,
        Delivery::Suppress
    ));
    assert_eq!(
        bindings
            .read("other", &new, "request-1")
            .await
            .unwrap()
            .unwrap()
            .state,
        "cancelled"
    );
}

#[tokio::test]
async fn disconnect_is_terminal_and_rebind_only_allows_fresh_requests() {
    let bindings = ExecutorBindings::default();
    bindings.bind("owner", "thread-1", false).await.unwrap();
    bindings
        .observe("owner", requested("request-1", "thread-1"))
        .await;
    assert_eq!(bindings.disconnect("owner").await.len(), 1);
    let (lease, _) = bindings.bind("new", "thread-1", false).await.unwrap();
    assert_eq!(
        bindings
            .read("new", &lease, "request-1")
            .await
            .unwrap()
            .unwrap()
            .state,
        "disconnected"
    );
    assert!(matches!(
        bindings
            .observe("new", requested("request-1", "thread-1"))
            .await,
        Delivery::Suppress
    ));
    assert!(matches!(
        bindings
            .observe("new", requested("request-2", "thread-1"))
            .await,
        Delivery::Send(_)
    ));
}
