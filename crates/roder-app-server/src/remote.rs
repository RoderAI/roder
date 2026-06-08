mod network;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use futures::{SinkExt, StreamExt};
use qrcode::QrCode;
use qrcode::render::unicode;
use roder_api::events::{
    RemoteAuthFailed, RemoteClientConnected, RemoteClientDisconnected, RemoteServerStarted,
    RemoteServerStopped, RoderEvent,
};
use roder_protocol::JsonRpcRequest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode, header};

use crate::AppServer;
use network::connect_urls;

pub const REMOTE_PROTOCOL: &str = "roder.remote.v1";
const TOKEN_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteToken {
    token: String,
    hash: Vec<u8>,
    preview: String,
}

impl RemoteToken {
    pub fn new(token: String) -> anyhow::Result<Self> {
        if token.trim().is_empty() {
            anyhow::bail!("remote token cannot be empty");
        }
        let hash = Sha256::digest(token.as_bytes()).to_vec();
        let preview = token_preview(&token);
        Ok(Self {
            token,
            hash,
            preview,
        })
    }

    pub fn secret(&self) -> &str {
        &self.token
    }

    pub fn preview(&self) -> &str {
        &self.preview
    }
}

#[derive(Debug, Clone)]
pub struct RemoteAuth {
    enabled: bool,
    token_hash: Vec<u8>,
    token_preview: String,
    expires_at: Option<OffsetDateTime>,
}

impl RemoteAuth {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            token_hash: Vec::new(),
            token_preview: String::new(),
            expires_at: None,
        }
    }

    pub fn enabled(token: &RemoteToken) -> Self {
        Self::enabled_until(token, None)
    }

    pub fn enabled_until(token: &RemoteToken, expires_at: Option<OffsetDateTime>) -> Self {
        Self {
            enabled: true,
            token_hash: token.hash.clone(),
            token_preview: token.preview.clone(),
            expires_at,
        }
    }

    pub fn token_preview(&self) -> &str {
        &self.token_preview
    }

    pub fn verify_request(&self, request: &Request) -> bool {
        self.verify_request_at(request, OffsetDateTime::now_utc())
    }

    pub fn verify_request_at(&self, request: &Request, now: OffsetDateTime) -> bool {
        if !self.enabled {
            return true;
        }
        if self.expires_at.is_some_and(|expires_at| now >= expires_at) {
            return false;
        }
        let Some(token) = bearer_from_headers(request) else {
            return false;
        };
        let hash = Sha256::digest(token.as_bytes());
        constant_time_eq(&self.token_hash, hash.as_slice())
    }
}

#[derive(Debug, Clone)]
pub struct RemoteServerOptions {
    pub listen: String,
    pub token: RemoteToken,
    pub token_ttl: Option<time::Duration>,
    pub allowed_origins: Vec<String>,
    pub print_qr: bool,
    pub workspace: Option<String>,
}

#[derive(Debug, Default)]
struct RemoteAuthBackoff {
    failures: Mutex<HashMap<String, u32>>,
}

impl RemoteAuthBackoff {
    fn record_failure(&self, key: &str) -> Option<u64> {
        let mut failures = self.failures.lock().expect("remote auth backoff poisoned");
        let count = failures
            .entry(key.to_string())
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
        retry_after_seconds(*count)
    }

