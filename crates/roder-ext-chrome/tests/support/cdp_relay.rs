// Delayed acknowledgements exercise cancellation after Chrome applied input.
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Copy)]
enum Fault {
    Key,
    Screenshot,
    Drag,
}

impl Fault {
    fn matches(self, request: &Value) -> bool {
        match self {
            Self::Key => {
                request["method"] == "Input.dispatchKeyEvent"
                    && request["params"]["type"] == "rawKeyDown"
            }
            Self::Screenshot => request["method"] == "Page.captureScreenshot",
            Self::Drag => {
                request["method"] == "Input.dispatchMouseEvent"
                    && request["params"]["type"] == "mouseMoved"
                    && request["params"]["buttons"] == 1
            }
        }
    }
}

async fn relay(
    tab: &DirectTab,
    fault: Fault,
) -> (
    DirectTab,
    tokio::sync::oneshot::Receiver<()>,
    tokio::task::JoinHandle<()>,
) {
    let DirectTab::Page {
        websocket,
        target_id,
    } = tab
    else {
        panic!("page websocket required")
    };
    let websocket = websocket.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let routed = DirectTab::Page {
        websocket: format!("ws://{}", listener.local_addr().unwrap()),
        target_id: target_id.clone(),
    };
    let (applied, wait) = tokio::sync::oneshot::channel();
    let signal = Arc::new(std::sync::Mutex::new(Some(applied)));
    let handle = tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let websocket = websocket.clone();
            let signal = signal.clone();
            tokio::spawn(async move {
                let mut peer = tokio_tungstenite::accept_async(tcp).await.unwrap();
                let (mut chrome, _) = tokio_tungstenite::connect_async(websocket).await.unwrap();
                let mut delayed = None;
                loop {
                    tokio::select! {
                        message = peer.next() => {
                            let Some(Ok(message)) = message else { break };
                            if let Message::Text(text) = &message {
                                let request: Value = serde_json::from_str(text).unwrap();
                                if signal.lock().unwrap().is_some() && fault.matches(&request) {
                                    delayed = request["id"].as_u64();
                                }
                            }
                            if chrome.send(message).await.is_err() { break }
                        }
                        message = chrome.next() => {
                            let Some(Ok(message)) = message else { break };
                            if let Message::Text(text) = &message {
                                let response: Value = serde_json::from_str(text).unwrap();
                                if delayed.is_some() && response["id"].as_u64() == delayed {
                                    assert!(response.get("error").is_none(), "fault must follow real successful input");
                                    if let Some(signal) = signal.lock().unwrap().take() { let _ = signal.send(()); }
                                    if matches!(fault, Fault::Drag) {
                                        let error = json!({"id":delayed.take(),"error":{"message":"fixture drag failure"}});
                                        if peer.send(Message::Text(error.to_string().into())).await.is_err() { break }
                                    }
                                    continue;
                                }
                            }
                            if peer.send(message).await.is_err() { break }
                        }
                    }
                }
            });
        }
    });
    (routed, wait, handle)
}
