//! `RemoteAppClient` (roadmap phase 67, Stage 2): an `AppClient` over the
//! secure agent-node WebSocket transport, so the TUI/CLI/SDK can drive a
//! remote Roder runtime exactly like a local one.
//!
//! - Requests are id-correlated (the client substitutes unique internal
//!   ids and restores the caller's id on the response) with bounded
//!   in-flight slots and timeouts.
//! - Notification frames and `node/event` envelope frames feed the same
//!   broadcast-based receivers `LocalAppClient` uses.
//! - On connection loss, pending requests fail with explicit JSON-RPC
//!   errors (mutating requests are never silently replayed) and the
//!   connection task reconnects with backoff; subscriptions resume on the
//!   new connection automatically because the node pushes everything over
//!   the control connection.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Context;
use futures::{SinkExt, StreamExt};
use roder_api::events::EventEnvelope;
use roder_protocol::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::agent_node::server::NODE_EVENT_METHOD;
use crate::agent_node::tls::{TlsIdentity, client_tls_config};
use crate::client::AppClient;

/// Maximum in-flight requests before callers receive an overload error.
const MAX_IN_FLIGHT: usize = 256;
/// Per-request timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Reconnect backoff bounds.
const RECONNECT_MIN: Duration = Duration::from_millis(250);
const RECONNECT_MAX: Duration = Duration::from_secs(10);

/// Connection settings for one remote node.
#[derive(Clone)]
pub struct RemoteNodeConnection {
    /// `host:port` of the agent node.
    pub address: String,
    /// Pinned node certificate fingerprint (hex sha256) from pairing.
    pub server_fingerprint: String,
    /// Controller identity presented for mTLS.
    pub controller_identity: TlsIdentity,
    /// Single-use pairing token for first-time enrollment; `None` once the
    /// controller certificate is enrolled.
    pub pairing_token: Option<String>,
}

impl std::fmt::Debug for RemoteNodeConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteNodeConnection")
            .field("address", &self.address)
            .field("server_fingerprint", &self.server_fingerprint)
            .field("pairing_token", &self.pairing_token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

struct Pending {
    original_id: Option<serde_json::Value>,
    tx: oneshot::Sender<JsonRpcResponse>,
}

struct Inner {
    connection: RemoteNodeConnection,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, Pending>>,
    outbound: Mutex<Option<mpsc::UnboundedSender<Message>>>,
    events: broadcast::Sender<EventEnvelope>,
    notifications: broadcast::Sender<JsonRpcNotification>,
}

#[derive(Clone)]
pub struct RemoteAppClient {
    inner: Arc<Inner>,
}

impl RemoteAppClient {
    /// Connects to the node and spawns the connection manager. Fails fast
    /// when the first connection cannot be established (bad address, TLS
    /// trust failure, rejected certificate/token).
    pub async fn connect(connection: RemoteNodeConnection) -> anyhow::Result<Self> {
        let (events, _) = broadcast::channel(1024);
        let (notifications, _) = broadcast::channel(1024);
        let inner = Arc::new(Inner {
            connection,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            outbound: Mutex::new(None),
            events,
            notifications,
        });

        // First connection is established eagerly so trust/auth failures
        // surface to the caller instead of a background retry loop.
        let socket = open_socket(&inner.connection).await?;
        spawn_connection_driver(inner.clone(), socket).await;
        tokio::spawn(reconnect_loop(inner.clone()));
        Ok(Self { inner })
    }

    pub async fn request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let original_id = request.id.clone();
        let internal_id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.inner.pending.lock().await;
            if pending.len() >= MAX_IN_FLIGHT {
                return error_response(
                    original_id,
                    -32000,
                    format!("remote node connection has {MAX_IN_FLIGHT} requests in flight"),
                );
            }
            pending.insert(
                internal_id,
                Pending {
                    original_id: original_id.clone(),
                    tx,
                },
            );
        }

        let mut wire_request = request;
        wire_request.id = Some(serde_json::json!(internal_id));
        let Ok(text) = serde_json::to_string(&wire_request) else {
            self.inner.pending.lock().await.remove(&internal_id);
            return error_response(original_id, -32700, "request serialization failed".into());
        };
        let sent = {
            let outbound = self.inner.outbound.lock().await;
            match outbound.as_ref() {
                Some(sender) => sender.send(Message::Text(text.into())).is_ok(),
                None => false,
            }
        };
        if !sent {
            self.inner.pending.lock().await.remove(&internal_id);
            return error_response(
                original_id,
                -32000,
                "remote node connection is offline; the request was not sent".into(),
            );
        }

        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => error_response(
                original_id,
                -32000,
                "remote node connection closed before answering; the request was not replayed"
                    .into(),
            ),
            Err(_) => {
                self.inner.pending.lock().await.remove(&internal_id);
                error_response(original_id, -32000, "remote node request timed out".into())
            }
        }
    }
}

