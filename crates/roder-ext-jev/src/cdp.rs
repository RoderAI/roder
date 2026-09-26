//! A minimal Chrome DevTools Protocol client.
//!
//! This replaces the Python `browser_harness` transport: one WebSocket to the
//! browser endpoint, flat sessions, request/response by id. Only the methods
//! Jev needs are used, and nothing here is specific to Jev.

use std::time::Duration;

use anyhow::{Context, bail};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct Connection {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: u64,
}

impl Connection {
    /// Connect to the browser-level endpoint of a DevTools HTTP address.
    pub(crate) async fn connect(http_endpoint: &str) -> anyhow::Result<Self> {
        let version: Value = reqwest::Client::new()
            .get(format!(
                "{}/json/version",
                http_endpoint.trim_end_matches('/')
            ))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .context("reach the Chrome DevTools endpoint")?
            .json()
            .await
            .context("decode the Chrome DevTools version")?;
        let url = version["webSocketDebuggerUrl"]
            .as_str()
            .context("Chrome did not advertise a browser websocket")?;
        let (socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .context("open the Chrome DevTools websocket")?;
        Ok(Self { socket, next_id: 1 })
    }

    /// Issue a command and wait for the reply with the matching id, ignoring
    /// events and traffic for other sessions.
    pub(crate) async fn call(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let mut request = json!({"id": id, "method": method, "params": params});
        if let Some(session) = session {
            request["sessionId"] = json!(session);
        }
        self.socket
            .send(Message::Text(request.to_string().into()))
            .await
            .with_context(|| format!("send {method}"))?;
        let deadline = tokio::time::Instant::now() + CALL_TIMEOUT;
        loop {
            let message = tokio::time::timeout_at(deadline, self.socket.next())
                .await
                .map_err(|_| anyhow::anyhow!("{method} timed out"))?
                .context("Chrome closed the DevTools connection")?
                .with_context(|| format!("read the reply to {method}"))?;
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).context("decode a DevTools message")?;
            if value["id"].as_u64() != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                let detail = error["message"].as_str().unwrap_or("unknown error");
                bail!("{method} failed: {detail}");
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_missing_endpoint_is_reported_not_panicked() {
        // Port 1 is never a DevTools endpoint.
        let error = match Connection::connect("http://127.0.0.1:1").await {
            Ok(_) => panic!("port 1 answered as a DevTools endpoint"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("Chrome DevTools endpoint"),
            "{error}"
        );
    }
}
