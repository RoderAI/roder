//! A test's own DevTools endpoint on the shared Chrome, which puts everything
//! the test opens in a browser context of its own.
//!
//! Every test shares one Chrome, and two things about one browser are not
//! shared safely. First, `Target.getTargets` lists the tabs of every test
//! running at that moment, and the tests assert that no tab leaks (that a
//! failed load, a closed session or a deadline left none behind), which a
//! tab on `about:blank`, or one that went to another origin, cannot be told
//! from another test's by looking at it. Second, a window has one active tab
//! and one focus. Jev brings the tab it works in to the front, and the keys
//! a page handles depend on that: with every test's tabs in one window,
//! tests that press keys (a Tab to move focus, then Enter or typed text)
//! failed intermittently when many ran at once.
//!
//! A browser context is a window of its own, with storage of its own. So a
//! test is handed this relay as its Chrome's address. It speaks the part of
//! DevTools' HTTP and websocket interface that Jev and the tests use,
//! forwards every message unchanged, and adds the test's context to each
//! `Target.createTarget` request. The tabs the test owns are the pages in
//! that context, popups included, and the end of the test disposes of the
//! context, closing whatever it left open, as the end of a private Chrome
//! used to.
//!
//! A relay with no context ([`TabRelay::start`] with `shared` false) is for a
//! Chrome that only one test uses: it forwards everything as it is, and the
//! test owns every page.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use super::chrome_process::{ChromeProcess, blocking_call};
use crate::cdp::Connection;

/// The longest request head read before the request is taken for a
/// websocket handshake.
const HEAD_LIMIT: usize = 8192;

pub(super) struct TabRelay {
    url: String,
    chrome: Arc<ChromeProcess>,
    /// The test's browser context, when the Chrome is shared.
    context: Option<String>,
    accept: JoinHandle<()>,
}

/// What each connection to the relay needs.
struct Shared {
    /// Chrome's own browser websocket. The Chrome itself is not held, so the
    /// tasks serving the relay cannot keep a Chrome of one test's own running.
    upstream: String,
    context: Option<String>,
    /// The websocket the relay advertises for itself.
    websocket: String,
}

impl TabRelay {
    /// Listen on a free loopback port of the calling test's runtime. With
    /// `shared`, other tests use the Chrome too, and the test gets a browser
    /// context of its own on it.
    pub(super) async fn start(chrome: Arc<ChromeProcess>, shared: bool) -> anyhow::Result<Self> {
        let context = if shared {
            let mut connection = Connection::connect(&chrome.http()).await?;
            let created = connection
                .call("Target.createBrowserContext", json!({}), None)
                .await
                .context("create the test's browser context")?;
            Some(
                created["browserContextId"]
                    .as_str()
                    .context("Chrome did not name the browser context")?
                    .to_string(),
            )
        } else {
            None
        };
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("listen for the test's DevTools connections")?;
        let port = listener.local_addr()?.port();
        let relayed = Arc::new(Shared {
            upstream: chrome.websocket(),
            context: context.clone(),
            websocket: format!("ws://127.0.0.1:{port}/devtools/browser/relay"),
        });
        let accept = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, relayed.clone()));
            }
        });
        Ok(Self {
            url: format!("http://127.0.0.1:{port}"),
            chrome,
            context,
            accept,
        })
    }

    /// The address to give the test as its Chrome's DevTools endpoint.
    pub(super) fn url(&self) -> &str {
        &self.url
    }

    /// The page targets the test owns that Chrome lists now.
    pub(super) async fn owned_pages(&self) -> anyhow::Result<Vec<Value>> {
        // Straight to Chrome: this is not a tab the test made.
        let mut connection = Connection::connect(&self.chrome.http()).await?;
        let listed = connection
            .call("Target.getTargets", json!({}), None)
            .await?;
        Ok(pages_in(&listed, self.context.as_deref()))
    }
}

impl Drop for TabRelay {
    fn drop(&mut self) {
        self.accept.abort();
        let Some(context) = &self.context else {
            return;
        };
        // Disposing of a context closes its tabs.
        if let Some(mut socket) = self.chrome.blocking_socket() {
            blocking_call(
                &mut socket,
                1,
                "Target.disposeBrowserContext",
                &json!({"browserContextId": context}),
            );
        }
    }
}

