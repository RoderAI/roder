//! A local site serving `tests/fixtures/pages/*.html` and recording POSTs.
//!
//! The idea follows fastbrowse's fixture server (MIT, evals/local.py); the
//! code is our own: plain HTTP/1.1 on a tokio listener, one request per
//! connection, no new dependencies.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const THANKS: &str = "<!doctype html><title>Thanks</title><h1>Thanks, we received it.</h1>";

/// One form submission as the server received it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Post {
    pub(crate) path: String,
    pub(crate) body: String,
}

impl Post {
    /// The decoded `application/x-www-form-urlencoded` value of `field`.
    pub(crate) fn field(&self, field: &str) -> Option<String> {
        self.body.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(key) == field).then(|| decode(value))
        })
    }
}

pub(crate) struct FixtureSite {
    origin: String,
    posts: Arc<Mutex<Vec<Post>>>,
    /// The path of every request that reached a failure route.
    failures: Arc<Mutex<Vec<String>>>,
}

impl FixtureSite {
    /// Bind `127.0.0.1:0` and serve until the test's runtime ends.
    pub(crate) async fn start() -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let posts = Arc::new(Mutex::new(Vec::new()));
        let failures = Arc::new(Mutex::new(Vec::new()));
        let (recorded, failed) = (posts.clone(), failures.clone());
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (recorded, failed) = (recorded.clone(), failed.clone());
                tokio::spawn(async move { serve(stream, recorded, failed).await });
            }
        });
        Ok(Self {
            origin,
            posts,
            failures,
        })
    }

    /// A URL on this site outside `/pages/`, such as a failure route.
    pub(crate) fn path_url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }

    /// How many requests reached failure routes starting with `prefix`.
    pub(crate) fn failure_hits(&self, prefix: &str) -> usize {
        let failures = self.failures.lock().unwrap();
        failures
            .iter()
            .filter(|path| path.starts_with(prefix))
            .count()
    }

    /// `http://127.0.0.1:<port>`.
    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    /// The URL of a fixture page, with an optional query string.
    pub(crate) fn url(&self, page: &str) -> String {
        format!("{}/pages/{page}", self.origin)
    }

    pub(crate) fn posts(&self) -> Vec<Post> {
        self.posts.lock().unwrap().clone()
    }

    /// Wait for at least `count` submissions; a submit navigates, so the POST
    /// can land after the act that caused it has returned.
    pub(crate) async fn wait_for_posts(&self, count: usize, timeout: Duration) -> Vec<Post> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let posts = self.posts();
            if posts.len() >= count || tokio::time::Instant::now() >= deadline {
                return posts;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

fn pages_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages")
}

/// Failure routes for loads that go wrong: `/fail/drop` closes the
/// connection with no reply (`net::ERR_EMPTY_RESPONSE`),
/// `/fail/no-content` answers 204, which Chrome abandons (`net::ERR_ABORTED`),
/// and `/fail/abort-once/<page>.html` answers 204 the first time and serves
/// the page after that.
async fn serve(
    mut stream: TcpStream,
    posts: Arc<Mutex<Vec<Post>>>,
    failures: Arc<Mutex<Vec<String>>>,
) {
    let Some((method, path, body)) = read_request(&mut stream).await else {
        return;
    };
    if let Some(page_path) = path.strip_prefix("/fail/abort-once") {
        let seen = {
            let mut failures = failures.lock().unwrap();
            let seen = failures.contains(&path);
            failures.push(path.clone());
            seen
        };
        if seen && let Some(content) = page(&format!("/pages{page_path}")).await {
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n",
                content.len()
            );
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(&content).await;
        } else {
            let head = "HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
            let _ = stream.write_all(head.as_bytes()).await;
        }
        let _ = stream.shutdown().await;
        return;
    }
    if path.starts_with("/fail/") {
        failures.lock().unwrap().push(path.clone());
        if path.starts_with("/fail/no-content") {
            let head = "HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
            let _ = stream.write_all(head.as_bytes()).await;
        }
        let _ = stream.shutdown().await;
        return;
    }
    let (status, content) = match method.as_str() {
        "GET" => match delayed_page(&path).await {
            Some(content) => (served_status(&path), content),
            None => ("404 Not Found", b"not found".to_vec()),
        },
        "POST" => {
            posts.lock().unwrap().push(Post { path, body });
            ("200 OK", THANKS.as_bytes().to_vec())
        }
        _ => ("405 Method Not Allowed", Vec::new()),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\n\
         content-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
        content.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(&content).await;
    let _ = stream.shutdown().await;
}

