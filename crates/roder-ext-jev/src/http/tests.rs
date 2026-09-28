use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::*;

/// One scripted answer from the mock provider.
#[derive(Clone)]
pub(crate) enum Reply {
    /// A status line, extra headers and a body.
    Status(u16, Vec<(&'static str, &'static str)>, &'static str),
    /// Read the request, then never answer.
    Hang,
    /// Read the request, then close the socket without a response.
    Drop,
}

impl Reply {
    pub(crate) fn ok(body: &'static str) -> Self {
        Self::Status(200, Vec::new(), body)
    }

    pub(crate) fn status(code: u16) -> Self {
        Self::Status(code, Vec::new(), r#"{"error":"scripted"}"#)
    }
}

/// A local HTTP/1.1 server that answers each connection with the next
/// scripted reply (the last one repeats) and records every request body.
pub(crate) struct MockServer {
    pub(crate) url: String,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
}

impl MockServer {
    pub(crate) async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
        let recorded = requests.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let queue = queue.clone();
                let recorded = recorded.clone();
                tokio::spawn(async move { serve(stream, queue, recorded).await });
            }
        });
        Self { url, requests }
    }

    pub(crate) fn hits(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    pub(crate) fn requests(&self) -> Vec<(String, Value)> {
        self.requests.lock().unwrap().clone()
    }
}

async fn serve(
    mut stream: TcpStream,
    queue: Arc<Mutex<VecDeque<Reply>>>,
    recorded: Arc<Mutex<Vec<(String, Value)>>>,
) {
    let Some((authorization, body)) = read_request(&mut stream).await else {
        return;
    };
    let reply = {
        let mut queue = queue.lock().unwrap();
        if queue.len() > 1 {
            queue.pop_front().unwrap()
        } else {
            queue.front().cloned().unwrap()
        }
    };
    recorded.lock().unwrap().push((authorization, body));
    match reply {
        Reply::Status(code, headers, body) => {
            let mut head = format!(
                "HTTP/1.1 {code} Scripted\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
                body.len()
            );
            for (name, value) in headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str("\r\n");
            stream.write_all(head.as_bytes()).await.ok();
            stream.write_all(body.as_bytes()).await.ok();
            stream.shutdown().await.ok();
        }
        Reply::Hang => tokio::time::sleep(Duration::from_secs(30)).await,
        Reply::Drop => drop(stream),
    }
}

/// The request's `authorization` header and JSON body.
async fn read_request(stream: &mut TcpStream) -> Option<(String, Value)> {
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
    let header = |name: &str| {
        head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
    };
    let length = header("content-length")?.parse::<usize>().ok()?;
    while buffer.len() < header_end + length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = serde_json::from_slice(&buffer[header_end..header_end + length]).ok()?;
    Some((header("authorization").unwrap_or_default(), body))
}

/// The production schedule's shape with millisecond waits.
pub(crate) fn fast_policy() -> RetryPolicy {
    RetryPolicy {
        attempt_timeout: Duration::from_millis(300),
        delays: vec![Duration::from_millis(1); 5],
        max_retry_after: Duration::from_secs(10),
        resend_limit: 1,
    }
}

#[test]
fn default_policy_matches_the_documented_schedule() {
    let policy = RetryPolicy::default();
    assert_eq!(policy.attempt_timeout, Duration::from_secs(15));
    assert_eq!(
        policy.delays,
        [500, 1500, 4000, 8000, 8000].map(Duration::from_millis)
    );
    assert_eq!(policy.max_retry_after, Duration::from_secs(10));
    assert_eq!(policy.resend_limit, 1);
    let text = RetryPolicy::text_helper();
    assert_eq!(text.delays, [500, 1500].map(Duration::from_millis));
    assert_eq!(text.attempt_timeout, policy.attempt_timeout);
}