    fn reset(&self, key: &str) {
        self.failures
            .lock()
            .expect("remote auth backoff poisoned")
            .remove(key);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemotePairingPayload {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub url: String,
    pub headers: std::collections::BTreeMap<String, String>,
    pub subprotocols: Vec<String>,
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteServerHandle {
    pub listen_addr: SocketAddr,
    pub connect_urls: Vec<String>,
    pub token_preview: String,
    pub pairing_url: String,
    pub pair_url: String,
}

#[derive(Debug)]
pub struct RemoteServerController {
    handle: RemoteServerHandle,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl RemoteServerController {
    pub fn handle(&self) -> &RemoteServerHandle {
        &self.handle
    }

    pub async fn stop(mut self) -> anyhow::Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.await?;
        Ok(())
    }
}

pub fn generate_remote_token(mut rng: impl std::io::Read) -> anyhow::Result<RemoteToken> {
    let mut bytes = [0_u8; TOKEN_BYTES];
    rng.read_exact(&mut bytes)?;
    RemoteToken::new(base64_url_no_pad(&bytes))
}

pub fn generate_remote_token_from_os() -> anyhow::Result<RemoteToken> {
    let bytes: [u8; TOKEN_BYTES] = rand::random();
    RemoteToken::new(base64_url_no_pad(&bytes))
}

pub async fn listen_remote_websocket(
    app_server: Arc<AppServer>,
    options: RemoteServerOptions,
) -> anyhow::Result<RemoteServerHandle> {
    let (handle, task) = spawn_remote_websocket(app_server, options, None).await?;
    drop(task);
    Ok(handle)
}

pub async fn listen_remote_websocket_controller(
    app_server: Arc<AppServer>,
    options: RemoteServerOptions,
) -> anyhow::Result<RemoteServerController> {
    let (shutdown, shutdown_rx) = oneshot::channel();
    let (handle, task) = spawn_remote_websocket(app_server, options, Some(shutdown_rx)).await?;
    Ok(RemoteServerController {
        handle,
        shutdown: Some(shutdown),
        task,
    })
}

async fn spawn_remote_websocket(
    app_server: Arc<AppServer>,
    options: RemoteServerOptions,
    mut shutdown: Option<oneshot::Receiver<()>>,
) -> anyhow::Result<(RemoteServerHandle, JoinHandle<()>)> {
    let bind_addr = parse_ws_listen(&options.listen)?;
    let listener = TcpListener::bind(bind_addr).await?;
    let listen_addr = listener.local_addr()?;
    let connect_urls = connect_urls(&options.listen, listen_addr);
    let payload = pairing_payload(
        connect_urls
            .first()
            .cloned()
            .unwrap_or_else(|| format!("ws://{listen_addr}")),
        options.token.secret(),
        options.workspace.clone(),
    );
    let remote_initialize_metadata = serde_json::json!({
        "authenticated": true,
        "authSchemes": ["authorization_bearer", "websocket_subprotocol_bearer"],
        "serverName": "Roder Go",
        "workspace": options.workspace.clone(),
    });
    let pairing_url = pairing_deep_link(&payload)?;
    let pair_url = pairing_web_url(listen_addr, options.token.secret());
    let handle = RemoteServerHandle {
        listen_addr,
        connect_urls,
        token_preview: options.token.preview().to_string(),
        pairing_url,
        pair_url,
    };
    app_server
        .runtime
        .bus
        .emit(RoderEvent::RemoteServerStarted(RemoteServerStarted {
            listen_addr: listen_addr.to_string(),
            connect_urls: handle.connect_urls.clone(),
            token_preview: handle.token_preview.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }));
    let auth = Arc::new(RemoteAuth::enabled_until(
        &options.token,
        options.token_ttl.map(|ttl| OffsetDateTime::now_utc() + ttl),
    ));
    let auth_backoff = Arc::new(RemoteAuthBackoff::default());
    let stop_events = app_server.clone();
    let task = tokio::spawn(async move {
        loop {
            let accepted = if let Some(shutdown) = shutdown.as_mut() {
                tokio::select! {
                    _ = shutdown => break,
                    accepted = listener.accept() => accepted,
                }
            } else {
                listener.accept().await
            };
            let Ok((stream, peer_addr)) = accepted else {
                break;
            };
            let auth = auth.clone();
            let app_server = app_server.clone();
            let remote_initialize_metadata = remote_initialize_metadata.clone();
            let auth_backoff = auth_backoff.clone();
            tokio::spawn(async move {
                let mut stream = stream;
                if respond_to_health_probe(&mut stream).await {
                    return;
                }
                let remote_addr = peer_addr.to_string();
                let auth_events = app_server.clone();
                let auth_remote_addr = remote_addr.clone();
                #[allow(clippy::result_large_err)]
                let callback = move |request: &Request,
                                     mut response: Response|
                      -> Result<Response, ErrorResponse> {
                    if auth.verify_request(request) {
                        auth_backoff.reset(&auth_remote_addr);
                        if request_supports_remote_protocol(request) {
                            response.headers_mut().insert(
                                "Sec-WebSocket-Protocol",
                                HeaderValue::from_static(REMOTE_PROTOCOL),
                            );
                        }
                        Ok(response)
                    } else {
                        auth_events.runtime.bus.emit(RoderEvent::RemoteAuthFailed(
                            RemoteAuthFailed {
                                remote_addr: Some(auth_remote_addr.clone()),
                                timestamp: OffsetDateTime::now_utc(),
                            },
                        ));
                        let mut response = ErrorResponse::new(Some("unauthorized".to_string()));
                        *response.status_mut() = StatusCode::UNAUTHORIZED;
                        if let Some(retry_after) = auth_backoff.record_failure(&auth_remote_addr)
                            && let Ok(value) = HeaderValue::from_str(&retry_after.to_string())
                        {
                            response.headers_mut().insert(header::RETRY_AFTER, value);
                        }
                        Err(response)
                    }
                };
                let Ok(websocket) = tokio_tungstenite::accept_hdr_async(stream, callback).await
                else {
                    return;
                };
                app_server
                    .runtime
                    .bus
                    .emit(RoderEvent::RemoteClientConnected(RemoteClientConnected {
                        remote_addr: Some(remote_addr.clone()),
                        timestamp: OffsetDateTime::now_utc(),
                    }));

                // Split the socket so the connection can both answer JSON-RPC
                // requests AND push browser-bridge commands to a connected Chrome
                // extension. A writer task owns the sink; everything else enqueues
                // onto `outbound_tx`.
                let (mut ws_write, mut ws_read) = websocket.split();
                let (outbound_tx, mut outbound_rx) =
                    tokio::sync::mpsc::unbounded_channel::<Message>();
                let writer = tokio::spawn(async move {
                    while let Some(message) = outbound_rx.recv().await {
                        if ws_write.send(message).await.is_err() {
                            break;
                        }
                    }
                });

                let chrome_bridge = roder_api::chrome::bridge();
                let mut chrome_client: Option<u64> = None;

                while let Some(message) = ws_read.next().await {
                    let Ok(Message::Text(text)) = message else {
                        continue;
                    };

                    // Browser-bridge frames are tagged `{ "type": ... }` and are
                    // not JSON-RPC. Route them to the Chrome bridge; on the first
                    // `hello`, register the client and forward its command stream.
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                        && value.get("type").is_some()
                        && value.get("method").is_none()
                        && value.get("jsonrpc").is_none()
                    {
                        let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if kind == "hello" && chrome_client.is_none() {
                            let registration =
                                chrome_bridge.register_client(Some(remote_addr.clone()), &value);
                            chrome_client = Some(registration.client_id);
                            let command_tx = outbound_tx.clone();
                            let mut commands = registration.commands;
                            tokio::spawn(async move {
                                while let Some(frame) = commands.recv().await {
                                    let Ok(text) = serde_json::to_string(&frame) else {
                                        continue;
                                    };
                                    if command_tx.send(Message::Text(text.into())).is_err() {
                                        break;
                                    }
                                }
                            });
                        }
                        chrome_bridge.ingest_frame(chrome_client, value);
                        continue;
                    }

                    let response = match serde_json::from_str::<JsonRpcRequest>(&text) {
                        Ok(request) => {
                            let is_initialize = request.method == "initialize";
                            let mut response = app_server.handle_request(request).await;
                            if is_initialize
                                && let Some(result) = response.result.as_mut()
                                && let Some(object) = result.as_object_mut()
                            {
                                object.insert(
                                    "remote".to_string(),
                                    remote_initialize_metadata.clone(),
                                );
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

                if let Some(client_id) = chrome_client {
                    chrome_bridge.unregister_client(client_id);
                }
                drop(outbound_tx);
                let _ = writer.await;
                app_server
                    .runtime
                    .bus
                    .emit(RoderEvent::RemoteClientDisconnected(
                        RemoteClientDisconnected {
                            remote_addr: Some(remote_addr),
                            timestamp: OffsetDateTime::now_utc(),
                        },
                    ));
            });
        }
        stop_events
            .runtime
            .bus
            .emit(RoderEvent::RemoteServerStopped(RemoteServerStopped {
                listen_addr: listen_addr.to_string(),
                timestamp: OffsetDateTime::now_utc(),
            }));
    });
    Ok((handle, task))
}

const PAIR_PAGE_HTML: &str = "<!doctype html><meta charset=utf8><title>Roder pairing</title><body style=\"font-family:system-ui;padding:2rem\"><h2 id=\"roder-pair-status\">Pairing with Roder…</h2><p>You can close this tab once it connects.</p></body>";

async fn respond_to_health_probe(stream: &mut TcpStream) -> bool {
    let mut buffer = [0_u8; 512];
    let Ok(bytes_read) = stream.peek(&mut buffer).await else {
        return false;
    };
    if is_pair_request(&buffer[..bytes_read]) {
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            PAIR_PAGE_HTML.len(),
            PAIR_PAGE_HTML
        );
        let _ = stream.write_all(response.as_bytes()).await;
        return true;
    }
    if !is_health_probe(&buffer[..bytes_read]) {
        return false;
    }
    let response = b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 3\r\nconnection: close\r\n\r\nok\n";
    let _ = stream.write_all(response).await;
    true
}

fn is_health_probe(buffer: &[u8]) -> bool {
    buffer.starts_with(b"GET /readyz HTTP/1.1\r\n")
        || buffer.starts_with(b"GET /readyz HTTP/1.0\r\n")
        || buffer.starts_with(b"GET /healthz HTTP/1.1\r\n")
        || buffer.starts_with(b"GET /healthz HTTP/1.0\r\n")
}

fn is_pair_request(buffer: &[u8]) -> bool {
    buffer.starts_with(b"GET /pair")
}

pub fn pairing_payload(
    url: String,
    token: &str,
    workspace: Option<String>,
) -> RemotePairingPayload {
    let mut headers = std::collections::BTreeMap::new();
    headers.insert("Authorization".to_string(), format!("Bearer {token}"));
    RemotePairingPayload {
        kind: REMOTE_PROTOCOL.to_string(),
        name: "Roder Go".to_string(),
        url,
        headers,
        subprotocols: vec![REMOTE_PROTOCOL.to_string(), format!("bearer.{token}")],
        workspace,
    }
}

pub fn pairing_deep_link(payload: &RemotePairingPayload) -> anyhow::Result<String> {
    let json = serde_json::to_vec(payload)?;
    Ok(format!(
        "roder://connect?payload={}",
        base64_url_no_pad(&json)
    ))
}

/// Build the 1-click pairing web URL served by this app-server. The token lives
/// only in the URL fragment (after `#`), so the server itself never receives it;
/// the loaded Chrome extension reads `location.hash` and auto-configures.
pub fn pairing_web_url(listen_addr: SocketAddr, token: &str) -> String {
    let port = listen_addr.port();
    let endpoint = format!("ws://127.0.0.1:{port}");
    let payload = serde_json::json!({
        "endpoint": endpoint,
        "token": token,
    });
    let encoded = base64_url_no_pad(
        serde_json::to_string(&payload)
            .unwrap_or_default()
            .as_bytes(),
    );
    format!("http://127.0.0.1:{port}/pair#roder-pair={encoded}")
}

pub fn render_terminal_pairing(handle: &RemoteServerHandle) -> String {
    let qr = render_pairing_qr(&handle.pairing_url)
        .unwrap_or_else(|err| format!("QR unavailable: {err}"));
    format!(
        "Remote app-server listening\n\n{}\nurls:\n{}\ntoken: {}\nconnect: {}\n",
        qr,
        handle
            .connect_urls
            .iter()
            .map(|url| format!("  {url}"))
            .collect::<Vec<_>>()
            .join("\n"),
        handle.token_preview,
        handle.pairing_url
    )
}

pub fn render_pairing_qr(pairing_url: &str) -> anyhow::Result<String> {
    let code = QrCode::new(pairing_url.as_bytes())?;
    Ok(code
        .render::<unicode::Dense1x2>()
        .quiet_zone(true)
        .module_dimensions(2, 1)
        .build())
}

pub fn parse_ws_listen(listen: &str) -> anyhow::Result<SocketAddr> {
    let rest = listen
        .strip_prefix("ws://")
        .ok_or_else(|| anyhow::anyhow!("remote listen must start with ws://"))?;
    let addr = rest
        .parse::<SocketAddr>()
        .map_err(|err| anyhow::anyhow!("invalid websocket listen address {listen:?}: {err}"))?;
    Ok(addr)
}

fn bearer_from_headers(request: &Request) -> Option<String> {
    request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(ToString::to_string)
        .or_else(|| {
            request
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .find_map(|part| part.strip_prefix("bearer.").map(ToString::to_string))
                })
        })
}

fn request_supports_remote_protocol(request: &Request) -> bool {
    request
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|part| part == REMOTE_PROTOCOL)
        })
}

fn token_preview(token: &str) -> String {
    let prefix = token.chars().take(4).collect::<String>();
    let suffix = token
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{prefix}...{suffix}")
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (left, right) in left.iter().zip(right.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

fn base64_url_no_pad(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn retry_after_seconds(failure_count: u32) -> Option<u64> {
    match failure_count {
        0..=2 => None,
        count => Some(1_u64 << (count - 3).min(5)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::http::Request;

    #[test]
    fn remote_auth_accepts_header_and_rejects_wrong_token() {
        let token = RemoteToken::new("secret-token".to_string()).unwrap();
        let auth = RemoteAuth::enabled(&token);
        let ok = Request::builder()
            .uri("ws://127.0.0.1")
            .header("Authorization", "Bearer secret-token")
            .body(())
            .unwrap();
        let bad = Request::builder()
            .uri("ws://127.0.0.1")
            .header("Authorization", "Bearer wrong-token")
            .body(())
            .unwrap();
        assert!(auth.verify_request(&ok));
        assert!(!auth.verify_request(&bad));
        assert_eq!(auth.token_preview(), "secr...oken");
    }

    #[test]
    fn local_websocket_auth_disabled_accepts_missing_and_wrong_token() {
        let auth = RemoteAuth::disabled();
        let missing = Request::builder().uri("ws://127.0.0.1").body(()).unwrap();
        let wrong = Request::builder()
            .uri("ws://127.0.0.1")
            .header("Authorization", "Bearer wrong-token")
            .body(())
            .unwrap();

        assert!(auth.verify_request(&missing));
        assert!(auth.verify_request(&wrong));
        assert_eq!(auth.token_preview(), "");
    }

    #[test]
    fn remote_auth_accepts_subprotocol_bearer() {
        let token = RemoteToken::new("secret-token".to_string()).unwrap();
        let auth = RemoteAuth::enabled(&token);
        let request = Request::builder()
            .uri("ws://127.0.0.1")
            .header(
                "Sec-WebSocket-Protocol",
                "roder.remote.v1, bearer.secret-token",
            )
            .body(())
            .unwrap();
        assert!(auth.verify_request(&request));
        assert!(request_supports_remote_protocol(&request));
    }

    #[test]
    fn remote_auth_accepts_valid_bearer_from_any_origin() {
        let token = RemoteToken::new("secret-token".to_string()).unwrap();
        let auth = RemoteAuth::enabled(&token);
        let request = Request::builder()
            .uri("ws://127.0.0.1")
            .header("Origin", "app://obsidian.md")
            .header(
                "Sec-WebSocket-Protocol",
                "roder.remote.v1, bearer.secret-token",
            )
            .body(())
            .unwrap();

        assert!(auth.verify_request(&request));
        assert!(request_supports_remote_protocol(&request));
    }

    #[test]
    fn remote_auth_rejects_expired_token_with_fake_clock() {
        let token = RemoteToken::new("secret-token".to_string()).unwrap();
        let expires_at = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(60);
        let auth = RemoteAuth::enabled_until(&token, Some(expires_at));
        let request = Request::builder()
            .uri("ws://127.0.0.1")
            .header("Authorization", "Bearer secret-token")
            .body(())
            .unwrap();

        assert!(auth.verify_request_at(
            &request,
            OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(59)
        ));
        assert!(!auth.verify_request_at(
            &request,
            OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(60)
        ));
    }

    #[test]
    fn remote_auth_backoff_adds_retry_after_after_repeated_failures_and_resets() {
        let backoff = RemoteAuthBackoff::default();

        assert_eq!(backoff.record_failure("127.0.0.1:1234"), None);
        assert_eq!(backoff.record_failure("127.0.0.1:1234"), None);
        assert_eq!(backoff.record_failure("127.0.0.1:1234"), Some(1));
        assert_eq!(backoff.record_failure("127.0.0.1:1234"), Some(2));
        assert_eq!(backoff.record_failure("127.0.0.1:1234"), Some(4));

        backoff.reset("127.0.0.1:1234");
        assert_eq!(backoff.record_failure("127.0.0.1:1234"), None);
    }

    #[test]
    fn pairing_link_does_not_put_token_in_websocket_query() {
        let payload = pairing_payload("ws://127.0.0.1:1234".to_string(), "secret-token", None);
        assert_eq!(payload.url, "ws://127.0.0.1:1234");
        assert!(!payload.url.contains("secret-token"));
        let link = pairing_deep_link(&payload).unwrap();
        assert!(link.starts_with("roder://connect?payload="));
        assert_eq!(payload.kind, REMOTE_PROTOCOL);
        assert_eq!(payload.name, "Roder Go");
        assert_eq!(payload.subprotocols[0], REMOTE_PROTOCOL);
    }

    #[test]
    fn pairing_web_url_carries_decodable_endpoint_and_token_in_fragment() {
        use base64::Engine;

        let addr: SocketAddr = "127.0.0.1:4545".parse().unwrap();
        let url = pairing_web_url(addr, "secret-token");

        let prefix = "http://127.0.0.1:4545/pair#roder-pair=";
        assert!(url.starts_with(prefix), "unexpected url: {url}");

        let encoded = url.strip_prefix(prefix).unwrap();
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .expect("base64url payload");
        let value: serde_json::Value = serde_json::from_slice(&decoded).expect("json payload");
        assert_eq!(value["endpoint"], "ws://127.0.0.1:4545");
        assert_eq!(value["token"], "secret-token");
    }

    #[test]
    fn terminal_pairing_renders_qr_and_roder_link() {
        let handle = RemoteServerHandle {
            listen_addr: "127.0.0.1:1234".parse().unwrap(),
            connect_urls: vec!["ws://127.0.0.1:1234".to_string()],
            token_preview: "secr...oken".to_string(),
            pairing_url: "roder://connect?payload=test".to_string(),
            pair_url: "http://127.0.0.1:1234/pair#roder-pair=test".to_string(),
        };
        let rendered = render_terminal_pairing(&handle);
        assert!(rendered.contains("Remote app-server listening"));
        assert!(rendered.contains("roder://connect?payload=test"));
        assert!(rendered.contains("█"));
    }
}
