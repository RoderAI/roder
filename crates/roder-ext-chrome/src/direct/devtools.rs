//! What every DevTools client in Roder needs, whoever owns the socket: the
//! browser websocket an endpoint names, reading one message, and the rule for
//! answering a JavaScript dialog. Jev's own connection (`roder-ext-jev`'s
//! `cdp.rs`) and this crate's [`super::DirectSession`] both use these.

use std::time::Duration;

use anyhow::Context;
use serde_json::Value;

/// Whether `endpoint` is a browser websocket already, rather than an
/// http(s) DevTools address that advertises one.
pub fn is_websocket(endpoint: &str) -> bool {
    let scheme = endpoint.split_once("://").map(|(scheme, _)| scheme);
    scheme.is_some_and(|scheme| {
        scheme.eq_ignore_ascii_case("ws") || scheme.eq_ignore_ascii_case("wss")
    })
}

/// `/json/version` under a DevTools HTTP address: appended to its path, as
/// segments, with its query (often a token) kept.
pub fn version_url(http_endpoint: &str) -> anyhow::Result<reqwest::Url> {
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

/// The browser websocket for `endpoint`: a ws(s) URL is that websocket, and
/// an http(s) address advertises it at `/json/version`. Errors never repeat
/// the URL, which may carry a token.
pub async fn browser_websocket(endpoint: &str) -> anyhow::Result<String> {
    if is_websocket(endpoint) {
        return Ok(endpoint.to_string());
    }
    let version: Value = reqwest::Client::new()
        .get(version_url(endpoint)?)
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

/// An `alert` only informs and a `beforeunload` guards a navigation the
/// task asked for, so both go ahead. A `confirm` or `prompt` asks for
/// consent or input nobody gave, so both are declined.
pub fn accepts_dialog(kind: &str) -> bool {
    matches!(kind, "alert" | "beforeunload")
}

/// Parse one DevTools message. A string holding a lone UTF-16 surrogate
/// (half of an emoji a page script cut, or a page's own text) arrives as an
/// unpaired `\uD83D` escape, which serde_json rejects, and with it the whole
/// reply; such an escape is read as U+FFFD instead.
pub fn decode_message(text: &str) -> anyhow::Result<Value> {
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(error) => match repair_surrogates(text) {
            Some(repaired) => serde_json::from_str(&repaired),
            None => Err(error),
        }
        .context("decode a DevTools message"),
    }
}

/// `text` with every unpaired surrogate escape replaced by `�`, or
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_surrogate_escape_does_not_lose_the_message() {
        let message = r#"{"id":3,"result":{"result":{"value":"ab\ud83d","pair":"😀","low":"\udc00x","slash":"\\ud83d"}}}"#;
        assert!(serde_json::from_str::<Value>(message).is_err());
        let value = decode_message(message).unwrap();
        let result = &value["result"]["result"];
        assert_eq!(result["value"], "ab\u{fffd}");
        assert_eq!(result["pair"], "\u{1f600}");
        assert_eq!(result["low"], "\u{fffd}x");
        // An escaped backslash before "u" is text, not an escape.
        assert_eq!(result["slash"], "\\ud83d");
        assert_eq!(repair_surrogates(r#"{"a":"😀 fine"}"#), None);
        assert!(decode_message("not json").is_err());
    }

    #[test]
    fn only_alerts_and_beforeunload_are_accepted() {
        assert!(accepts_dialog("alert"));
        assert!(accepts_dialog("beforeunload"));
        assert!(!accepts_dialog("confirm"));
        assert!(!accepts_dialog("prompt"));
        assert!(!accepts_dialog("anything else"));
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

    #[test]
    fn websocket_endpoints_skip_the_version_lookup() {
        assert!(is_websocket("ws://127.0.0.1:9222/devtools/browser/abc"));
        assert!(is_websocket("WSS://browser.example.com/?token=t"));
        assert!(!is_websocket("http://127.0.0.1:9222"));
        assert!(!is_websocket("https://ws.example.com"));
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
        let socket = browser_websocket(&format!("http://{address}/t/abc?token=s"))
            .await
            .unwrap();
        assert_eq!(socket, "ws://127.0.0.1:1/devtools/browser/x");
        let line = server.await.unwrap();
        assert!(
            line.starts_with("GET /t/abc/json/version?token=s "),
            "{line}"
        );
    }
}
