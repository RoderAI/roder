//! HMAC-signed webhook delivery with bounded retries, timeout, a per-hook
//! circuit breaker, redacted delivery records, and dead-letter retention
//! (roadmap phase 72, Task 4).
//!
//! Failure handling is bounded by design: a failing hook produces a
//! `Failed`/`Dead` delivery record and (for blocking hook kinds) a
//! deny/continue decision per `on_failure` — never a hanging request.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use roder_api::hosted_hooks::{
    HookDelivery, HookDeliveryStatus, HookRetryPolicy, HostedHookDefinition,
};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use super::hooks::resolve_signing_secret;

/// Signature header carried on every delivery.
pub const SIGNATURE_HEADER: &str = "x-roder-signature";

/// HMAC-SHA256 (RFC 2104) over the payload using the resolved secret.
pub fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    const BLOCK: usize = 64;
    let mut key_block = [0u8; BLOCK];
    if key.len() > BLOCK {
        key_block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(key_block.map(|byte| byte ^ 0x36));
    inner.update(message);
    let inner_hash = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(key_block.map(|byte| byte ^ 0x5c));
    outer.update(inner_hash);
    outer
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What blocking hook kinds do when delivery ultimately fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookFailureMode {
    /// Treat the event as approved/continue (default).
    Continue,
    /// Fail closed: deny the gated action.
    Deny,
}

#[derive(Debug, Clone)]
pub struct HookDeliveryConfig {
    pub retry: HookRetryPolicy,
    /// Per-attempt HTTP timeout.
    pub timeout: Duration,
    /// Consecutive terminal failures before the breaker opens.
    pub circuit_threshold: u32,
    /// How long an open breaker fails fast before retrying.
    pub circuit_cooldown: Duration,
    pub on_failure: HookFailureMode,
}

impl Default for HookDeliveryConfig {
    fn default() -> Self {
        Self {
            retry: HookRetryPolicy::default(),
            timeout: Duration::from_secs(10),
            circuit_threshold: 3,
            circuit_cooldown: Duration::from_secs(60),
            on_failure: HookFailureMode::Continue,
        }
    }
}

#[derive(Default)]
struct BreakerState {
    consecutive_failures: u32,
    open_until: Option<std::time::Instant>,
}

pub struct HookDeliveryService {
    config: HookDeliveryConfig,
    client: reqwest::Client,
    breakers: Mutex<HashMap<String, BreakerState>>,
    /// Terminal failures kept for operators (redacted records only).
    dead_letters: Mutex<Vec<HookDelivery>>,
    deliveries: Mutex<Vec<HookDelivery>>,
}

impl HookDeliveryService {
    pub fn new(config: HookDeliveryConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("reqwest client");
        Self {
            config,
            client,
            breakers: Mutex::new(HashMap::new()),
            dead_letters: Mutex::new(Vec::new()),
            deliveries: Mutex::new(Vec::new()),
        }
    }

    pub fn on_failure(&self) -> HookFailureMode {
        self.config.on_failure
    }

    pub fn dead_letters(&self) -> Vec<HookDelivery> {
        self.dead_letters.lock().unwrap().clone()
    }

    pub fn deliveries(&self) -> Vec<HookDelivery> {
        self.deliveries.lock().unwrap().clone()
    }

    /**
     * Delivers a payload to a hook with signing, bounded retries, and the
     * circuit breaker. Returns the redacted delivery record; payloads,
     * response bodies, headers, and secrets are never recorded.
     */
    pub async fn deliver(
        &self,
        hook: &HostedHookDefinition,
        event_kind: &str,
        payload: &serde_json::Value,
    ) -> HookDelivery {
        let mut delivery = HookDelivery {
            id: uuid::Uuid::new_v4().to_string(),
            hook_id: hook.id.clone(),
            event_kind: event_kind.to_string(),
            status: HookDeliveryStatus::Pending,
            attempts: 0,
            last_error: None,
            created_at: OffsetDateTime::now_utc(),
            delivered_at: None,
        };

        if self.breaker_is_open(&hook.id) {
            delivery.status = HookDeliveryStatus::Failed;
            delivery.last_error = Some("circuit_open".to_string());
            self.deliveries.lock().unwrap().push(delivery.clone());
            return delivery;
        }

        let body = serde_json::to_vec(payload).unwrap_or_default();
        let signature = match &hook.signing_secret_ref {
            Some(secret_ref) => match resolve_signing_secret(secret_ref) {
                Ok(secret) => Some(format!(
                    "sha256={}",
                    hmac_sha256_hex(secret.as_bytes(), &body)
                )),
                Err(_) => {
                    // A configured-but-unresolvable secret fails closed.
                    delivery.status = HookDeliveryStatus::Failed;
                    delivery.last_error = Some("secret_unresolvable".to_string());
                    self.record_failure(&hook.id);
                    self.deliveries.lock().unwrap().push(delivery.clone());
                    return delivery;
                }
            },
            None => None,
        };

        for attempt in 1..=self.config.retry.max_attempts.max(1) {
            delivery.attempts = attempt;
            let mut request = self
                .client
                .post(&hook.url)
                .header("content-type", "application/json")
                .header("x-roder-event", event_kind)
                .body(body.clone());
            if let Some(signature) = &signature {
                request = request.header(SIGNATURE_HEADER, signature);
            }
            match request.send().await {
                Ok(response) if response.status().is_success() => {
                    delivery.status = HookDeliveryStatus::Delivered;
                    delivery.last_error = None;
                    delivery.delivered_at = Some(OffsetDateTime::now_utc());
                    self.record_success(&hook.id);
                    self.deliveries.lock().unwrap().push(delivery.clone());
                    return delivery;
                }
                Ok(response) => {
                    let class = if response.status().is_client_error() {
                        "http_4xx"
                    } else {
                        "http_5xx"
                    };
                    delivery.last_error = Some(class.to_string());
                    // 4xx responses are not retried: the target rejected us.
                    if class == "http_4xx" {
                        break;
                    }
                }
                Err(error) => {
                    delivery.last_error = Some(if error.is_timeout() {
                        "timeout".to_string()
                    } else {
                        "connect".to_string()
                    });
                }
            }
            if attempt < self.config.retry.max_attempts {
                let backoff = (self.config.retry.initial_backoff_ms << (attempt - 1))
                    .min(self.config.retry.max_backoff_ms);
                tokio::time::sleep(Duration::from_millis(backoff)).await;
            }
        }

        delivery.status = if delivery.attempts >= self.config.retry.max_attempts
            || delivery.last_error.as_deref() == Some("http_4xx")
        {
            HookDeliveryStatus::Dead
        } else {
            HookDeliveryStatus::Failed
        };
        self.record_failure(&hook.id);
        if delivery.status == HookDeliveryStatus::Dead {
            self.dead_letters.lock().unwrap().push(delivery.clone());
        }
        self.deliveries.lock().unwrap().push(delivery.clone());
        delivery
    }

    fn breaker_is_open(&self, hook_id: &str) -> bool {
        let mut breakers = self.breakers.lock().unwrap();
        let Some(state) = breakers.get_mut(hook_id) else {
            return false;
        };
        match state.open_until {
            Some(until) if std::time::Instant::now() < until => true,
            Some(_) => {
                // Cooldown elapsed: half-open, allow one probe.
                state.open_until = None;
                false
            }
            None => false,
        }
    }

    fn record_failure(&self, hook_id: &str) {
        let mut breakers = self.breakers.lock().unwrap();
        let state = breakers.entry(hook_id.to_string()).or_default();
        state.consecutive_failures += 1;
        if state.consecutive_failures >= self.config.circuit_threshold {
            state.open_until = Some(std::time::Instant::now() + self.config.circuit_cooldown);
        }
    }

    fn record_success(&self, hook_id: &str) {
        let mut breakers = self.breakers.lock().unwrap();
        let state = breakers.entry(hook_id.to_string()).or_default();
        state.consecutive_failures = 0;
        state.open_until = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_matches_rfc4231_test_vector() {
        // RFC 4231 test case 2: key "Jefe", data "what do ya want for nothing?".
        let mac = hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            mac,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }
}
