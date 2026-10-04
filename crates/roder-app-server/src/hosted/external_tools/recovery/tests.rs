use std::{path::Path, sync::Arc, time::Duration};

use roder_api::extension::ExtensionRegistryBuilder;
use roder_core::{Runtime, fake_provider::FakeInferenceEngine};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use roder_protocol::JsonRpcRequest;
use serde_json::{Value, json};

fn server(directory: &Path) -> Arc<crate::AppServer> {
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(Arc::new(FakeInferenceEngine));
    registry.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: directory.join("threads"),
    }));
    Arc::new(crate::AppServer::new(Arc::new(
        Runtime::new(registry.build().unwrap(), Default::default()).unwrap(),
    )))
}

async fn request(server: &crate::AppServer, method: &str, params: Value) -> Value {
    let response = server
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: method.into(),
            params: Some(params),
        })
        .await;
    assert!(response.error.is_none(), "{:?}", response.error);
    response.result.unwrap()
}

async fn hosted(
    server: &crate::AppServer,
    connection: &str,
    method: &str,
    params: Value,
) -> roder_protocol::JsonRpcResponse {
    crate::hosted::executor_gateway::dispatch(
        server,
        connection,
        &JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: method.into(),
            params: Some(params),
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "subprocess fixture: invoked by recovery test with an isolated directory"]
async fn crash_fixture() {
    let directory =
        std::path::PathBuf::from(std::env::var("RODER_EXECUTION_CRASH_FIXTURE").unwrap());
    let server = server(&directory);
    server.external_tool_executors.monitor(&server);
    let workspace = request(
        &server,
        "workspace/create",
        json!({"roots":[{"path":directory}]}),
    )
    .await;
    let thread = request(&server, "thread/start", json!({
        "workspaceId":workspace["workspace"]["id"], "rootId":workspace["workspace"]["defaultRootId"],
        "model":"mock", "externalTools":[{"name":"acme_lookup", "description":"lookup", "parameters":{"type":"object"}}]
    })).await["thread"]["id"].as_str().unwrap().to_owned();
    let lease = hosted(
        &server,
        "old",
        "tools/bind_executor",
        json!({"threadId":thread}),
    )
    .await
    .result
    .unwrap()["executor"]
        .clone();
    let mut notifications = server.subscribe_notifications();
    request(
        &server,
        "turn/start",
        json!({"threadId":thread,"prompt":"FAKE_EXTERNAL_TOOL lookup"}),
    )
    .await;
    let execution = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let notification = notifications.recv().await.unwrap();
            if notification.method == "thread/toolExecutionRequested" {
                break notification.params;
            }
        }
    })
    .await
    .unwrap();
    // Simulate an external effect that finished before the process lost its acknowledgement.
    std::fs::write(directory.join("effect"), "applied once").unwrap();
    std::fs::write(
        directory.join("request.json"),
        serde_json::to_vec(&json!({"execution":execution,"lease":lease})).unwrap(),
    )
    .unwrap();
    // Bypass destructors and graceful cancellation: the new host must derive
    // uncertainty from durable history, not an in-memory disconnect receipt.
    std::process::exit(0);
}

#[tokio::test]
async fn process_loss_restores_uncertainty_and_rejects_old_or_rebound_results() {
    let directory = tempfile::tempdir().unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "hosted::external_tools::recovery::tests::crash_fixture",
            "--ignored",
        ])
        .env("RODER_EXECUTION_CRASH_FIXTURE", directory.path())
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    let saved: Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("request.json")).unwrap())
            .unwrap();
    let execution = &saved["execution"];
    let server = server(directory.path());
    let lease = hosted(
        &server,
        "replacement",
        "tools/bind_executor",
        json!({"threadId":execution["threadId"]}),
    )
    .await
    .result
    .unwrap()["executor"]
        .clone();
    for _ in 0..2 {
        let read = hosted(
            &server,
            "replacement",
            "tools/execution_read",
            json!({"executor":lease,"requestId":execution["requestId"]}),
        )
        .await;
        assert!(read.error.is_none(), "{:?}", read.error);
        let read = read.result.unwrap()["execution"].clone();
        assert_eq!(read["state"], "uncertain");
        assert_eq!(read["isError"], true);
        assert_eq!(read["turnId"], execution["turnId"]);
        assert!(read.get("arguments").is_none());
    }
    for executor in [&saved["lease"], &lease] {
        let response = hosted(&server, "replacement", "tools/resolve", json!({
            "executor":executor,"requestId":execution["requestId"],"turnId":execution["turnId"],"output":"late","isError":false
        })).await;
        assert!(
            response.error.is_some(),
            "a recovered request must never become pending again"
        );
    }
    assert_eq!(
        std::fs::read_to_string(directory.path().join("effect")).unwrap(),
        "applied once"
    );
    assert!(
        server
            .runtime
            .pending_external_executions()
            .await
            .is_empty()
    );
    let unknown = hosted(
        &server,
        "replacement",
        "tools/execution_read",
        json!({"executor":lease,"requestId":"unknown"}),
    )
    .await;
    assert!(unknown.result.unwrap()["execution"].is_null());
    let outsider = hosted(
        &server,
        "other-connection",
        "tools/execution_read",
        json!({"executor":lease,"requestId":execution["requestId"]}),
    )
    .await;
    assert!(outsider.error.is_some());
    let other_thread = server.runtime.create_thread(None).await.unwrap().thread_id;
    let (other_lease, _) = server
        .external_tool_executors
        .bind("replacement", &other_thread, false)
        .await
        .unwrap();
    let hidden = server
        .external_tool_executors
        .read_with_history(
            "replacement",
            &other_lease,
            execution["requestId"].as_str().unwrap(),
            &server.runtime,
        )
        .await
        .unwrap();
    assert!(
        hidden.is_none(),
        "history must stay within the bound thread"
    );

    // A later restart can recover a durable terminal receipt too.
    use roder_api::events::{ExternalToolCallOutcome, ExternalToolCallResolved, RoderEvent};
    for outcome in [
        ExternalToolCallOutcome::Resolved,
        ExternalToolCallOutcome::TimedOut,
        ExternalToolCallOutcome::Cancelled,
    ] {
        let expected = serde_json::to_value(&outcome).unwrap();
        let is_error = !matches!(outcome, ExternalToolCallOutcome::Resolved);
        server
            .runtime
            .emit(RoderEvent::ExternalToolCallResolved(
                ExternalToolCallResolved {
                    thread_id: execution["threadId"].as_str().unwrap().into(),
                    turn_id: execution["turnId"].as_str().unwrap().into(),
                    request_id: execution["requestId"].as_str().unwrap().into(),
                    tool_id: execution["call"]["id"].as_str().unwrap().into(),
                    tool_name: execution["call"]["name"].as_str().unwrap().into(),
                    outcome,
                    is_error,
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        let recovered = self::server(directory.path());
        let (lease, _) = recovered
            .external_tool_executors
            .bind(
                "replacement",
                execution["threadId"].as_str().unwrap(),
                false,
            )
            .await
            .unwrap();
        let terminal = recovered
            .external_tool_executors
            .read_with_history(
                "replacement",
                &lease,
                execution["requestId"].as_str().unwrap(),
                &recovered.runtime,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(terminal.state, expected.as_str().unwrap());
        assert_eq!(terminal.is_error, is_error);
    }
}