#[test]
fn retry_after_prefers_milliseconds_and_is_capped() {
    let cap = Duration::from_secs(10);
    let headers = |pairs: &[(&'static str, &'static str)]| {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, value.parse().unwrap());
        }
        map
    };
    assert_eq!(
        retry_after(
            &headers(&[("retry-after-ms", "250"), ("retry-after", "3")]),
            cap
        ),
        Some(Duration::from_millis(250))
    );
    assert_eq!(
        retry_after(&headers(&[("retry-after", "2.5")]), cap),
        Some(Duration::from_millis(2500))
    );
    assert_eq!(
        retry_after(&headers(&[("retry-after", "3600")]), cap),
        Some(cap)
    );
    assert_eq!(
        retry_after(&headers(&[("retry-after", "1e400")]), cap),
        Some(cap)
    );
    assert_eq!(
        retry_after(&headers(&[("retry-after", "-4")]), cap),
        Some(Duration::ZERO)
    );
    assert_eq!(
        retry_after(
            &headers(&[("retry-after", "Wed, 21 Oct 2026 07:28:00 GMT")]),
            cap
        ),
        None
    );
    assert_eq!(retry_after(&headers(&[]), cap), None);
    // An unreadable header defers to the next one.
    assert_eq!(
        retry_after(
            &headers(&[("retry-after-ms", "soon"), ("retry-after", "5")]),
            cap
        ),
        Some(Duration::from_secs(5))
    );
}

