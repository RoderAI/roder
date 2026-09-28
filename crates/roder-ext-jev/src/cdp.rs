//! A minimal Chrome DevTools Protocol client.
//!
//! This replaces the Python `browser_harness` transport: one WebSocket to the
//! browser endpoint, flat sessions, request/response by id. Only the methods
//! Jev needs are used.
//!
//! A JavaScript dialog blocks its renderer, so a call that waits on that
//! renderer (an input event whose handler called `confirm()`, or the next
//! evaluate) would run into its 30 s timeout. Every call therefore answers
//! the `Page.javascriptDialogOpening` events it reads while waiting, for the
//! sessions that enabled the Page domain, and keeps going: `alert` and
//! `beforeunload` are accepted, `confirm` and `prompt` dismissed, and each
//! is kept for [`Connection::take_dialogs`]. The rule follows fastbrowse's
//! browser/session.py (MIT); jev never has two calls in flight, so no reader
//! task is needed.

use std::time::Duration;

use anyhow::{Context, bail};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use crate::engine::JevDialog;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// A dialog message longer than this is cut; it is page text, not a document.
const DIALOG_MESSAGE_CHARS: usize = 500;

pub(crate) struct Connection {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: u64,
    /// Dialogs answered since the last [`Connection::take_dialogs`].
    dialogs: Vec<JevDialog>,
}

