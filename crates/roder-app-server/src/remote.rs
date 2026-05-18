use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use roder_api::events::{
    RemoteAuthFailed, RemoteClientConnected, RemoteClientDisconnected, RemoteServerStarted,
    RoderEvent,
};
use roder_protocol::JsonRpcRequest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

use crate::AppServer;

pub const REMOTE_PROTOCOL: &str = "gode.remote.v1";
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
}

impl RemoteAuth {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            token_hash: Vec::new(),
            token_preview: String::new(),
        }
    }

    pub fn enabled(token: &RemoteToken) -> Self {
        Self {
            enabled: true,
            token_hash: token.hash.clone(),
            token_preview: token.preview.clone(),
        }
    }

    pub fn token_preview(&self) -> &str {
        &self.token_preview
    }

    pub fn verify_request(&self, request: &Request) -> bool {
        if !self.enabled {
            return true;
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
    pub print_qr: bool,
    pub workspace: Option<String>,
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
        "serverName": "Gode Remote",
        "workspace": options.workspace.clone(),
    });
    let pairing_url = pairing_deep_link(&payload)?;
    let handle = RemoteServerHandle {
        listen_addr,
        connect_urls,
        token_preview: options.token.preview().to_string(),
        pairing_url,
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
    let auth = Arc::new(RemoteAuth::enabled(&options.token));
    tokio::spawn(async move {
        loop {
            let Ok((stream, peer_addr)) = listener.accept().await else {
                break;
            };
            let auth = auth.clone();
            let app_server = app_server.clone();
            let remote_initialize_metadata = remote_initialize_metadata.clone();
            tokio::spawn(async move {
                let remote_addr = peer_addr.to_string();
                let auth_events = app_server.clone();
                let auth_remote_addr = remote_addr.clone();
                #[allow(clippy::result_large_err)]
                let callback = move |request: &Request,
                                     response: Response|
                      -> Result<Response, ErrorResponse> {
                    if auth.verify_request(request) {
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
                        Err(response)
                    }
                };
                let Ok(mut websocket) = tokio_tungstenite::accept_hdr_async(stream, callback).await
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
                while let Some(message) = websocket.next().await {
                    let Ok(Message::Text(text)) = message else {
                        continue;
                    };
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
                        let _ = websocket.send(Message::Text(text.into())).await;
                    }
                }
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
    });
    Ok(handle)
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
        name: "Gode Remote".to_string(),
        url,
        headers,
        subprotocols: vec![REMOTE_PROTOCOL.to_string(), format!("bearer.{token}")],
        workspace,
    }
}

pub fn pairing_deep_link(payload: &RemotePairingPayload) -> anyhow::Result<String> {
    let json = serde_json::to_vec(payload)?;
    Ok(format!(
        "gode://connect?payload={}",
        base64_url_no_pad(&json)
    ))
}

pub fn render_terminal_pairing(handle: &RemoteServerHandle) -> String {
    format!(
        "Remote app-server listening\nurls:\n{}\ntoken: {}\nconnect: {}\n",
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

pub fn parse_ws_listen(listen: &str) -> anyhow::Result<SocketAddr> {
    let rest = listen
        .strip_prefix("ws://")
        .ok_or_else(|| anyhow::anyhow!("remote listen must start with ws://"))?;
    let addr = rest
        .parse::<SocketAddr>()
        .map_err(|err| anyhow::anyhow!("invalid websocket listen address {listen:?}: {err}"))?;
    Ok(addr)
}

fn connect_urls(listen: &str, actual: SocketAddr) -> Vec<String> {
    let host = listen
        .strip_prefix("ws://")
        .and_then(|rest| rest.rsplit_once(':').map(|(host, _)| host))
        .unwrap_or("127.0.0.1");
    if host == "0.0.0.0" {
        vec![
            format!("ws://127.0.0.1:{}", actual.port()),
            format!("ws://{}:{}", local_private_fallback(), actual.port()),
        ]
    } else {
        vec![format!("ws://{}:{}", actual.ip(), actual.port())]
    }
}

fn local_private_fallback() -> Ipv4Addr {
    Ipv4Addr::new(192, 168, 0, 1)
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

#[allow(dead_code)]
fn is_tailscale(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            octets[0] == 100 && (64..=127).contains(&octets[1])
        }
        IpAddr::V6(_) => false,
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
    fn remote_auth_accepts_subprotocol_bearer() {
        let token = RemoteToken::new("secret-token".to_string()).unwrap();
        let auth = RemoteAuth::enabled(&token);
        let request = Request::builder()
            .uri("ws://127.0.0.1")
            .header(
                "Sec-WebSocket-Protocol",
                "gode.remote.v1, bearer.secret-token",
            )
            .body(())
            .unwrap();
        assert!(auth.verify_request(&request));
    }

    #[test]
    fn pairing_link_does_not_put_token_in_websocket_query() {
        let payload = pairing_payload("ws://127.0.0.1:1234".to_string(), "secret-token", None);
        assert_eq!(payload.url, "ws://127.0.0.1:1234");
        assert!(!payload.url.contains("secret-token"));
        let link = pairing_deep_link(&payload).unwrap();
        assert!(link.starts_with("gode://connect?payload="));
    }
}
