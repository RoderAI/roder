//! One DevTools session on one tab.
//!
//! A [`DirectTab`] names the tab two ways. Through its browser: an http(s)
//! DevTools address or the browser websocket itself, and the tab's target
//! id; the client opens the browser websocket and attaches a flat session to
//! that target, which is how a browser another component started (Jev's)
//! is reached, and which lets the client see tabs the page opens. Or as a
//! page websocket opened directly, as Roder Desktop's integrated browser
//! lists them at `/json`.
//!
//! Every call answers the `Page.javascriptDialogOpening` events it reads
//! while it waits, by [`accepts_dialog`]'s rule, and keeps them for the
//! caller: a dialog blocks its renderer, so the next evaluate would
//! otherwise wait out its timeout. One call is in flight at a time, so no
//! reader task is needed.

use std::time::Duration;

use anyhow::{Context, bail};
use futures::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use super::cleanup::{Cleanup, Pending};
use super::devtools::{accepts_dialog, browser_websocket, decode_message};

const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// A dialog message longer than this is cut when it is read; it is page text.
const DIALOG_MESSAGE_CHARS: usize = 500;
/// What is kept of a message until it is read, so that the owner's secrets
/// are scrubbed from it before the cut: a secret across the cut would
/// otherwise show its first characters.
const DIALOG_KEPT_CHARS: usize = 4000;

/// The tab a [`super::DirectSession`] drives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectTab {
    /// A tab of the browser at `endpoint` (an http(s) DevTools address, or
    /// the browser's ws(s) websocket), by target id.
    Target { endpoint: String, target_id: String },
    /// A page websocket opened directly, and its target id.
    Page {
        websocket: String,
        target_id: String,
    },
}

impl DirectTab {
    pub fn target_id(&self) -> &str {
        match self {
            Self::Target { target_id, .. } | Self::Page { target_id, .. } => target_id,
        }
    }
}

/// A JavaScript dialog the page opened, and how the client answered it.
/// The message is page text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirectDialog {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    pub accepted: bool,
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub(crate) struct TabClient {
    tab: DirectTab,
    pending: Pending,
    socket: Socket,
    next_id: u64,
    /// The flat session on the target; `None` on a page websocket.
    session: Option<String>,
    target_id: String,
    /// Dialogs answered since the last [`TabClient::take_dialogs`].
    dialogs: Vec<DirectDialog>,
}

impl TabClient {
    /// Attach to `tab` and set it up to be driven: the Page domain (for
    /// dialogs) and focus emulation (a background tab keeps rendering).
    pub(crate) async fn attach(tab: &DirectTab) -> anyhow::Result<Self> {
        let (url, target) = match tab {
            DirectTab::Target {
                endpoint,
                target_id,
            } => (browser_websocket(endpoint).await?, Some(target_id.clone())),
            DirectTab::Page { websocket, .. } => (websocket.clone(), None),
        };
        // A host that compiles in both rustls backends must name one, or a
        // wss endpoint panics; an error only means one is already set.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (socket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .context("open the Chrome DevTools websocket")?;
        let mut client = Self {
            tab: tab.clone(),
            pending: Pending::default(),
            socket,
            next_id: 1,
            session: None,
            target_id: tab.target_id().to_string(),
            dialogs: Vec::new(),
        };
        if let Some(target) = target {
            client.attach_to(&target).await?;
        } else {
            client.set_up().await?;
        }
        Ok(client)
    }

    /// Move the session to another target of the same browser.
    pub(crate) async fn attach_to(&mut self, target: &str) -> anyhow::Result<()> {
        self.cleanup().finish().await?;
        let attached = self
            .browser_call(
                "Target.attachToTarget",
                json!({"targetId": target, "flatten": true}),
            )
            .await?;
        let session = attached["sessionId"]
            .as_str()
            .context("Chrome did not return a session")?
            .to_string();
        if let Some(old) = self.session.replace(session) {
            let _ = self
                .browser_call("Target.detachFromTarget", json!({"sessionId": old}))
                .await;
        }
        self.target_id = target.to_string();
        if let DirectTab::Target { target_id, .. } = &mut self.tab {
            *target_id = target.to_string();
        }
        self.set_up().await
    }

    async fn set_up(&mut self) -> anyhow::Result<()> {
        self.call("Page.enable", json!({})).await?;
        self.call(
            "Emulation.setFocusEmulationEnabled",
            json!({"enabled": true}),
        )
        .await?;
        Ok(())
    }

