use super::*;
use serde_json::json;

#[test]
fn status_reports_disconnected_by_default() {
    let bridge = ChromeBridge::new();
    let status = bridge.status();
    assert!(!status.connected);
    assert_eq!(status.client_count, 0);
    assert!(status.enabled);
    assert_eq!(status.mode, ChromePermissionMode::Assist);
}

#[tokio::test]
async fn dispatch_respects_explicit_disable() {
    let bridge = ChromeBridge::new();
    bridge.set_enabled(false);
    let reg = bridge.register_client(None, &json!({ "capabilities": ["chat"] }));
    drop(reg.commands);
    let err = bridge
        .dispatch(ChromeCommand::new("tabs/list"))
        .await
        .unwrap_err();
    assert_eq!(err, ChromeError::Disabled);
}

#[tokio::test]
async fn dispatch_round_trips_command_and_result() {
    let bridge = Arc::new(ChromeBridge::new());
    bridge.set_enabled(true);
    let mut reg = bridge.register_client(
        Some("127.0.0.1:9".to_string()),
        &json!({ "capabilities": ["tabs.list"] }),
    );

    // Simulated extension: read the command frame, echo a result.
    let echo = bridge.clone();
    let handle = tokio::spawn(async move {
        let frame = reg.commands.recv().await.expect("command frame");
        assert_eq!(frame["type"], "page/snapshot");
        assert_eq!(frame["tabId"], 7);
        let id = frame["id"].as_str().unwrap().to_string();
        echo.ingest_frame(
                Some(reg.client_id),
                json!({ "type": "command/result", "id": id, "ok": true, "result": { "title": "Example" } }),
            );
    });

    let result = bridge
        .dispatch(ChromeCommand::with_params(
            "page/snapshot",
            json!({ "tabId": 7 }),
        ))
        .await
        .expect("dispatch ok");
    assert_eq!(result["title"], "Example");
    handle.await.unwrap();

    let status = bridge.status();
    assert!(status.connected);
    assert_eq!(status.capabilities, vec!["tabs.list".to_string()]);
}

#[tokio::test]
async fn dispatch_times_out_without_response() {
    let bridge = ChromeBridge::with_timeout(Duration::from_millis(20));
    bridge.set_enabled(true);
    let reg = bridge.register_client(None, &json!({}));
    // Keep the receiver alive but never answer.
    let _keep = reg.commands;
    let err = bridge
        .dispatch(ChromeCommand::new("tabs/list"))
        .await
        .unwrap_err();
    assert_eq!(err, ChromeError::Timeout);
}

#[tokio::test]
async fn not_connected_when_no_clients() {
    let bridge = ChromeBridge::new();
    bridge.set_enabled(true);
    let err = bridge
        .dispatch(ChromeCommand::new("tabs/list"))
        .await
        .unwrap_err();
    assert_eq!(err, ChromeError::NotConnected);
}

#[tokio::test]
async fn cancelled_dispatch_removes_waiter_and_cancels_same_client() {
    let bridge = Arc::new(ChromeBridge::new());
    let mut reg = bridge.register_client(None, &json!({}));
    let caller = bridge.clone();
    let task =
        tokio::spawn(async move { caller.dispatch(ChromeCommand::new("page/keypress")).await });
    let command = reg.commands.recv().await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let cancel = reg.commands.recv().await.unwrap();
    assert_eq!(cancel["type"], "command/cancel");
    assert_eq!(cancel["targetId"], command["id"]);
    assert!(bridge.lock().pending.is_empty());
}
#[tokio::test]
async fn timed_out_dispatch_cancels_browser_command() {
    let bridge = ChromeBridge::with_timeout(Duration::from_millis(10));
    let mut reg = bridge.register_client(None, &json!({}));
    assert_eq!(
        bridge
            .dispatch(ChromeCommand::new("page/click"))
            .await
            .unwrap_err(),
        ChromeError::Timeout
    );
    let command = reg.commands.recv().await.unwrap();
    let cancel = reg.commands.recv().await.unwrap();
    assert_eq!(cancel["targetId"], command["id"]);
    assert!(bridge.lock().pending.is_empty());
}
