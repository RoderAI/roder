//! Hosted hook delivery tests (phase 72, Task 4): HMAC signatures verified
//! by the receiver, retry/dead-letter behavior, timeout classing, the
//! circuit breaker, and secret redaction. Fully offline against a local
//! fake HTTP server.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use roder_api::hosted_hooks::{
    HookDeliveryStatus, HookRetryPolicy, HookScope, HostedHookDefinition,
};
use roder_app_server_hosted::hook_delivery::hmac_sha256_hex;
use roder_app_server_hosted::{HookDeliveryConfig, HookDeliveryService, HookStore};
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SECRET_ENV: &str = "RODER_TEST_HOOK_SECRET_72";
const SECRET: &str = "hook-secret-value";

/// Behavior of the fake hook target per request index.
#[derive(Clone, Copy)]
enum Plan {
    Ok,
    ServerError,
    Hang,
}

struct FakeTarget {
    url: String,
    hits: Arc<AtomicUsize>,
    /// Captured `(event header, signature header, body)` tuples.
    captures: Arc<std::sync::Mutex<Vec<(String, String, String)>>>,
}

async fn fake_target(plan: Vec<Plan>) -> FakeTarget {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let captures = Arc::new(std::sync::Mutex::new(Vec::new()));
    let task_hits = hits.clone();
    let task_captures = captures.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let index = task_hits.fetch_add(1, Ordering::SeqCst);
            let step = plan.get(index).copied().unwrap_or(Plan::Ok);
            let mut buffer = vec![0u8; 65536];
            let mut read = 0;
            // Read until the full body arrived (content-length framing).
            let request = loop {
                let n = stream.read(&mut buffer[read..]).await.unwrap_or(0);
                if n == 0 {
                    break String::new();
                }
                read += n;
                let text = String::from_utf8_lossy(&buffer[..read]).to_string();
                if let Some(headers_end) = text.find("\r\n\r\n") {
                    let content_length = text.lines().find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    });
                    let body_len = read - (headers_end + 4);
                    if body_len >= content_length.unwrap_or(0) {
                        break text;
                    }
                }
            };
            if !request.is_empty() {
                let header = |name: &str| {
                    request
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .starts_with(&format!("{name}:"))
                                .then(|| line.split_once(':').unwrap().1.trim().to_string())
                        })
                        .unwrap_or_default()
                };
                let body = request
                    .split_once("\r\n\r\n")
                    .map(|(_, body)| body.to_string())
                    .unwrap_or_default();
                task_captures.lock().unwrap().push((
                    header("x-roder-event"),
                    header("x-roder-signature"),
                    body,
                ));
            }
            match step {
                Plan::Ok => {
                    let _ = stream
                        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                        .await;
                }
                Plan::ServerError => {
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\n\r\n",
                        )
                        .await;
                }
                Plan::Hang => {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }
            }
        }
    });
    FakeTarget {
        url,
        hits,
        captures,
    }
}