/// The page targets in `listed` (a `Target.getTargets` result) that are in
/// `context`, or every one when there is none.
fn pages_in(listed: &Value, context: Option<&str>) -> Vec<Value> {
    listed["targetInfos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|target| target["type"] == "page")
        .filter(|target| {
            context.is_none_or(|context| target["browserContextId"].as_str() == Some(context))
        })
        .cloned()
        .collect()
}

/// One connection: `GET /json/version` is answered here, a websocket
/// handshake is relayed to Chrome.
async fn serve(stream: TcpStream, shared: Arc<Shared>) {
    match request_head(&stream).await {
        Some(head) if head.starts_with(b"GET /json/version") => {
            let body = json!({
                "Browser": "Chrome (shared by the Jev fixture tests)",
                "Protocol-Version": "1.3",
                "webSocketDebuggerUrl": shared.websocket,
            });
            respond(stream, head.len(), "200 OK", &body.to_string()).await;
        }
        Some(head) if head.starts_with(b"GET /json") => {
            respond(stream, head.len(), "404 Not Found", "{}").await;
        }
        _ => relay(stream, shared).await,
    }
}

/// The request's head, up to and including its blank line, left unread on
/// the socket.
async fn request_head(stream: &TcpStream) -> Option<Vec<u8>> {
    let mut buffer = [0u8; HEAD_LIMIT];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let read = tokio::time::timeout_at(deadline, stream.peek(&mut buffer))
            .await
            .ok()?
            .ok()?;
        if let Some(end) = buffer[..read].windows(4).position(|w| w == b"\r\n\r\n") {
            return Some(buffer[..end + 4].to_vec());
        }
        if read == 0 || read == buffer.len() || tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Read the `head_len` bytes of the request and answer it with `body`.
async fn respond(mut stream: TcpStream, head_len: usize, status: &str, body: &str) {
    let mut head = vec![0u8; head_len];
    if stream.read_exact(&mut head).await.is_err() {
        return;
    }
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=UTF-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(reply.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Forward every message both ways until either side closes, with the
/// test's context added to the requests that create a tab.
async fn relay(stream: TcpStream, shared: Arc<Shared>) {
    let Ok(client) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    let Ok((server, _)) = tokio_tungstenite::connect_async(shared.upstream.as_str()).await else {
        return;
    };
    let (mut to_client, mut from_client) = client.split();
    let (mut to_server, mut from_server) = server.split();
    let outbound = async {
        while let Some(Ok(message)) = from_client.next().await {
            let message = match &shared.context {
                Some(context) => in_context(message, context),
                None => message,
            };
            if to_server.send(message).await.is_err() {
                break;
            }
        }
    };
    let inbound = async {
        while let Some(Ok(message)) = from_server.next().await {
            if to_client.send(message).await.is_err() {
                break;
            }
        }
    };
    tokio::select! {
        () = outbound => {}
        () = inbound => {}
    }
}

/// `message`, with `context` as the browser context of a `Target.createTarget`
/// request that names none; any other message as it is.
fn in_context(message: Message, context: &str) -> Message {
    let Message::Text(text) = &message else {
        return message;
    };
    if !text.contains("\"Target.createTarget\"") {
        return message;
    }
    let Ok(mut request) = serde_json::from_str::<Value>(text) else {
        return message;
    };
    if request["method"] != "Target.createTarget"
        || request["params"]["browserContextId"].is_string()
    {
        return message;
    }
    request["params"]["browserContextId"] = json!(context);
    Message::Text(request.to_string().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &Value) -> Message {
        Message::Text(value.to_string().into())
    }

    fn json_of(message: &Message) -> Value {
        match message {
            Message::Text(text) => serde_json::from_str(text).unwrap(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_new_tab_is_created_in_the_tests_context() {
        let create = json!({"id": 7, "method": "Target.createTarget",
            "params": {"url": "about:blank", "background": true}});
        let sent = in_context(text(&create), "CTX");
        assert_eq!(
            json_of(&sent),
            json!({"id": 7, "method": "Target.createTarget",
                "params": {"url": "about:blank", "background": true, "browserContextId": "CTX"}})
        );
        // One that names a context keeps it.
        let named = text(&json!({"id": 8, "method": "Target.createTarget",
            "params": {"url": "about:blank", "browserContextId": "OTHER"}}));
        assert_eq!(
            json_of(&in_context(named, "CTX"))["params"]["browserContextId"],
            "OTHER"
        );
    }

    #[test]
    fn nothing_else_is_changed() {
        let close = text(&json!({"id": 9, "method": "Target.closeTarget",
            "params": {"targetId": "T"}}));
        assert_eq!(json_of(&in_context(close.clone(), "CTX")), json_of(&close));
        // A page that merely mentions the method is not a request.
        let mention = text(&json!({"id": 10, "method": "Runtime.evaluate",
            "params": {"expression": "'\"Target.createTarget\"'"}}));
        assert_eq!(
            json_of(&in_context(mention.clone(), "CTX")),
            json_of(&mention)
        );
        let binary = Message::Binary(vec![1, 2, 3].into());
        assert_eq!(in_context(binary.clone(), "CTX"), binary);
    }

    #[test]
    fn a_test_owns_the_pages_of_its_context() {
        let listed = json!({"targetInfos": [
            {"targetId": "a", "type": "page", "browserContextId": "MINE"},
            {"targetId": "b", "type": "page", "browserContextId": "THEIRS"},
            {"targetId": "c", "type": "page", "browserContextId": "MINE", "openerId": "a"},
            {"targetId": "d", "type": "shared_worker", "browserContextId": "MINE"},
            {"targetId": "e", "type": "page", "browserContextId": "DEFAULT"},
        ]});
        let ids = |pages: Vec<Value>| {
            pages
                .iter()
                .map(|page| page["targetId"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(pages_in(&listed, Some("MINE"))), ["a", "c"]);
        // A Chrome of one test's own: every page is the test's.
        assert_eq!(ids(pages_in(&listed, None)), ["a", "b", "c", "e"]);
    }
}
