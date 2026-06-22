//! Encrypted agent-node control server (roadmap phase 67, Stages 1–2).
//!
//! Serves the canonical app-server JSON-RPC surface over `wss://` with
//! mTLS-pinned controller authorization. The wire stays JSON-RPC 2.0
//! request/response plus notification frames; runtime event envelopes are
//! pushed as `node/event` notifications so a remote controller can satisfy
//! the `AppClient` event/notification receivers over one authenticated
//! connection.
//!
//! Authorization per connection (all over TLS):
//! - presented client certificate with an enrolled fingerprint → control;
//! - unenrolled client certificate plus a valid single-use pairing token
//!   in the `Authorization` header → the fingerprint is enrolled and the
//!   connection is immediately trusted;
//! - anything else → rejected before any app-server request is handled.
//!
//! Tokens in query strings are always rejected.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use futures::{SinkExt, StreamExt};
use roder_api::events::{
    RemoteAuthFailed, RemoteClientConnected, RemoteClientDisconnected, RemoteServerStarted,
    RemoteServerStopped, RoderEvent,
};
use roder_protocol::{JsonRpcNotification, JsonRpcRequest};
use time::OffsetDateTime;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

use roder_app_server::AppServer;
use crate::agent_node::auth::{ControllerTrust, PairingTokens};
use crate::agent_node::tls::{TlsIdentity, fingerprint_der, generate_identity, server_tls_config};

/// Agent-node protocol identifier surfaced in `initialize` metadata.
pub const AGENT_NODE_PROTOCOL: &str = "roder.agent-node.v1";
/// Notification method used to push runtime event envelopes.
pub const NODE_EVENT_METHOD: &str = "node/event";

#[derive(Debug, Clone)]
pub struct AgentNodeOptions {
    /// `host:port` listen address (use port 0 for tests).
    pub listen: String,
    /// Operator-facing node name (also the certificate CN for generated
    /// identities).
    pub node_name: String,
    /// Identity + trust persistence dir (default `~/.roder/agent-node/`).
    pub state_dir: PathBuf,
    /// Remote workspace label surfaced in initialize metadata.
    pub workspace: Option<String>,
}

pub struct AgentNodeHandle {
    pub listen_addr: SocketAddr,
    pub node_id: String,
    pub fingerprint: String,
    pub cert_pem: String,
    pub trust: Arc<ControllerTrust>,
    pub tokens: Arc<PairingTokens>,
}

pub struct AgentNodeController {
    pub handle: AgentNodeHandle,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
    connections: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>>,
}

impl AgentNodeController {
    /// Stops the listener and closes every active controller connection.
    pub async fn stop(mut self) -> anyhow::Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.await?;
        for connection in self.connections.lock().unwrap().drain(..) {
            connection.abort();
        }
        Ok(())
    }
}

/// Loads or generates the persistent node identity under `state_dir`.
pub fn load_or_generate_identity(
    state_dir: &std::path::Path,
    node_name: &str,
) -> anyhow::Result<TlsIdentity> {
    std::fs::create_dir_all(state_dir)?;
    let cert_path = state_dir.join("node-cert.pem");
    let key_path = state_dir.join("node-key.pem");
    if cert_path.exists() && key_path.exists() {
        let cert_pem = std::fs::read_to_string(&cert_path)?;
        let key_pem = std::fs::read_to_string(&key_path)?;
        let fingerprint = crate::agent_node::tls::fingerprint_from_pem(&cert_pem)?;
        return Ok(TlsIdentity {
            cert_pem,
            key_pem,
            fingerprint,
        });
    }
    let identity = generate_identity(node_name)?;
    std::fs::write(&cert_path, &identity.cert_pem)?;
    std::fs::write(&key_path, &identity.key_pem)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(identity)
}