impl Connection {
    /// Connect to a browser-level DevTools endpoint: a ws(s) URL is that
    /// websocket already, and an http(s) address advertises it at
    /// `/json/version`. Errors never repeat the URL, which may carry a token.
    pub(crate) async fn connect(endpoint: &str) -> anyhow::Result<Self> {
        let url = match is_websocket(endpoint) {
            true => endpoint.to_string(),
            false => advertised_websocket(endpoint).await?,
        };
        let (socket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .context("open the Chrome DevTools websocket")?;
        Ok(Self {
            socket,
            next_id: 1,
            dialogs: Vec::new(),
        })
    }

    /// The dialogs answered since the last call to this, oldest first.
    pub(crate) fn take_dialogs(&mut self) -> Vec<JevDialog> {
        std::mem::take(&mut self.dialogs)
    }

    fn send_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    async fn send(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut request = json!({"id": id, "method": method, "params": params});
        if let Some(session) = session {
            request["sessionId"] = json!(session);
        }
        self.socket
            .send(Message::Text(request.to_string().into()))
            .await
            .with_context(|| format!("send {method}"))
    }

    /// Answer a dialog the page opened, without waiting for the answer's
    /// reply (the id match skips it), and keep it.
    async fn answer_dialog(&mut self, event: &Value) -> anyhow::Result<()> {
        let params = &event["params"];
        let kind = params["type"].as_str().unwrap_or("alert").to_string();
        let accept = accepts(&kind);
        let id = self.send_id();
        let session = event["sessionId"].as_str().map(str::to_string);
        self.send(
            id,
            "Page.handleJavaScriptDialog",
            json!({"accept": accept}),
            session.as_deref(),
        )
        .await?;
        let message = params["message"].as_str().unwrap_or_default();
        self.dialogs.push(JevDialog {
            kind,
            message: message.chars().take(DIALOG_MESSAGE_CHARS).collect(),
            accepted: accept,
        });
        Ok(())
    }

    /// Issue a command and wait for the reply with the matching id, ignoring
    /// events and traffic for other sessions, and answering any dialog.
    pub(crate) async fn call(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> anyhow::Result<Value> {
        let id = self.send_id();
        self.send(id, method, params, session).await?;
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
            let value = decode(&text)?;
            if value["method"] == "Page.javascriptDialogOpening" {
                self.answer_dialog(&value).await?;
                continue;
            }
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

/// Parse one DevTools message. A string holding a lone UTF-16 surrogate
/// (half of an emoji a page script cut, or a page's own text) arrives as an
/// unpaired `\uD83D` escape, which serde_json rejects, and with it the whole
/// reply; such an escape is read as U+FFFD instead.
fn decode(text: &str) -> anyhow::Result<Value> {
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(error) => match repair_surrogates(text) {
            Some(repaired) => serde_json::from_str(&repaired),
            None => Err(error),
        }
        .context("decode a DevTools message"),
    }
}

/// `text` with every unpaired surrogate escape replaced by `\uFFFD`, or
/// `None` when it has none.
fn repair_surrogates(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let unit = |at: usize| -> Option<u16> {
        let digits = bytes.get(at..at + 6)?;
        (digits[0] == b'\\' && digits[1] == b'u')
            .then(|| std::str::from_utf8(&digits[2..]).ok())
            .flatten()
            .and_then(|hex| u16::from_str_radix(hex, 16).ok())
    };
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    let (mut copied, mut at) = (0, 0);
    while at < bytes.len() {
        if bytes[at] != b'\\' {
            at += 1;
            continue;
        }
        let Some(code) = unit(at) else {
            // Any other escape is two bytes; skip both so `\\u` is not read
            // as the start of a `\u` escape.
            at += 2;
            continue;
        };
        let paired = (0xD800..0xDC00).contains(&code)
            && unit(at + 6).is_some_and(|low| (0xDC00..0xE000).contains(&low));
        if paired {
            at += 12;
        } else if (0xD800..0xE000).contains(&code) {
            out.push_str(&text[copied..at]);
            out.push_str("\\uFFFD");
            changed = true;
            at += 6;
            copied = at;
        } else {
            at += 6;
        }
    }
    changed.then(|| {
        out.push_str(&text[copied..]);
        out
    })
}

/// An `alert` only informs and a `beforeunload` guards a navigation the
/// task asked for, so both go ahead. A `confirm` or `prompt` asks for
/// consent or input that the goal never gave, so both are declined.
fn accepts(kind: &str) -> bool {
    matches!(kind, "alert" | "beforeunload")
}

fn is_websocket(endpoint: &str) -> bool {
    let scheme = endpoint.split_once("://").map(|(scheme, _)| scheme);
    scheme.is_some_and(|scheme| {
        scheme.eq_ignore_ascii_case("ws") || scheme.eq_ignore_ascii_case("wss")
    })
}

/// `/json/version` under a DevTools HTTP address: appended to its path, as
/// segments, with its query (often a token) kept.
fn version_url(http_endpoint: &str) -> anyhow::Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(http_endpoint.trim())
        .ok()
        .filter(|url| !url.cannot_be_a_base())
        .context("the Chrome DevTools endpoint is not a valid URL")?;
    url.path_segments_mut()
        .map_err(|()| anyhow::anyhow!("the Chrome DevTools endpoint is not a valid URL"))?
        .pop_if_empty()
        .extend(["json", "version"]);
    Ok(url)
}

/// The browser websocket a DevTools HTTP address advertises.
async fn advertised_websocket(http_endpoint: &str) -> anyhow::Result<String> {
    let version: Value = reqwest::Client::new()
        .get(version_url(http_endpoint)?)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .context("reach the Chrome DevTools endpoint")?
        .json()
        .await
        .context("decode the Chrome DevTools version")?;
    version["webSocketDebuggerUrl"]
        .as_str()
        .map(str::to_string)
        .context("Chrome did not advertise a browser websocket")
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

    #[test]
    fn a_lone_surrogate_escape_does_not_lose_the_message() {
        let message = r#"{"id":3,"result":{"result":{"value":"ab\ud83d","pair":"\ud83d\ude00","low":"\udc00x","slash":"\\ud83d"}}}"#;
        assert!(serde_json::from_str::<Value>(message).is_err());
        let value = decode(message).unwrap();
        let result = &value["result"]["result"];
        assert_eq!(result["value"], "ab\u{fffd}");
        assert_eq!(result["pair"], "\u{1f600}");
        assert_eq!(result["low"], "\u{fffd}x");
        // An escaped backslash before "u" is text, not an escape.
        assert_eq!(result["slash"], "\\ud83d");
        assert_eq!(repair_surrogates(r#"{"a":"\ud83d\ude00 fine"}"#), None);
        assert!(decode("not json").is_err());
    }

    #[test]
    fn only_alerts_and_beforeunload_are_accepted() {
        assert!(accepts("alert"));
        assert!(accepts("beforeunload"));
        assert!(!accepts("confirm"));
        assert!(!accepts("prompt"));
        assert!(!accepts("anything else"));
    }

    #[test]
    fn the_version_lookup_keeps_the_endpoints_path_and_query() {
        let url = |endpoint: &str| version_url(endpoint).unwrap().to_string();
        assert_eq!(
            url("http://127.0.0.1:9222"),
            "http://127.0.0.1:9222/json/version"
        );
        assert_eq!(
            url("http://127.0.0.1:9222/"),
            "http://127.0.0.1:9222/json/version"
        );
        assert_eq!(
            url("https://chrome.example.com/devtools/?token=secret"),
            "https://chrome.example.com/devtools/json/version?token=secret"
        );
        assert_eq!(
            url("https://chrome.example.com/t/abc?token=s&x=1"),
            "https://chrome.example.com/t/abc/json/version?token=s&x=1"
        );
        let error = version_url("not a url ?token=secret")
            .unwrap_err()
            .to_string();
        assert!(!error.contains("secret"), "{error}");
    }

    /// A DevTools address with a path and a token is asked for its
    /// websocket at `<path>/json/version?<token>`.
    #[tokio::test]
    async fn an_endpoint_with_a_path_and_token_is_looked_up_under_that_path() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let read = stream.read(&mut request).await.unwrap();
            let line = String::from_utf8_lossy(&request[..read])
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            let (status, body) = match line.starts_with("GET /t/abc/json/version?token=s ") {
                // A websocket nothing listens on: the lookup itself worked.
                true => (
                    "200 OK",
                    r#"{"webSocketDebuggerUrl":"ws://127.0.0.1:1/devtools/browser/x"}"#,
                ),
                false => ("404 Not Found", "{}"),
            };
            let reply = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).await.unwrap();
            line
        });
        let error = match Connection::connect(&format!("http://{address}/t/abc?token=s")).await {
            Ok(_) => panic!("a websocket on port 1 answered"),
            Err(error) => format!("{error:#}"),
        };
        let line = server.await.unwrap();
        assert!(
            line.starts_with("GET /t/abc/json/version?token=s "),
            "{line}"
        );
        assert!(
            error.contains("open the Chrome DevTools websocket"),
            "{error}"
        );
    }

    #[test]
    fn websocket_endpoints_skip_the_version_lookup() {
        assert!(is_websocket("ws://127.0.0.1:9222/devtools/browser/abc"));
        assert!(is_websocket("WSS://browser.example.com/?token=t"));
        assert!(!is_websocket("http://127.0.0.1:9222"));
        assert!(!is_websocket("https://ws.example.com"));
    }

    #[tokio::test]
    async fn a_dead_websocket_is_reported_without_its_url() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "ws://{}/devtools/browser/x?token=secret",
            listener.local_addr().unwrap()
        );
        drop(listener);
        let error = match Connection::connect(&url).await {
            Ok(_) => panic!("a closed port answered as a websocket"),
            Err(error) => format!("{error:#}"),
        };
        assert!(error.contains("Chrome DevTools websocket"), "{error}");
        assert!(!error.contains("secret"), "{error}");
    }
}