#[async_trait::async_trait]
impl AppClient for RemoteAppClient {
    type EventReceiver = broadcast::Receiver<EventEnvelope>;
    type NotificationReceiver = broadcast::Receiver<JsonRpcNotification>;

    async fn send_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        RemoteAppClient::request(self, request).await
    }

    fn subscribe_events(&self) -> Self::EventReceiver {
        self.inner.events.subscribe()
    }

    fn subscribe_notifications(&self) -> Self::NotificationReceiver {
        self.inner.notifications.subscribe()
    }
}

type SecureSocket =
    tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>;

async fn open_socket(connection: &RemoteNodeConnection) -> anyhow::Result<SecureSocket> {
    let tls = client_tls_config(
        &connection.server_fingerprint,
        Some(&connection.controller_identity),
    )?;
    let connector = TlsConnector::from(Arc::new(tls));
    let tcp = TcpStream::connect(&connection.address)
        .await
        .with_context(|| format!("connect to agent node {}", connection.address))?;
    let server_name = rustls::pki_types::ServerName::try_from("localhost".to_string())?;
    let tls_stream = connector
        .connect(server_name, tcp)
        .await
        .context("TLS handshake with agent node failed (certificate trust)")?;

    let mut request = format!("wss://{}/control", connection.address)
        .into_client_request()
        .context("build agent node websocket request")?;
    if let Some(token) = &connection.pairing_token {
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {token}").parse().context("bearer header")?,
        );
    }
    let (socket, _) = tokio_tungstenite::client_async(request, tls_stream)
        .await
        .context("agent node rejected the connection (authorization)")?;
    Ok(socket)
}

async fn spawn_connection_driver(inner: Arc<Inner>, socket: SecureSocket) {
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Message>();
    // Install the outbound sender before spawning so a request issued
    // immediately after `connect()` returns can never observe the link as
    // offline while the driver task is still starting up.
    *inner.outbound.lock().await = Some(outbound_tx);
    {
        let inner = inner.clone();
        tokio::spawn(async move {
            let (mut write, mut read) = socket.split();
            let writer = tokio::spawn(async move {
                while let Some(message) = outbound_rx.recv().await {
                    if write.send(message).await.is_err() {
                        break;
                    }
                }
            });

            while let Some(message) = read.next().await {
                let Ok(Message::Text(text)) = message else {
                    continue;
                };
                route_frame(&inner, &text).await;
            }

            // Connection lost: fail every pending request explicitly.
            *inner.outbound.lock().await = None;
            writer.abort();
            let mut pending = inner.pending.lock().await;
            for (_, entry) in pending.drain() {
                let _ = entry.tx.send(error_response(
                    entry.original_id,
                    -32000,
                    "remote node connection lost; the request was not replayed".into(),
                ));
            }
        });
    }
}

async fn route_frame(inner: &Arc<Inner>, text: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    if value.get("id").is_some_and(|id| !id.is_null()) {
        // Response: correlate by internal id and restore the caller's id.
        let Ok(mut response) = serde_json::from_value::<JsonRpcResponse>(value) else {
            return;
        };
        let Some(internal_id) = response.id.as_ref().and_then(serde_json::Value::as_u64) else {
            return;
        };
        if let Some(entry) = inner.pending.lock().await.remove(&internal_id) {
            response.id = entry.original_id.clone();
            let _ = entry.tx.send(response);
        }
        return;
    }
    let Ok(notification) = serde_json::from_value::<JsonRpcNotification>(value) else {
        return;
    };
    if notification.method == NODE_EVENT_METHOD {
        if let Ok(envelope) = serde_json::from_value::<EventEnvelope>(notification.params) {
            let _ = inner.events.send(envelope);
        }
        return;
    }
    let _ = inner.notifications.send(notification);
}

/// Background reconnect: when the connection drops, retry with capped
/// exponential backoff. The pairing token is single-use, so reconnects rely
/// on the (now enrolled) controller certificate only.
async fn reconnect_loop(inner: Arc<Inner>) {
    let mut backoff = RECONNECT_MIN;
    loop {
        tokio::time::sleep(backoff).await;
        if Arc::strong_count(&inner) == 1 {
            // Client dropped; stop reconnecting.
            return;
        }
        let connected = inner.outbound.lock().await.is_some();
        if connected {
            backoff = RECONNECT_MIN;
            continue;
        }
        let mut connection = inner.connection.clone();
        connection.pairing_token = None;
        match open_socket(&connection).await {
            Ok(socket) => {
                spawn_connection_driver(inner.clone(), socket).await;
                backoff = RECONNECT_MIN;
            }
            Err(_) => {
                backoff = (backoff * 2).min(RECONNECT_MAX);
            }
        }
    }
}

fn error_response(
    id: Option<serde_json::Value>,
    code: i32,
    message: String,
) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message,
            data: None,
        }),
    }
}
