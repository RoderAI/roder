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

async fn cancellation_faults(tab: &DirectTab, registry: &ToolRegistry) {
    eval(registry, "window.heldKeys=[]; document.addEventListener('keydown',e=>heldKeys.push(e.key)); document.addEventListener('keyup',e=>{heldKeys=heldKeys.filter(k=>k!==e.key); window.releaseModifiers=[e.ctrlKey,e.altKey,e.metaKey,e.shiftKey]}); true").await;
    for (fault, name, args, restored) in [
        (
            Fault::Key,
            "key",
            json!({"key":"Control+ArrowDown"}),
            "heldKeys.length===0 && releaseModifiers.every(m=>!m)",
        ),
        (
            Fault::Screenshot,
            "screenshot",
            json!({}),
            "![...document.documentElement.children].some(e=>e.style.zIndex==='2147483647')",
        ),
        (
            Fault::Drag,
            "drag",
            json!({"from_x":0,"from_y":0,"to_x":100,"to_y":100}),
            "window.dragHeld===false",
        ),
    ] {
        let (routed, applied, relay) = relay(tab, fault).await;
        let mut session = DirectSession::attach(&routed, Arc::new(OpenGuard), false)
            .await
            .unwrap();
        if matches!(fault, Fault::Key) {
            // Cancel only the borrowed run future; its owner keeps the session.
            {
                let running = session.run(name, &args);
                tokio::pin!(running);
                tokio::select! {
                    _ = &mut running => panic!("keydown reply was not delayed"),
                    result = tokio::time::timeout(Duration::from_secs(5), applied) => result.unwrap().unwrap(),
                }
            }
            // A new operation must await recovery before driving this tab again.
            let resumed = session.run("look", &json!({})).await;
            assert!(!resumed.is_error, "{}", resumed.text);
            assert_eq!(eval(registry, restored).await, true);
            relay.abort();
            continue;
        }
        let task = tokio::spawn(async move {
            let step = session.run(name, &args).await;
            // Keep the session alive: error cleanup must happen before drop.
            (step, session)
        });
        tokio::time::timeout(Duration::from_secs(5), applied)
            .await
            .unwrap()
            .unwrap();
        match fault {
            Fault::Drag => {
                let (step, _session) = task.await.unwrap();
                assert!(step.is_error && step.text.contains("fixture drag failure"));
                assert_eq!(eval(registry, restored).await, true);
            }
            _ => {
                assert!(
                    !task.is_finished(),
                    "delayed reply did not hold the operation"
                );
                task.abort();
                assert!(matches!(task.await, Err(error) if error.is_cancelled()));
                let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
                while eval(registry, restored).await != true {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "cancelled {name} left browser state behind"
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
        relay.abort();
    }
}