pub async fn serve_agent_node(
    app_server: Arc<AppServer>,
    options: AgentNodeOptions,
) -> anyhow::Result<AgentNodeController> {
    let identity = load_or_generate_identity(&options.state_dir, &options.node_name)?;
    let trust = Arc::new(ControllerTrust::open(&options.state_dir)?);
    let tokens = Arc::new(PairingTokens::default());
    let acceptor = TlsAcceptor::from(Arc::new(server_tls_config(&identity)?));

    let listener = TcpListener::bind(&options.listen)
        .await
        .with_context(|| format!("bind agent node listener on {}", options.listen))?;
    let listen_addr = listener.local_addr()?;
    let node_id = identity.fingerprint[..12].to_string();
    let handle = AgentNodeHandle {
        listen_addr,
        node_id: node_id.clone(),
        fingerprint: identity.fingerprint.clone(),
        cert_pem: identity.cert_pem.clone(),
        trust: trust.clone(),
        tokens: tokens.clone(),
    };

    app_server.set_node_identity(roder_protocol::agent_node::NodeIdentity {
        node_id: node_id.clone(),
        name: options.node_name.clone(),
        fingerprint: identity.fingerprint.clone(),
        auth_mode: None,
        protocol_version: AGENT_NODE_PROTOCOL.to_string(),
        workspace: options.workspace.clone(),
    });
    app_server
        .runtime
        .bus
        .emit(RoderEvent::RemoteServerStarted(RemoteServerStarted {
            listen_addr: listen_addr.to_string(),
            connect_urls: vec![format!("wss://{listen_addr}")],
            token_preview: format!("node:{node_id}"),
            timestamp: OffsetDateTime::now_utc(),
        }));

    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    let node_metadata = serde_json::json!({
        "nodeId": node_id,
        "name": options.node_name,
        "fingerprint": identity.fingerprint,
        "protocolVersion": AGENT_NODE_PROTOCOL,
        "workspace": options.workspace,
    });
    let stop_events = app_server.clone();
    let connections: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>> = Arc::default();
    let connection_registry = connections.clone();
    let task = tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((stream, peer_addr)) = accepted else {
                break;
            };
            let acceptor = acceptor.clone();
            let app_server = app_server.clone();
            let trust = trust.clone();
            let tokens = tokens.clone();
            let node_metadata = node_metadata.clone();
            let connection = tokio::spawn(async move {
                let Ok(tls_stream) = acceptor.accept(stream).await else {
                    return;
                };
                serve_connection(
                    app_server,
                    trust,
                    tokens,
                    node_metadata,
                    tls_stream,
                    peer_addr,
                )
                .await;
            });
            let mut registry = connection_registry.lock().unwrap();
            registry.retain(|handle| !handle.is_finished());
            registry.push(connection);
        }
        stop_events
            .runtime
            .bus
            .emit(RoderEvent::RemoteServerStopped(RemoteServerStopped {
                listen_addr: listen_addr.to_string(),
                timestamp: OffsetDateTime::now_utc(),
            }));
    });

    Ok(AgentNodeController {
        handle,
        shutdown: Some(shutdown_tx),
        task,
        connections,
    })
}

