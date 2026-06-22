//! Deterministic per-tenant/per-principal rate and size limits.
//!
//! Token buckets keyed by `(tenant, principal)` with an injected clock so
//! tests are deterministic. Size limits are enforced by the gateway before
//! JSON parsing.

use std::collections::HashMap;
use std::sync::Mutex;

use time::OffsetDateTime;

#[derive(Debug, Clone, Copy)]
pub struct RateLimitConfig {
    /// Maximum burst of requests.
    pub burst: u32,
    /// Sustained requests per second refill rate.
    pub per_second: f64,
    /// Maximum request frame size in bytes.
    pub max_request_bytes: usize,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            burst: 60,
            per_second: 10.0,
            max_request_bytes: 1024 * 1024,
        }
    }
}

struct Bucket {
    tokens: f64,
    refilled_at: OffsetDateTime,
}

pub struct RateLimiter {
    config: RateLimitConfig,
    buckets: Mutex<HashMap<(String, String), Bucket>>,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub fn max_request_bytes(&self) -> usize {
        self.config.max_request_bytes
    }

    /// Consumes one token; `false` means the caller is over its limit.
    #[must_use]
    pub fn check(&self, tenant: &str, principal: &str, now: OffsetDateTime) -> bool {
        let mut buckets = self.buckets.lock().unwrap();
        let bucket = buckets
            .entry((tenant.to_string(), principal.to_string()))
            .or_insert(Bucket {
                tokens: f64::from(self.config.burst),
                refilled_at: now,
            });
        let elapsed = (now - bucket.refilled_at).as_seconds_f64().max(0.0);
        bucket.tokens =
            (bucket.tokens + elapsed * self.config.per_second).min(f64::from(self.config.burst));
        bucket.refilled_at = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_deterministic_and_keyed_per_principal() {
        let limiter = RateLimiter::new(RateLimitConfig {
            burst: 2,
            per_second: 1.0,
            max_request_bytes: 1024,
        });
        let t0 = OffsetDateTime::UNIX_EPOCH;
        assert!(limiter.check("tenant-a", "user-1", t0));
        assert!(limiter.check("tenant-a", "user-1", t0));
        assert!(!limiter.check("tenant-a", "user-1", t0));
        // Another principal in the same tenant has its own bucket.
        assert!(limiter.check("tenant-a", "user-2", t0));
        // One second refills one token.
        let t1 = t0 + time::Duration::seconds(1);
        assert!(limiter.check("tenant-a", "user-1", t1));
        assert!(!limiter.check("tenant-a", "user-1", t1));
    }
}