/// A `status=<code>` query serves the page with that refusal status
/// instead of 200, as a bot-protection edge serves an automated browser.
fn served_status(path: &str) -> &'static str {
    let status = query_value(path, "status");
    match status {
        Some("401") => "401 Unauthorized",
        Some("403") => "403 Forbidden",
        Some("429") => "429 Too Many Requests",
        _ => "200 OK",
    }
}

fn query_value<'a>(path: &'a str, key: &str) -> Option<&'a str> {
    path.split_once('?')?.1.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then_some(value)
    })
}

/// A `delay=<ms>` query (capped at 10 s) holds the reply back, so a fixture
/// can fetch from this server with real network latency.
async fn delayed_page(path: &str) -> Option<Vec<u8>> {
    let delay = query_value(path, "delay")
        .and_then(|ms| ms.parse::<u64>().ok())
        .unwrap_or(0)
        .min(10_000);
    tokio::time::sleep(Duration::from_millis(delay)).await;
    page(path).await
}

/// Only flat `*.html` names under `/pages/` are served; nothing can escape
/// the fixture directory.
async fn page(path: &str) -> Option<Vec<u8>> {
    let name = path.split('?').next()?.strip_prefix("/pages/")?;
    let safe = name.ends_with(".html")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.contains("..");
    if !safe {
        return None;
    }
    tokio::fs::read(pages_dir().join(name)).await.ok()
}

pub(crate) async fn read_request(stream: &mut TcpStream) -> Option<(String, String, String)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut request_line = head.lines().next()?.split_whitespace();
    let method = request_line.next()?.to_string();
    let path = request_line.next()?.to_string();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    while buffer.len() < header_end + length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let end = buffer.len().min(header_end + length);
    let body = String::from_utf8_lossy(&buffer[header_end..end]).to_string();
    Some((method, path, body))
}

/// Percent-decoding for form bodies, with `+` as a space.
fn decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes
                    .get(index + 1..index + 3)
                    .and_then(|hex| std::str::from_utf8(hex).ok())
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                match hex {
                    Some(byte) => {
                        out.push(byte);
                        index += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_fields_are_decoded() {
        let post = Post {
            path: "/submit".into(),
            body: "name=Ada+Lovelace&email=ada%40example.com&empty=&flag".into(),
        };
        assert_eq!(post.field("name").as_deref(), Some("Ada Lovelace"));
        assert_eq!(post.field("email").as_deref(), Some("ada@example.com"));
        assert_eq!(post.field("empty").as_deref(), Some(""));
        assert_eq!(post.field("flag").as_deref(), Some(""));
        assert_eq!(post.field("missing"), None);
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz"), "%zz");
    }

    #[tokio::test]
    async fn serves_only_fixture_pages_and_records_posts() {
        let site = FixtureSite::start().await.unwrap();
        let client = reqwest::Client::new();
        let page = client.get(site.url("basic.html")).send().await.unwrap();
        assert_eq!(page.status(), 200);
        assert!(page.text().await.unwrap().contains("Basic controls"));
        for path in ["/pages/../Cargo.toml", "/pages/missing.html", "/Cargo.toml"] {
            let url = format!("{}{path}", site.origin);
            assert_eq!(
                client.get(url).send().await.unwrap().status(),
                404,
                "{path}"
            );
        }
        let refused = client
            .get(site.url("access-denied.html?status=403"))
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 403);
        assert!(refused.text().await.unwrap().contains("Access Denied"));
        let posted = client
            .post(format!("{}/submit/x", site.origin))
            .body("a=1&b=two+words")
            .send()
            .await
            .unwrap();
        assert!(posted.text().await.unwrap().contains("Thanks"));
        let posts = site.wait_for_posts(1, Duration::from_secs(1)).await;
        assert_eq!(posts[0].path, "/submit/x");
        assert_eq!(posts[0].field("b").as_deref(), Some("two words"));
    }
}
