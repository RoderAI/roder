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
//! task is needed. Finding the browser websocket, decoding a message and the
//! dialog rule are shared with Roder's direct CDP tools
//! (`roder_ext_chrome::direct::devtools`), which Jev's fallback drives.
//!
//! The same reads keep the [`requests`] ledger from the Network events of
//! the sessions that enabled that domain, and every `Input.*` call marks it,
//! so the settle can wait for a fetch or XHR request the action started
//! ([`Connection::await_requests`]). The [`navigation`] ledger keeps, from
//! the same reads, each tab's committed document status, the document
//! requests and the downloads an action started. The [`frames`] ledger keeps
//! the sessions Chrome attached to each tab's frames of another process.

mod frames;
mod navigation;
mod requests;

use std::time::Duration;

use anyhow::{Context, bail};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use roder_ext_chrome::direct::devtools::{accepts_dialog, browser_websocket, decode_message};

use crate::engine::JevDialog;
pub(crate) use frames::FrameSession;
use frames::Frames;
use navigation::Navigation;
pub(crate) use navigation::{DownloadEvent, DownloadState, NavigationMark};
use requests::Requests;

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
    /// The fetch and XHR requests the pages have open.
    requests: Requests,
    /// Documents, their statuses and the downloads the pages started.
    navigation: Navigation,
    /// The frames of another process each tab has.
    frames: Frames,
}

impl Connection {
    /// Connect to a browser-level DevTools endpoint: a ws(s) URL is that
    /// websocket already, and an http(s) address advertises it at
    /// `/json/version`. Errors never repeat the URL, which may carry a token.
    pub(crate) async fn connect(endpoint: &str) -> anyhow::Result<Self> {
        let url = browser_websocket(endpoint).await?;
        // A host that compiles in both rustls backends must name one, or a
        // wss endpoint panics; an error only means one is already set.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (socket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .context("open the Chrome DevTools websocket")?;
        Ok(Self {
            socket,
            next_id: 1,
            dialogs: Vec::new(),
            requests: Requests::default(),
            navigation: Navigation::default(),
            frames: Frames::default(),
        })
    }

    /// The dialogs answered since the last call to this, oldest first.
    pub(crate) fn take_dialogs(&mut self) -> Vec<JevDialog> {
        std::mem::take(&mut self.dialogs)
    }

    /// The request mark the inputs sent since the last call to this took,
    /// if any were sent.
    pub(crate) fn take_input_mark(&mut self) -> Option<u64> {
        self.requests.take_mark()
    }

    /// Whether a fetch or XHR request `session`'s page started after `mark`
    /// is still waiting for its response.
    pub(crate) fn requests_open_since(&self, session: &str, mark: u64) -> bool {
        self.requests.open_since(session, mark)
    }

    /// Read events, answering dialogs, until no request `session`'s page
    /// started after `mark` is open. False when `deadline` came first.
    pub(crate) async fn await_requests(
        &mut self,
        session: &str,
        mark: u64,
        deadline: tokio::time::Instant,
    ) -> anyhow::Result<bool> {
        while self.requests.open_since(session, mark) {
            if self.next_message("a response", deadline).await?.is_none() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The navigation mark the inputs sent since the last call to this
    /// took, if any were sent.
    pub(crate) fn take_navigation_mark(&mut self) -> Option<NavigationMark> {
        self.navigation.take_mark()
    }

    /// The downloads the browser announced since `mark`, oldest first.
    pub(crate) fn downloads_since(&self, mark: NavigationMark) -> &[DownloadEvent] {
        self.navigation.downloads_since(mark)
    }

    /// Whether a document request `session`'s page started after `mark` is
    /// still unresolved: a navigation on its way, or one turning into a
    /// download.
    pub(crate) fn documents_open_since(&self, session: &str, mark: NavigationMark) -> bool {
        self.navigation.documents_open_since(session, mark)
    }

    /// The HTTP status `session`'s committed document was served with.
    pub(crate) fn document_status(&self, session: &str) -> Option<u16> {
        self.navigation.document_status(session)
    }

    /// Read events, answering dialogs, until `done` holds for what the
    /// ledger knows of documents and downloads. False when `deadline` came
    /// first.
    pub(crate) async fn await_navigation(
        &mut self,
        deadline: tokio::time::Instant,
        mut done: impl FnMut(&Self) -> bool,
    ) -> anyhow::Result<bool> {
        while !done(self) {
            if self.next_message("a download", deadline).await?.is_none() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The frames of another process Chrome attached under the tab
    /// `session`, oldest first.
    pub(crate) fn frame_sessions(&self, session: &str) -> Vec<FrameSession> {
        self.frames.of(session)
    }

    /// Note that a read of the frame on `session` ran past its deadline at
    /// `at`, or (`None`) that one answered.
    pub(crate) fn set_frame_missed(&mut self, session: &str, at: Option<std::time::Instant>) {
        self.frames.set_missed(session, at);
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
        let accept = accepts_dialog(&kind);
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
        self.call_within(method, params, session, CALL_TIMEOUT)
            .await
    }

    /// [`Connection::call`] with its own deadline, for a read that must not
    /// wait out the usual one (a frame whose renderer stopped answering).
    /// A reply that comes later is skipped like any other id's.
    pub(crate) async fn call_within(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
        within: Duration,
    ) -> anyhow::Result<Value> {
        let id = self.send_id();
        if method.starts_with("Input.") {
            self.requests.input_sent();
            self.navigation.input_sent();
        }
        self.send(id, method, params, session).await?;
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let value = self
                .next_message(method, deadline)
                .await?
                .ok_or_else(|| anyhow::anyhow!("{method} timed out"))?;
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

    /// The next message, an event already answered (a dialog) or noted (a
    /// request), or `None` once `deadline` passes; `what` names the read in
    /// errors. Only the read itself is bounded, so a dialog's answer is
    /// never cut off half sent.
    async fn next_message(
        &mut self,
        what: &str,
        deadline: tokio::time::Instant,
    ) -> anyhow::Result<Option<Value>> {
        loop {
            let Ok(message) = tokio::time::timeout_at(deadline, self.socket.next()).await else {
                return Ok(None);
            };
            let message = message
                .context("Chrome closed the DevTools connection")?
                .with_context(|| format!("read the reply to {what}"))?;
            let Message::Text(text) = message else {
                continue;
            };
            let value = decode_message(&text)?;
            if value["method"] == "Page.javascriptDialogOpening" {
                self.answer_dialog(&value).await?;
            } else if value.get("method").is_some() {
                self.requests.on_event(&value);
                self.navigation.on_event(&value);
                self.frames.on_event(&value);
            }
            return Ok(Some(value));
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
