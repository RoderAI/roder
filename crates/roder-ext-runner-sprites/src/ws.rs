//! WebSocket client paths for the Sprites exec and proxy channels.
//!
//! - Exec: `WSS /v1/sprites/{name}/exec` with repeated `cmd` query
//!   parameters; the server streams the same non-PTY stream-prefixed binary
//!   frames as the POST exec path plus JSON text frames for session info.
//! - Proxy: `WSS /v1/sprites/{name}/proxy` with an init JSON message naming
//!   the target host/port, then a raw TCP byte relay.
//!
//! Both paths authenticate with the same bearer token as the HTTP client
//! and are covered by offline fake-WSS tests; live use stays opt-in.

use std::net::SocketAddr;

use anyhow::Context;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message;

use crate::client::SpritesClient;
use crate::exec_ws::{ExecFrame, decode_non_tty_frame};

#[derive(Debug, Clone, Default)]
pub struct WsExecRequest {
    /// Argv, sent as repeated `cmd` query parameters.
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
}

/// Exec output plus any JSON text frames (session info, port notifications)
/// the server sent on the same stream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WsExecOutcome {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub info: Vec<serde_json::Value>,
}

impl SpritesClient {
    fn ws_url(&self, sprite_name: &str, path_and_query: &str) -> String {
        let url = format!(
            "{}/v1/sprites/{}{}",
            self.config.base_url,
            urlencoding::encode(sprite_name),
            path_and_query
        );
        if let Some(rest) = url.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = url.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            url
        }
    }

    fn ws_request(
        &self,
        url: &str,
    ) -> anyhow::Result<tokio_tungstenite::tungstenite::handshake::client::Request> {
        let mut request = url
            .into_client_request()
            .with_context(|| format!("build websocket request for {url}"))?;
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {}", self.config.token)
                .parse()
                .context("bearer header")?,
        );
        Ok(request)
    }

    /// Runs a command over the WSS exec channel and collects the multiplexed
    /// stdout/stderr/exit frames plus JSON info frames.
    pub async fn exec_ws(
        &self,
        sprite_name: &str,
        request: &WsExecRequest,
    ) -> anyhow::Result<WsExecOutcome> {
        anyhow::ensure!(!request.cmd.is_empty(), "ws exec requires a command");
        let mut query: Vec<(String, String)> = request
            .cmd
            .iter()
            .map(|cmd| ("cmd".to_string(), cmd.clone()))
            .collect();
        if let Some(cwd) = &request.cwd {
            query.push(("cwd".to_string(), cwd.clone()));
        }
        for (key, value) in &request.env {
            query.push(("env".to_string(), format!("{key}={value}")));
        }
        let query = query
            .iter()
            .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        let url = self.ws_url(sprite_name, &format!("/exec?{query}"));
        let (mut stream, _) = connect_async(self.ws_request(&url)?)
            .await
            .with_context(|| format!("connect sprites exec websocket for {sprite_name}"))?;

        let mut outcome = WsExecOutcome::default();
        while let Some(message) = stream.next().await {
            match message.context("sprites exec websocket frame")? {
                Message::Binary(frame) => match decode_non_tty_frame(&frame)? {
                    ExecFrame::Stdout(bytes) => {
                        outcome.stdout.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    ExecFrame::Stderr(bytes) => {
                        outcome.stderr.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    ExecFrame::Exit(code) => {
                        outcome.exit_code = Some(code);
                    }
                    ExecFrame::Stdin(_) | ExecFrame::StdinEof => {}
                },
                Message::Text(text) => {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                        outcome.info.push(value);
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
            if outcome.exit_code.is_some() {
                break;
            }
        }
        let _ = stream.close(None).await;
        Ok(outcome)
    }

    /**
     * Starts a local TCP listener that relays each connection to
     * `target_host:target_port` inside the sprite via the WSS proxy channel.
     * Returns the bound local address; the relay runs until the returned
     * task handle is aborted or dropped by the caller.
     */
    pub async fn serve_port_proxy(
        &self,
        sprite_name: &str,
        target_host: &str,
        target_port: u16,
    ) -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind local proxy listener")?;
        let local_addr = listener.local_addr()?;
        let url = self.ws_url(sprite_name, "/proxy");
        let init = serde_json::json!({ "host": target_host, "port": target_port }).to_string();
        let client = self.clone();

        let handle = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    break;
                };
                let Ok(request) = client.ws_request(&url) else {
                    break;
                };
                let init = init.clone();
                tokio::spawn(async move {
                    let Ok((mut ws, _)) = connect_async(request).await else {
                        return;
                    };
                    if ws.send(Message::Text(init.into())).await.is_err() {
                        return;
                    }
                    relay_tcp_over_ws(tcp, ws).await;
                });
            }
        });
        Ok((local_addr, handle))
    }
}

/// Bidirectional byte relay: local TCP bytes become WS binary frames and
/// vice versa, until either side closes.
async fn relay_tcp_over_ws<S>(
    mut tcp: tokio::net::TcpStream,
    mut ws: tokio_tungstenite::WebSocketStream<S>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        tokio::select! {
            read = tcp.read(&mut buffer) => {
                match read {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if ws
                            .send(Message::Binary(buffer[..count].to_vec().into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            frame = ws.next() => {
                match frame {
                    Some(Ok(Message::Binary(bytes))) => {
                        if tcp.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
    let _ = ws.close(None).await;
    let _ = tcp.shutdown().await;
}
