//! The HTTP transport both model calls share.
//!
//! Each caller holds its own `reqwest::Client`, so every step reuses the
//! pooled connection instead of paying TLS setup again. It is deliberately not
//! a process-wide static: pooled connections belong to the runtime that opened
//! them, and a shared client breaks across `#[tokio::test]` runtimes.
//!
//! Jev diverges from upstream's three attempts on 429/529/503 here. The status
//! set, the backoff schedule and the `retry-after` handling follow fastbrowse
//! (MIT, `clients/validation.py`), reimplemented rather than ported.
//!
//! Only a failure the provider cannot have acted on is retried in full: a
//! refused connection, or a status saying the request was not served. An
//! attempt that timed out, lost its body, answered with something other than
//! JSON, or failed with a status the origin may have answered after running
//! it (500, 504, 524) may already have been run and billed, so it is
//! repeated at most once.
//! No retry starts that would outlast the run (see [`RUN_DEADLINE`]), so the
//! run reports the provider's failure rather than its own timeout.
//!
//! A reply sent as server-sent events (the Codex backend's Responses API
//! only streams) is read to its end and reduced to its final `response`
//! object (see [`sse`]), so callers see one JSON value either way.

mod sse;

use std::time::Duration;

use tokio::time::Instant;

use reqwest::header::HeaderMap;
use serde_json::Value;

use crate::engine::{JevStatus, JevStop};
use crate::usage::JevBilled;

/// Statuses that say the request was not served, so repeating it is free:
/// timed out waiting for it, rate limited, a gateway or Cloudflare's edge
/// that could not reach the origin (502, 520 to 523), or overloaded.
const SAFE_STATUSES: [u16; 9] = [408, 429, 502, 503, 520, 521, 522, 523, 529];
/// Statuses after which the origin may have run, and billed, the request: an
/// internal error mid-way (500), or a gateway or Cloudflare's edge that gave
/// up waiting for the origin's answer (504, 524). Sent again at most once,
/// like a reply lost in transit.
const RESEND_STATUSES: [u16; 3] = [500, 504, 524];

/// Room left before the run's deadline for the caller to report a failure.
const DEADLINE_RESERVE: Duration = Duration::from_secs(1);

tokio::task_local! {
    /// When the current run times out. `JevEngine::run` sets it, so a retry
    /// that cannot finish in time is not started.
    pub(crate) static RUN_DEADLINE: Instant;
}

/// When and how long to retry a model call.
#[derive(Debug, Clone)]
pub(crate) struct RetryPolicy {
    /// How long one attempt may take before it is abandoned and retried.
    pub(crate) attempt_timeout: Duration,
    /// The wait before each retry; one attempt more than there are delays.
    pub(crate) delays: Vec<Duration>,
    /// The longest a server's `retry-after` may hold the run.
    pub(crate) max_retry_after: Duration,
    /// How many times a request the provider may already have run is sent
    /// again (see the module docs).
    pub(crate) resend_limit: usize,
}

impl Default for RetryPolicy {
    /// About 22 s of backoff over six attempts of at most 15 s each. The model
    /// answers in about a second, so an attempt that old is stuck upstream.
    fn default() -> Self {
        Self {
            attempt_timeout: Duration::from_secs(15),
            delays: [500, 1500, 4000, 8000, 8000]
                .into_iter()
                .map(Duration::from_millis)
                .collect(),
            max_retry_after: Duration::from_secs(10),
            resend_limit: 1,
        }
    }
}

impl RetryPolicy {
    /// Three attempts and 2 s of backoff. The text helper's address is the
    /// user's own, so a failure there is more often a misconfiguration that
    /// no retry clears, and each fill should give up quickly.
    pub(crate) fn text_helper() -> Self {
        Self {
            delays: vec![Duration::from_millis(500), Duration::from_millis(1500)],
            ..Self::default()
        }
    }
}

/// Why a call produced no usable JSON. Callers turn each into their own
/// user-facing message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PostFailure {
    /// The request never completed: connect, timeout or a dropped body.
    Connection,
    /// The provider answered with an error status. A rejection (any 4xx
    /// but 401) carries its body, trimmed, since it says what the provider
    /// refused: the field, the model or the limit.
    Status(u16, Option<String>),
    /// The provider answered, but not with JSON.
    InvalidBody,
    /// The provider accepted the request, then reported inside its event
    /// stream that it failed, with its code and message.
    Failed(String),
}

