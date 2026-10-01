use super::*;
use futures::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn failed_release_keeps_input_armed_and_attempts_other_releases_and_mask() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tab = DirectTab::Page {
        websocket: format!("ws://{}", listener.local_addr().unwrap()),
        target_id: "test".into(),
    };
    let failed = Arc::new(AtomicBool::new(false));
    let released = Arc::new(Mutex::new(Vec::new()));
    let recorded = released.clone();
    let server = tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let failed = failed.clone();
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    let method = request["method"].as_str().unwrap();
                    let result = match method {
                        "Page.getFrameTree" => json!({"frameTree":{"frame":{"id":"frame"}}}),
                        "Page.createIsolatedWorld" => json!({"executionContextId":1}),
                        "Runtime.evaluate" => {
                            recorded.lock().unwrap().push("mask".to_string());
                            json!({"result":{"value":null}})
                        }
                        "Input.dispatchKeyEvent" => {
                            recorded
                                .lock()
                                .unwrap()
                                .push(request["params"]["key"].as_str().unwrap().to_string());
                            json!({})
                        }
                        _ => json!({}),
                    };
                    let response = if method == "Input.dispatchKeyEvent"
                        && request["params"]["key"] == "A"
                        && !failed.swap(true, Ordering::SeqCst)
                    {
                        json!({"id":request["id"],"error":{"message":"injected release failure"}})
                    } else {
                        json!({"id":request["id"],"result":result})
                    };
                    if socket
                        .send(Message::Text(response.to_string().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
    });
    let pending = Pending::default();
    pending.before(
        "Input.dispatchKeyEvent",
        &json!({"type":"rawKeyDown","key":"B"}),
    );
    pending.before(
        "Input.dispatchKeyEvent",
        &json!({"type":"rawKeyDown","key":"A"}),
    );
    pending.mask(true);
    assert!(recover(tab.clone(), pending.clone()).await.is_err());
    assert_eq!(pending.held.lock().unwrap().keys.len(), 1);
    assert!(!pending.held.lock().unwrap().mask);
    assert_eq!(*released.lock().unwrap(), vec!["A", "B", "mask"]);
    pending.wait(&tab).await.unwrap();
    assert!(pending.held.lock().unwrap().empty());
    assert_eq!(*released.lock().unwrap(), vec!["A", "B", "mask", "A"]);
    server.abort();
}