async fn serve_connection(
    app_server: Arc<AppServer>,
    trust: Arc<ControllerTrust>,
    tokens: Arc<PairingTokens>,
    node_metadata: serde_json::Value,
    tls_stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    peer_addr: SocketAddr,
) {
    let remote_addr = peer_addr.to_string();
    let client_fingerprint = tls_stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certs| certs.first())
        .map(fingerprint_der);

    // Authorize at WebSocket handshake time, before any request dispatch.
    let auth_events = app_server.clone();
    let auth_addr = remote_addr.clone();
    let auth_fingerprint = client_fingerprint.clone();
    let authorized_mode = Arc::new(std::sync::Mutex::new(None::<&'static str>));
    let callback_mode = authorized_mode.clone();
    #[allow(clippy::result_large_err)]
    let callback =
        move |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
            let deny = |reason: &str| {
                auth_events
                    .runtime
                    .bus
                    .emit(RoderEvent::RemoteAuthFailed(RemoteAuthFailed {
                        remote_addr: Some(auth_addr.clone()),
                        timestamp: OffsetDateTime::now_utc(),
                    }));
                let mut error = ErrorResponse::new(Some(reason.to_string()));
                *error.status_mut() = StatusCode::UNAUTHORIZED;
                error
            };
            // Tokens in query strings are forbidden, full stop.
            if request
                .uri()
                .query()
                .is_some_and(|query| query.to_ascii_lowercase().contains("token"))
            {
                return Err(deny("credentials in query parameters are not accepted"));
            }
            let Some(fingerprint) = auth_fingerprint.as_deref() else {
                return Err(deny("a controller client certificate is required"));
            };
            if trust.is_trusted(fingerprint) {
                *callback_mode.lock().unwrap() = Some("mtls");
                return Ok(response);
            }
            // Unenrolled certificate: a valid single-use pairing token
            // enrolls this connection's certificate fingerprint.
            let bearer = request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "));
            let Some(token) = bearer else {
                return Err(deny("controller certificate is not enrolled"));
            };
            if tokens.redeem(token).is_err() {
                return Err(deny("pairing token rejected"));
            }
            if trust.enroll(fingerprint, "paired-controller").is_err() {
                return Err(deny("controller enrollment rejected"));
            }
            *callback_mode.lock().unwrap() = Some("pairing-token-enrolled");
            Ok(response)
        };

    let Ok(websocket) = tokio_tungstenite::accept_hdr_async(tls_stream, callback).await else {
        return;
    };
    let auth_mode = authorized_mode.lock().unwrap().unwrap_or("mtls");

    app_server
        .runtime
        .bus
        .emit(RoderEvent::RemoteClientConnected(RemoteClientConnected {
            remote_addr: Some(remote_addr.clone()),
            timestamp: OffsetDateTime::now_utc(),
        }));

    let (mut ws_write, mut ws_read) = websocket.split();
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    // All connection subtasks live in a JoinSet so dropping/aborting the
    // connection tears the socket down deterministically.
    let mut subtasks = tokio::task::JoinSet::new();
    subtasks.spawn(async move {
        while let Some(message) = outbound_rx.recv().await {
            if ws_write.send(message).await.is_err() {
                break;
            }
        }
    });

    // Push protocol notifications and runtime event envelopes over the
    // same authenticated connection.
    let mut notifications = app_server.subscribe_notifications();
    let notification_tx = outbound_tx.clone();
    subtasks.spawn(async move {
        while let Ok(notification) = notifications.recv().await {
            let Ok(text) = serde_json::to_string(&notification) else {
                continue;
            };
            if notification_tx.send(Message::Text(text.into())).is_err() {
                break;
            }
        }
    });
    let mut events = app_server.subscribe_events();
    let event_tx = outbound_tx.clone();
    subtasks.spawn(async move {
        while let Ok(envelope) = events.recv().await {
            let Ok(params) = serde_json::to_value(&envelope) else {
                continue;
            };
            let frame = JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: NODE_EVENT_METHOD.to_string(),
                params,
            };
            let Ok(text) = serde_json::to_string(&frame) else {
                continue;
            };
            if event_tx.send(Message::Text(text.into())).is_err() {
                break;
            }
        }
    });

    let mut node_metadata = node_metadata;
    if let Some(object) = node_metadata.as_object_mut() {
        object.insert("authMode".to_string(), serde_json::json!(auth_mode));
    }
    while let Some(message) = ws_read.next().await {
        let Ok(Message::Text(text)) = message else {
            continue;
        };
        let response = match serde_json::from_str::<JsonRpcRequest>(&text) {
            Ok(request) => {
                let method = request.method.clone();
                let mut response = app_server.handle_request(request).await;
                if method == "initialize"
                    && let Some(result) = response.result.as_mut()
                    && let Some(object) = result.as_object_mut()
                {
                    object.insert("node".to_string(), node_metadata.clone());
                }
                // node/status carries the per-connection auth mode.
                if method == "node/status"
                    && let Some(result) = response.result.as_mut()
                    && let Some(node) = result.get_mut("node").and_then(|n| n.as_object_mut())
                {
                    node.insert("authMode".to_string(), serde_json::json!(auth_mode));
                }
                response
            }
            Err(err) => roder_protocol::JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: None,
                result: None,
                error: Some(roder_protocol::JsonRpcError {
                    code: -32700,
                    message: format!("Parse error: {err}"),
                    data: None,
                }),
            },
        };
        if let Ok(text) = serde_json::to_string(&response) {
            let _ = outbound_tx.send(Message::Text(text.into()));
        }
    }

    drop(outbound_tx);
    subtasks.abort_all();
    while subtasks.join_next().await.is_some() {}
    app_server
        .runtime
        .bus
        .emit(RoderEvent::RemoteClientDisconnected(
            RemoteClientDisconnected {
                remote_addr: Some(remote_addr),
                timestamp: OffsetDateTime::now_utc(),
            },
        ));
}