#[tokio::test]
async fn transient_failures_are_retried_until_success() {
    let server = MockServer::start(vec![
        Reply::status(503),
        Reply::status(520),
        Reply::Drop,
        Reply::status(429),
        Reply::ok(r#"{"answers":{}}"#),
    ])
    .await;
    let poster = JsonPoster::new(fast_policy());
    let body = json!({"model": "jev-latest"});

    let value = poster.post(&server.url, "sk-test", &body).await.unwrap();

    assert_eq!(value, json!({"answers": {}}));
    let requests = server.requests();
    assert_eq!(requests.len(), 5);
    // Every attempt resends the identical, authenticated request.
    assert!(
        requests
            .iter()
            .all(|(auth, sent)| auth == "Bearer sk-test" && *sent == body)
    );
}

#[tokio::test]
async fn a_stalled_attempt_times_out_and_is_retried() {
    let server = MockServer::start(vec![Reply::Hang, Reply::ok(r#"{"ok":true}"#)]).await;
    let poster = JsonPoster::new(RetryPolicy {
        attempt_timeout: Duration::from_millis(100),
        ..fast_policy()
    });
    let started = Instant::now();

    let value = poster.post(&server.url, "k", &json!({})).await.unwrap();

    assert_eq!(value, json!({"ok": true}));
    assert_eq!(server.hits(), 2);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_garbled_body_is_retried() {
    let server =
        MockServer::start(vec![Reply::ok("<html>gateway"), Reply::ok(r#"{"ok":1}"#)]).await;
    let poster = JsonPoster::new(fast_policy());

    assert_eq!(
        poster.post(&server.url, "k", &json!({})).await,
        Ok(json!({"ok": 1}))
    );
    assert_eq!(server.hits(), 2);
}

#[tokio::test]
async fn retry_after_overrides_the_backoff() {
    let server = MockServer::start(vec![
        Reply::Status(429, vec![("retry-after-ms", "400")], "{}"),
        Reply::ok("{}"),
    ])
    .await;
    // The policy's own delay is a millisecond, so only the header explains a
    // wait this long.
    let poster = JsonPoster::new(fast_policy());
    let started = Instant::now();

    poster.post(&server.url, "k", &json!({})).await.unwrap();

    let elapsed = started.elapsed();
    assert_eq!(server.hits(), 2);
    assert!(elapsed >= Duration::from_millis(400), "waited {elapsed:?}");
    assert!(elapsed < Duration::from_secs(5), "waited {elapsed:?}");
}

#[tokio::test]
async fn retry_after_is_capped_by_the_policy() {
    let server = MockServer::start(vec![
        Reply::Status(503, vec![("retry-after", "3600")], "{}"),
        Reply::ok("{}"),
    ])
    .await;
    let poster = JsonPoster::new(RetryPolicy {
        max_retry_after: Duration::from_millis(50),
        ..fast_policy()
    });
    let started = Instant::now();

    poster.post(&server.url, "k", &json!({})).await.unwrap();

    assert_eq!(server.hits(), 2);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_non_retryable_status_fails_at_once() {
    for status in [400, 401, 403, 404, 422, 501] {
        let server = MockServer::start(vec![Reply::status(status), Reply::ok("{}")]).await;
        let poster = JsonPoster::new(fast_policy());

        let failure = poster.post(&server.url, "k", &json!({})).await.unwrap_err();

        let detail = (status == 422).then(|| r#"{"error":"scripted"}"#.to_string());
        assert_eq!(failure, PostFailure::Status(status, detail));
        assert_eq!(server.hits(), 1, "status {status} was retried");
    }
}

#[tokio::test]
async fn exhausted_retries_report_the_last_failure() {
    let server = MockServer::start(vec![Reply::status(502)]).await;
    let poster = JsonPoster::new(fast_policy());

    let failure = poster.post(&server.url, "k", &json!({})).await.unwrap_err();

    assert_eq!(failure, PostFailure::Status(502, None));
    // One attempt plus one per delay.
    assert_eq!(server.hits(), 6);
}

#[tokio::test]
async fn an_unreachable_provider_is_a_connection_failure_after_retries() {
    // Bind then drop, so the port refuses connections.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    let poster = JsonPoster::new(fast_policy());

    let failure = poster.post(&url, "k", &json!({})).await.unwrap_err();

    assert_eq!(failure, PostFailure::Connection);
}

#[tokio::test]
async fn a_reply_the_provider_may_have_billed_is_resent_only_once() {
    // A proxy's HTML page will not turn into JSON however often it is asked.
    let server = MockServer::start(vec![Reply::ok("<html>gateway")]).await;
    let poster = JsonPoster::new(fast_policy());

    let failure = poster.post(&server.url, "k", &json!({})).await.unwrap_err();

    assert_eq!(failure, PostFailure::InvalidBody);
    assert_eq!(server.hits(), 2);

    // Nor an attempt that timed out after it was sent.
    let server = MockServer::start(vec![Reply::Hang]).await;
    let poster = JsonPoster::new(RetryPolicy {
        attempt_timeout: Duration::from_millis(100),
        ..fast_policy()
    });

    let failure = poster.post(&server.url, "k", &json!({})).await.unwrap_err();

    assert_eq!(failure, PostFailure::Connection);
    assert_eq!(server.hits(), 2);
}

/// A 500, 504 or 524 can come after the origin ran the request, so it is
/// sent again only once, like a reply lost in transit; the provider is still
/// reported unavailable.
#[tokio::test]
async fn a_status_after_which_the_origin_may_have_run_is_resent_only_once() {
    for status in [500, 504, 524] {
        let server = MockServer::start(vec![Reply::status(status)]).await;
        let poster = JsonPoster::new(fast_policy());

        let failure = poster.post(&server.url, "k", &json!({})).await.unwrap_err();

        assert_eq!(failure, PostFailure::Status(status, None));
        assert!(failure.unavailable(), "{status}");
        assert_eq!(server.hits(), 2, "status {status}");
    }
    // A status saying the request was never served is retried in full.
    let server = MockServer::start(vec![
        Reply::status(503),
        Reply::status(500),
        Reply::ok("{}"),
    ])
    .await;
    let poster = JsonPoster::new(fast_policy());
    assert_eq!(
        poster.post(&server.url, "k", &json!({})).await,
        Ok(json!({}))
    );
    assert_eq!(server.hits(), 3);
}

#[tokio::test]
async fn no_retry_starts_that_would_outlast_the_run() {
    let server = MockServer::start(vec![Reply::status(503)]).await;
    let poster = JsonPoster::new(RetryPolicy {
        delays: vec![Duration::from_millis(400); 5],
        ..fast_policy()
    });
    // The reserve leaves 1.5 s, room for the first retries but not all five.
    let deadline = Instant::now() + Duration::from_millis(2500);
    let started = Instant::now();

    let failure = RUN_DEADLINE
        .scope(deadline.into(), poster.post(&server.url, "k", &json!({})))
        .await
        .unwrap_err();

    // The provider's own failure, reported before the run's deadline (due
    // at about 1.2 s; the bound is the deadline itself, for a loaded
    // machine).
    assert_eq!(failure, PostFailure::Status(503, None));
    assert!(started.elapsed() < Duration::from_millis(2500));
    assert!((2..6).contains(&server.hits()), "{} hits", server.hits());
}
