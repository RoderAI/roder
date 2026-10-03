//! Authenticated executor contract exercised through real WebSocket connections.
use super::*;

async fn notification(socket: &mut Socket, method: &str) -> serde_json::Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(Ok(Message::Text(text))) = socket.next().await {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["method"] == method {
                    return value["params"].clone();
                }
            } else {
                panic!("socket closed");
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("notification timeout: {method}"))
}

#[tokio::test]
async fn hosted_executor_owns_execution_and_disconnect_never_replays() {
    let fixture = fixture("executor", RateLimitConfig::default(), true).await;
    let mut owner = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    let mut other = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    let mut outsider = connect(&fixture.url, "rk_test_tenant_b_writer")
        .await
        .unwrap();
    let directory = temp_dir("executor-workspace");
    let workspace = call(
        &mut owner,
        "workspace/create",
        serde_json::json!({
            "roots":[{"path":directory}],"defaultRootPath":directory
        }),
    )
    .await
    .result
    .unwrap()["workspace"]
        .clone();
    let start = call(&mut owner,"thread/start",serde_json::json!({
        "workspaceId":workspace["id"],"rootId":workspace["defaultRootId"],"model":"mock",
        "externalTools":[{"name":"acme_lookup","description":"lookup","parameters":{"type":"object"}}]
    })).await;
    assert!(start.error.is_none(), "{:?}", start.error);
    let thread = start.result.unwrap()["thread"]["id"].clone();
    let binding = serde_json::json!({"threadId":thread});
    assert!(
        call(&mut outsider, "tools/bind_executor", binding.clone())
            .await
            .error
            .is_some()
    );
    assert!(
        call(
            &mut owner,
            "turn/start",
            serde_json::json!({"threadId":thread,"prompt":"hello"})
        )
        .await
        .error
        .is_some()
    );
    let bound = call(&mut owner, "tools/bind_executor", binding.clone()).await;
    assert!(bound.error.is_none(), "{:?}", bound.error);
    let lease = bound.result.unwrap()["executor"].clone();
    assert!(
        call(&mut other, "tools/bind_executor", binding.clone())
            .await
            .error
            .is_some()
    );
    let turn = call(
        &mut owner,
        "turn/start",
        serde_json::json!({"threadId":thread,"prompt":"FAKE_EXTERNAL_TOOL lookup"}),
    )
    .await;
    assert!(turn.error.is_none(), "{:?}", turn.error);
    let request = notification(&mut owner, "thread/toolExecutionRequested").await;
    assert_eq!(request["executor"], lease);
    // Non-owner receives ordinary activity but never the execution request body.
    let drain = tokio::time::timeout(std::time::Duration::from_millis(100), async {
        while let Some(Ok(Message::Text(text))) = other.next().await {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_ne!(value["method"], "thread/toolExecutionRequested");
        }
    })
    .await;
    assert!(drain.is_err());
    let result = serde_json::json!({"executor":lease,"turnId":request["turnId"],
        "requestId":request["requestId"],"output":"ok","isError":false});
    assert!(
        call(&mut other, "tools/resolve", result.clone())
            .await
            .error
            .is_some()
    );
    assert!(
        call(&mut owner, "tools/resolve", result)
            .await
            .error
            .is_none()
    );
    let mut resolved = false;
    for _ in 0..50 {
        let state = call(
            &mut owner,
            "tools/execution_read",
            serde_json::json!({"executor":lease,"requestId":request["requestId"]}),
        )
        .await;
        if state.result.unwrap()["execution"]["state"] == "resolved" {
            resolved = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(resolved);
    let rebound = call(
        &mut other,
        "tools/bind_executor",
        serde_json::json!({"threadId":thread,"takeover":true}),
    )
    .await;
    assert!(rebound.error.is_none());
    assert_ne!(
        rebound.result.unwrap()["executor"]["leaseId"],
        lease["leaseId"]
    );
    assert!(
        call(
            &mut owner,
            "tools/execution_read",
            serde_json::json!({"executor":lease,"requestId":request["requestId"]})
        )
        .await
        .error
        .is_some()
    );
    let fresh = call(&mut owner,"thread/start",serde_json::json!({
        "workspaceId":workspace["id"],"rootId":workspace["defaultRootId"],"model":"mock",
        "externalTools":[{"name":"acme_lookup","description":"lookup","parameters":{"type":"object"}}]
    })).await.result.unwrap()["thread"]["id"].clone();
    assert!(
        call(
            &mut owner,
            "tools/bind_executor",
            serde_json::json!({"threadId":fresh})
        )
        .await
        .error
        .is_none()
    );
    assert!(
        call(
            &mut owner,
            "turn/start",
            serde_json::json!({"threadId":fresh,"prompt":"FAKE_EXTERNAL_TOOL lookup"})
        )
        .await
        .error
        .is_none()
    );
    let pending = notification(&mut owner, "thread/toolExecutionRequested").await;
    owner.close(None).await.unwrap();
    let mut new_lease = None;
    for _ in 0..50 {
        let bound = call(
            &mut other,
            "tools/bind_executor",
            serde_json::json!({"threadId":fresh}),
        )
        .await;
        if let Some(result) = bound.result {
            new_lease = Some(result["executor"].clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let new_lease = new_lease.expect("disconnect must release executor ownership");
    let state = call(
        &mut other,
        "tools/execution_read",
        serde_json::json!({"executor":new_lease,"requestId":pending["requestId"]}),
    )
    .await;
    assert_eq!(state.result.unwrap()["execution"]["state"], "disconnected");
    let replay = call(
        &mut other,
        "tools/resolve",
        serde_json::json!({"executor":new_lease,
        "turnId":pending["turnId"],"requestId":pending["requestId"],"output":"replay"}),
    )
    .await;
    assert!(replay.error.is_some());
    other.close(None).await.unwrap();
    outsider.close(None).await.unwrap();
    fixture.controller.stop().await.unwrap();
}

#[tokio::test]
async fn gateway_shutdown_terminalizes_pending_external_turn_before_returning() {
    let fixture = fixture("executor-shutdown", RateLimitConfig::default(), true).await;
    let mut owner = connect(&fixture.url, "rk_test_tenant_a_writer").await.unwrap();
    let directory = temp_dir("executor-shutdown-workspace");
    let workspace = call(&mut owner, "workspace/create", serde_json::json!({
        "roots":[{"path":directory}],"defaultRootPath":directory
    })).await.result.unwrap()["workspace"].clone();
    let thread = call(&mut owner, "thread/start", serde_json::json!({
        "workspaceId":workspace["id"],"model":"mock",
        "externalTools":[{"name":"acme_lookup","description":"lookup","parameters":{"type":"object"}}]
    })).await.result.unwrap()["thread"]["id"].clone();
    call(&mut owner, "tools/bind_executor", serde_json::json!({"threadId":thread})).await.result.unwrap();
    call(&mut owner, "turn/start", serde_json::json!({
        "threadId":thread,"prompt":"FAKE_EXTERNAL_TOOL lookup"
    })).await.result.unwrap();
    let pending = notification(&mut owner, "thread/toolExecutionRequested").await;
    let server = fixture.pool.lease("tenant-a").await.unwrap().server.clone();
    assert_eq!(server.runtime.active_turn_count().await, 1);
    fixture.controller.stop().await.unwrap();
    assert_eq!(server.runtime.active_turn_count().await, 0);
    let restored = server.handle_request(JsonRpcRequest {
        jsonrpc: "2.0".into(), id: Some(serde_json::json!("read-after-shutdown")),
        method: "thread/read".into(),
        params: Some(serde_json::json!({"threadId":thread,"includeTurns":true})),
    }).await.result.unwrap();
    let turns = restored["thread"]["turns"].as_array().unwrap();
    let turn = turns.iter().find(|turn| turn["id"] == pending["turnId"]).unwrap();
    assert_ne!(turn["status"], "inProgress", "{restored}");
    assert!(server.runtime.lifecycle_metrics().clean_shutdown_count > 0);
}