    /// Whether the client reaches the browser (and so other tabs).
    pub(crate) fn browser_level(&self) -> bool {
        self.session.is_some()
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    /// The dialogs answered since the last call, with `scrub` applied to each
    /// message before it is cut to [`DIALOG_MESSAGE_CHARS`].
    pub(crate) fn take_dialogs(&mut self, scrub: impl Fn(&str) -> String) -> Vec<DirectDialog> {
        released(std::mem::take(&mut self.dialogs), scrub)
    }

    /// A command to the tab.
    pub(crate) async fn call(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.pending.before(method, &params);
        let session = self.session.clone();
        let result = self
            .send_and_wait(method, params.clone(), session.as_deref())
            .await;
        if result.is_ok() {
            self.pending.after(method, &params);
        }
        result
    }

    pub(crate) fn cleanup(&self) -> Cleanup {
        Cleanup::new(self.tab.clone(), self.pending.clone())
    }

    pub(crate) async fn wait_cleanup(&self) -> anyhow::Result<()> {
        self.pending.wait(&self.tab).await
    }

    pub(crate) fn mask_pending(&self, armed: bool) {
        self.pending.mask(armed);
    }

    /// A command to the browser (the Target domain).
    pub(crate) async fn browser_call(
        &mut self,
        method: &str,
        params: Value,
    ) -> anyhow::Result<Value> {
        self.send_and_wait(method, params, None).await
    }

    /// `Runtime.evaluate` by value, awaiting a promise; a page-side
    /// exception is an error.
    pub(crate) async fn evaluate(&mut self, expression: &str) -> anyhow::Result<Value> {
        self.evaluate_context(expression, None).await
    }

    /// Keep ref maps and permission probes outside the page's JavaScript world.
    /// Re-resolve the world after navigation; the named world persists within a
    /// document across tool calls and page-websocket connections.
    pub(crate) async fn evaluate_isolated(&mut self, expression: &str) -> anyhow::Result<Value> {
        let tree = self.call("Page.getFrameTree", json!({})).await?;
        let frame = tree["frameTree"]["frame"]["id"]
            .as_str()
            .context("page frame id")?;
        let world = self
            .call(
                "Page.createIsolatedWorld",
                json!({
                    "frameId": frame, "worldName": "roder-direct-v2", "grantUniveralAccess": false
                }),
            )
            .await?;
        let context = world["executionContextId"]
            .as_u64()
            .context("isolated execution context")?;
        self.evaluate_context(expression, Some(context)).await
    }

    async fn evaluate_context(
        &mut self,
        expression: &str,
        context: Option<u64>,
    ) -> anyhow::Result<Value> {
        let mut params =
            json!({"expression": expression, "returnByValue": true, "awaitPromise": true});
        if let Some(context) = context {
            params["contextId"] = json!(context);
        }
        let result = self.call("Runtime.evaluate", params).await?;
        if let Some(exception) = result.get("exceptionDetails") {
            let detail = exception["exception"]["description"]
                .as_str()
                .or_else(|| exception["text"].as_str())
                .unwrap_or("an exception");
            bail!("the page script failed: {}", cut(detail, 200));
        }
        Ok(result["result"]["value"].clone())
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
        self.dialogs.push(DirectDialog {
            kind,
            message: cut(
                params["message"].as_str().unwrap_or_default(),
                DIALOG_KEPT_CHARS,
            ),
            accepted: accept,
        });
        Ok(())
    }

    async fn send_and_wait(
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
            let value = decode_message(&text)?;
            if value["method"] == "Page.javascriptDialogOpening" {
                // Only this client's own tab; another session's dialog is
                // its owner's to answer.
                let ours = self.session.is_none()
                    || value["sessionId"].as_str() == self.session.as_deref();
                if ours {
                    self.answer_dialog(&value).await?;
                }
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

impl Drop for TabClient {
    fn drop(&mut self) {
        drop(self.cleanup());
    }
}

/// `text` cut to `chars` characters.
pub(crate) fn cut(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// Dialogs as a reader may see them: scrubbed, then cut.
fn released(dialogs: Vec<DirectDialog>, scrub: impl Fn(&str) -> String) -> Vec<DirectDialog> {
    dialogs
        .into_iter()
        .map(|dialog| DirectDialog {
            message: cut(&scrub(&dialog.message), DIALOG_MESSAGE_CHARS),
            ..dialog
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dialog_message_is_scrubbed_before_it_is_cut() {
        // The secret straddles the 500-character cut: cut first, its first
        // characters would survive the scrub.
        let message = format!("{}hunter22 and more", "x".repeat(DIALOG_MESSAGE_CHARS - 4));
        assert!(message.contains("hunt"));
        let dialogs = vec![DirectDialog {
            kind: "alert".into(),
            message,
            accepted: true,
        }];
        let read = released(dialogs, |text| text.replace("hunter22", "[REDACTED]"));
        assert!(!read[0].message.contains("hunt"), "{}", read[0].message);
        assert!(read[0].message.contains("[RED"), "{}", read[0].message);
        assert!(read[0].message.chars().count() <= DIALOG_MESSAGE_CHARS + 1);
    }
}