impl PostFailure {
    /// The provider could not be reached or said it could not serve the
    /// request: the failures retries are for, so still failing after them
    /// means the provider is unavailable rather than the request being wrong.
    pub(crate) fn unavailable(&self) -> bool {
        match self {
            Self::Connection => true,
            Self::Status(status, _) => {
                SAFE_STATUSES.contains(status) || RESEND_STATUSES.contains(status)
            }
            Self::InvalidBody | Self::Failed(_) => false,
        }
    }

    /// End the run with `message`: `unavailable` for a failure the retries
    /// could not clear, `error` for any other.
    pub(crate) fn stop(&self, message: String) -> anyhow::Error {
        let status = match self.unavailable() {
            true => JevStatus::Unavailable,
            false => JevStatus::Error,
        };
        JevStop::new(status, message).into()
    }

    /// [`Self::stop`] for a decision call. A body that cannot be decoded is a
    /// reply the service gave (after the one resend), so it is marked
    /// unusable and the loop asks again, as for a reply that fails
    /// validation. What it was billed is not known, so its usage is empty,
    /// which the run's sums report as `unknown`, never as 0. Every other
    /// failure (no connection, a timeout, a refused key, billing or access)
    /// is no reply and ends the run as it was.
    pub(crate) fn stop_decision(&self, message: String) -> anyhow::Error {
        let stop = self.stop(message);
        match self {
            Self::InvalidBody => JevBilled::unusable(serde_json::json!({}), stop).into(),
            _ => stop,
        }
    }
}

/// Whether repeating a failed request can clear the failure, and at what
/// risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Retry {
    /// It fails the same way every time.
    Never,
    /// The provider did not run it, so repeating it is free.
    Safe,
    /// The provider may have run it already; repeating it may bill twice.
    Resend,
}

/// One attempt's failure, and whether repeating the request can clear it.
struct Attempt {
    failure: PostFailure,
    retry: Retry,
    retry_after: Option<Duration>,
}

/// A JSON POST client that owns its connection pool and retry policy.
pub(crate) struct JsonPoster {
    client: reqwest::Client,
    policy: RetryPolicy,
}

impl JsonPoster {
    pub(crate) fn new(policy: RetryPolicy) -> Self {
        Self {
            client: reqwest::Client::new(),
            policy,
        }
    }

    /// POST `body` and parse the reply, retrying transient failures (see the
    /// module docs for which, and how often).
    pub(crate) async fn post(
        &self,
        url: &str,
        key: &str,
        body: &Value,
    ) -> Result<Value, PostFailure> {
        self.post_with(url, key, &[], body).await
    }

    /// [`Self::post`] with extra request headers.
    pub(crate) async fn post_with(
        &self,
        url: &str,
        key: &str,
        headers: &[(&str, String)],
        body: &Value,
    ) -> Result<Value, PostFailure> {
        let deadline = RUN_DEADLINE
            .try_with(|deadline| deadline.checked_sub(DEADLINE_RESERVE).unwrap_or(*deadline))
            .ok();
        let mut retries = self.policy.delays.iter();
        let mut resends = 0;
        loop {
            // An attempt still waiting at the deadline fails as a timeout,
            // which the caller reports, rather than as the run's timeout.
            let timeout = deadline.map_or(self.policy.attempt_timeout, |deadline| {
                self.policy
                    .attempt_timeout
                    .min(deadline.saturating_duration_since(Instant::now()))
                    .max(Duration::from_millis(1))
            });
            let attempt = match self.attempt(url, key, headers, body, timeout).await {
                Ok(value) => return Ok(value),
                Err(attempt) => attempt,
            };
            let allowed = match attempt.retry {
                Retry::Never => false,
                Retry::Safe => true,
                Retry::Resend => {
                    resends += 1;
                    resends <= self.policy.resend_limit
                }
            };
            let delay = match retries.next() {
                Some(delay) if allowed => attempt.retry_after.unwrap_or(*delay),
                _ => return Err(attempt.failure),
            };
            // A retry has to leave time for an answer before the run ends.
            if deadline.is_some_and(|deadline| Instant::now() + delay >= deadline) {
                return Err(attempt.failure);
            }
            tokio::time::sleep(delay).await;
        }
    }

