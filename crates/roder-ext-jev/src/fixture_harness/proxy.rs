//! A DevTools websocket proxy that holds back one command, so a test can
//! put a task's deadline in the middle of that call.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;

/// Forwards every websocket connection to one browser websocket, holding
/// each message that sends `method` back for `delay` first.
pub(crate) struct SlowProxy {
    /// The proxy's own browser websocket URL.
    pub(crate) url: String,
}

impl SlowProxy {
    pub(crate) async fn start(http_endpoint: &str, method: &'static str, delay: Duration) -> Self {
        let version: Value = reqwest::get(format!("{http_endpoint}/json/version"))
            .await
            .expect("reach the test Chrome")
            .json()
            .await
            .expect("read /json/version");
        let upstream = version["webSocketDebuggerUrl"]
            .as_str()
            .expect("a browser websocket")
            .to_string();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "ws://{}/devtools/browser/proxy",
            listener.local_addr().unwrap()
        );
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let upstream = upstream.clone();
                tokio::spawn(async move {
                    let Ok(client) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    let Ok((server, _)) = tokio_tungstenite::connect_async(upstream.as_str()).await
                    else {
                        return;
                    };
                    let (mut to_client, mut from_client) = client.split();
                    let (mut to_server, mut from_server) = server.split();
                    let outbound = async {
                        while let Some(Ok(message)) = from_client.next().await {
                            let held = message
                                .to_text()
                                .is_ok_and(|text| text.contains(&format!("\"{method}\"")));
                            if held {
                                tokio::time::sleep(delay).await;
                            }
                            if to_server.send(message).await.is_err() {
                                break;
                            }
                        }
                    };
                    let inbound = async {
                        while let Some(Ok(message)) = from_server.next().await {
                            if to_client.send(message).await.is_err() {
                                break;
                            }
                        }
                    };
                    tokio::select! {
                        () = outbound => {}
                        () = inbound => {}
                    }
                });
            }
        });
        Self { url }
    }
}