fn hook(url: &str) -> HostedHookDefinition {
    HostedHookDefinition {
        id: "hook-1".to_string(),
        scope: HookScope::Tenant,
        tenant_id: Some("tenant-a".to_string()),
        event_kinds: vec!["turn.".to_string()],
        url: url.to_string(),
        signing_secret_ref: Some(format!("env:{SECRET_ENV}")),
        enabled: true,
        created_at: OffsetDateTime::UNIX_EPOCH,
        updated_at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn fast_config() -> HookDeliveryConfig {
    HookDeliveryConfig {
        retry: HookRetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 10,
            max_backoff_ms: 50,
        },
        timeout: std::time::Duration::from_millis(500),
        circuit_threshold: 2,
        circuit_cooldown: std::time::Duration::from_secs(60),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn deliveries_are_signed_and_receivers_can_verify() {
    // SAFETY: test-local env var, set before any delivery resolves it.
    unsafe { std::env::set_var(SECRET_ENV, SECRET) };
    let target = fake_target(vec![Plan::Ok]).await;
    let service = HookDeliveryService::new(fast_config());

    let payload = serde_json::json!({ "kind": "turn.started", "threadId": "t-1" });
    let delivery = service
        .deliver(&hook(&target.url), "turn.started", &payload)
        .await;
    assert_eq!(delivery.status, HookDeliveryStatus::Delivered);
    assert_eq!(delivery.attempts, 1);

    let captures = target.captures.lock().unwrap();
    let (event, signature, body) = captures.first().expect("captured request").clone();
    assert_eq!(event, "turn.started");
    // Receiver-side verification: recompute the HMAC over the raw body.
    let expected = format!(
        "sha256={}",
        hmac_sha256_hex(SECRET.as_bytes(), body.as_bytes())
    );
    assert_eq!(signature, expected);
    assert!(body.contains("turn.started"));
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_deliveries_retry_then_dead_letter_with_redacted_records() {
    unsafe { std::env::set_var(SECRET_ENV, SECRET) };
    let target = fake_target(vec![
        Plan::ServerError,
        Plan::ServerError,
        Plan::ServerError,
    ])
    .await;
    let service = HookDeliveryService::new(fast_config());

    let delivery = service
        .deliver(
            &hook(&target.url),
            "turn.started",
            &serde_json::json!({"secretish": "payload"}),
        )
        .await;
    assert_eq!(delivery.status, HookDeliveryStatus::Dead);
    assert_eq!(delivery.attempts, 3);
    assert_eq!(delivery.last_error.as_deref(), Some("http_5xx"));
    assert_eq!(
        target.hits.load(Ordering::SeqCst),
        3,
        "5xx responses are retried"
    );

    let dead = service.dead_letters();
    assert_eq!(dead.len(), 1);
    // Records are redacted: no payloads, no secrets, no URLs with secrets.
    let serialized = serde_json::to_string(&dead).unwrap();
    assert!(!serialized.contains("payload"));
    assert!(!serialized.contains(SECRET));
}

#[tokio::test(flavor = "multi_thread")]
async fn timeouts_are_classed_and_circuit_breaker_fails_fast() {
    unsafe { std::env::set_var(SECRET_ENV, SECRET) };
    let target = fake_target(vec![
        Plan::Hang,
        Plan::Hang,
        Plan::Hang,
        Plan::Hang,
        Plan::Hang,
        Plan::Hang,
    ])
    .await;
    let mut config = fast_config();
    config.retry.max_attempts = 2;
    let service = HookDeliveryService::new(config);
    let hook = hook(&target.url);

    let delivery = service
        .deliver(&hook, "turn.started", &serde_json::json!({}))
        .await;
    assert_eq!(delivery.status, HookDeliveryStatus::Dead);
    assert_eq!(delivery.last_error.as_deref(), Some("timeout"));

    // A second terminal failure reaches the breaker threshold (2); the
    // next delivery fails fast without touching the network.
    let second = service
        .deliver(&hook, "turn.started", &serde_json::json!({}))
        .await;
    assert_eq!(second.last_error.as_deref(), Some("timeout"));
    let hits_before = target.hits.load(Ordering::SeqCst);
    let fast_fail = service
        .deliver(&hook, "turn.started", &serde_json::json!({}))
        .await;
    assert_eq!(fast_fail.last_error.as_deref(), Some("circuit_open"));
    assert_eq!(fast_fail.attempts, 0);
    assert_eq!(target.hits.load(Ordering::SeqCst), hits_before);
}

#[tokio::test(flavor = "multi_thread")]
async fn unresolvable_secrets_fail_closed_without_sending() {
    let target = fake_target(vec![Plan::Ok]).await;
    let service = HookDeliveryService::new(fast_config());
    let mut hook = hook(&target.url);
    hook.signing_secret_ref = Some("env:RODER_DEFINITELY_UNSET_SECRET_72".to_string());

    let delivery = service
        .deliver(&hook, "turn.started", &serde_json::json!({}))
        .await;
    assert_eq!(delivery.status, HookDeliveryStatus::Failed);
    assert_eq!(delivery.last_error.as_deref(), Some("secret_unresolvable"));
    assert_eq!(
        target.hits.load(Ordering::SeqCst),
        0,
        "nothing must be sent unsigned"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hook_store_test_delivery_round_trips() {
    unsafe { std::env::set_var(SECRET_ENV, SECRET) };
    let target = fake_target(vec![Plan::Ok]).await;
    let store = HookStore::default();
    let created = store.create("tenant-a", hook(&target.url)).unwrap();

    // Matching is tenant- and kind-scoped.
    assert_eq!(store.matching("tenant-a", "turn.completed").len(), 1);
    assert!(store.matching("tenant-b", "turn.completed").is_empty());
    assert!(store.matching("tenant-a", "thread.created").is_empty());

    let service = HookDeliveryService::new(fast_config());
    let delivery = service
        .deliver(
            &created,
            "turn.completed",
            &serde_json::json!({ "test": true }),
        )
        .await;
    assert_eq!(delivery.status, HookDeliveryStatus::Delivered);
}