    async fn attempt(
        &self,
        url: &str,
        key: &str,
        headers: &[(&str, String)],
        body: &Value,
        timeout: Duration,
    ) -> Result<Value, Attempt> {
        let mut request = self.client.post(url).bearer_auth(key);
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        let response = request
            .json(body)
            .timeout(timeout)
            .send()
            .await
            .map_err(|error| Attempt {
                failure: PostFailure::Connection,
                retry: transient(&error),
                retry_after: None,
            })?;
        let status = response.status().as_u16();
        if status >= 400 {
            let retry_after = retry_after(response.headers(), self.policy.max_retry_after);
            // A 401's body says nothing the status does not.
            let detail = match status {
                400..=499 if status != 401 => response.text().await.ok().map(|body| trimmed(&body)),
                _ => None,
            };
            return Err(Attempt {
                failure: PostFailure::Status(status, detail),
                retry: if SAFE_STATUSES.contains(&status) {
                    Retry::Safe
                } else if RESEND_STATUSES.contains(&status) {
                    Retry::Resend
                } else {
                    Retry::Never
                },
                retry_after,
            });
        }
        let streamed = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
        // The provider has answered by now, so every failure from here on
        // may have been billed.
        let bytes = response.bytes().await.map_err(|error| Attempt {
            failure: PostFailure::Connection,
            retry: match transient(&error) {
                Retry::Never => Retry::Never,
                _ => Retry::Resend,
            },
            retry_after: None,
        })?;
        // The Codex backend streams without naming the content type, so
        // an event stream is also known by its first field.
        if streamed || sse::looks_like(&bytes) {
            return match sse::finish(&bytes) {
                sse::StreamEnd::Response(response) => Ok(response),
                sse::StreamEnd::Failed(message) => Err(Attempt {
                    failure: PostFailure::Failed(message),
                    retry: Retry::Never,
                    retry_after: None,
                }),
                // Cut off in transit, like a dropped body: one more try.
                sse::StreamEnd::Truncated => Err(Attempt {
                    failure: PostFailure::InvalidBody,
                    retry: Retry::Resend,
                    retry_after: None,
                }),
            };
        }
        // A body garbled in transit can clear, but a proxy's HTML page will
        // not, so it gets one more try.
        serde_json::from_slice(&bytes).map_err(|_| Attempt {
            failure: PostFailure::InvalidBody,
            retry: Retry::Resend,
            retry_after: None,
        })
    }
}

/// A rejection body on one line, short enough for an error message.
fn trimmed(body: &str) -> String {
    const LIMIT: usize = 300;
    let line = body.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(LIMIT) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line,
    }
}

/// Transport failures a repeat can clear. A failed connect never reached the
/// provider; a timeout, a dropped reply or a broken body may have. Anything
/// else, such as an invalid URL, fails the same way every time.
fn transient(error: &reqwest::Error) -> Retry {
    if error.is_connect() && !error.is_timeout() {
        Retry::Safe
    } else if error.is_timeout() || error.is_request() || error.is_body() || error.is_decode() {
        Retry::Resend
    } else {
        Retry::Never
    }
}

/// The server's own wait: `retry-after-ms`, else `retry-after` in seconds,
/// capped so an overloaded provider cannot hold the run past its budget. An
/// HTTP-date or unreadable value falls back to the policy's delay.
fn retry_after(headers: &HeaderMap, cap: Duration) -> Option<Duration> {
    for (name, scale) in [("retry-after-ms", 0.001), ("retry-after", 1.0)] {
        let Some(raw) = headers.get(name) else {
            continue;
        };
        // An unreadable value defers to the next header, not the policy.
        let Some(seconds) = raw
            .to_str()
            .ok()
            .and_then(|raw| raw.trim().parse::<f64>().ok())
            .map(|value| value * scale)
            .filter(|seconds| !seconds.is_nan())
        else {
            continue;
        };
        return Some(Duration::from_secs_f64(
            seconds.clamp(0.0, cap.as_secs_f64()),
        ));
    }
    None
}

#[cfg(test)]
pub(crate) mod tests;
