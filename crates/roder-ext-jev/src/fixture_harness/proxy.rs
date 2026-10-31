//! DevTools websocket proxies: one that holds back one command, so a test
//! can put a task's deadline in the middle of that call, and one that counts
//! the bytes each way, so a test can say what a read costs on the wire.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
        let url = forward(http_endpoint, Some((method, delay)), Arc::default()).await;
        Self { url }
    }
}

/// Forwards every websocket connection to one browser websocket, counting
/// the bytes of the messages sent to Chrome and received from it. Scripts
/// registered for every new document are counted apart: a tab pays for them
/// once, not per read.
pub(crate) struct CountingProxy {
    /// The proxy's own browser websocket URL.
    pub(crate) url: String,
    bytes: Arc<Bytes>,
}

#[derive(Default)]
struct Bytes {
    sent: AtomicU64,
    received: AtomicU64,
    registered: AtomicU64,
}

impl CountingProxy {
    pub(crate) async fn start(http_endpoint: &str) -> Self {
        let bytes = Arc::<Bytes>::default();
        let url = forward(http_endpoint, None, bytes.clone()).await;
        Self { url, bytes }
    }

    /// Bytes sent to Chrome and received from it since the last call.
    pub(crate) fn take(&self) -> (u64, u64) {
        (
            self.bytes.sent.swap(0, Ordering::SeqCst),
            self.bytes.received.swap(0, Ordering::SeqCst),
        )
    }

    /// Bytes of scripts registered for every new document since the last
    /// call.
    pub(crate) fn take_registered(&self) -> u64 {
        self.bytes.registered.swap(0, Ordering::SeqCst)
    }
}

/// Start forwarding, and return the proxy's browser websocket URL.
async fn forward(
    http_endpoint: &str,
    hold: Option<(&'static str, Duration)>,
    bytes: Arc<Bytes>,
) -> String {
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
            let bytes = bytes.clone();
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
                        let registers = message.to_text().is_ok_and(|text| {
                            text.contains("\"Page.addScriptToEvaluateOnNewDocument\"")
                        });
                        match registers {
                            true => &bytes.registered,
                            false => &bytes.sent,
                        }
                        .fetch_add(message.len() as u64, Ordering::SeqCst);
                        let held = hold.filter(|(method, _)| {
                            message
                                .to_text()
                                .is_ok_and(|text| text.contains(&format!("\"{method}\"")))
                        });
                        if let Some((_, delay)) = held {
                            tokio::time::sleep(delay).await;
                        }
                        if to_server.send(message).await.is_err() {
                            break;
                        }
                    }
                };
                let inbound = async {
                    while let Some(Ok(message)) = from_server.next().await {
                        bytes
                            .received
                            .fetch_add(message.len() as u64, Ordering::SeqCst);
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
    url
}
