//! Anonymous status transport after the source command API has stopped.
use std::time::Duration;

use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::{Instant, timeout_at};

use super::ForkError;

pub(super) const PORT: u16 = 4330;

#[path = "capture_channel/bootstrap.rs"]
mod bootstrap;
pub use bootstrap::{SourceApiBootstrapBinding, SourceApiBootstrapPermit};

/// Public routing identity to retain with the keyed preparation journal.
/// It contains no provider or application credential and grants no VM mutation.
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CaptureChannelPin {
    pub(super) workspace: String,
    pub(super) source: String,
    pub(super) external_id: String,
    pub(super) origin: String,
    pub(super) preview_name: String,
    pub(super) request_digest: String,
}

/// A bounded status client with its own credential-free connection pool.
/// Construction requires a fresh authenticated observation of the owned source
/// and its existing public preview. The caller owns preview creation and cleanup.
pub struct CaptureChannel {
    http: Client,
    pin: CaptureChannelPin,
    deadline: Instant,
}

/// Native operation state. Even `Complete` does not attest to a full VM seal,
/// authorize capture, permit source resume, or grant browser controls.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureChannelState {
    /// The native owner has not observed this exact operation.
    NotObserved,
    /// The native owner consumed the operation and has not finished it.
    Pending,
    /// The operation definitively failed and cannot be dispatched again.
    Failed,
    /// The exact neutral operation receipt was observed.
    Complete,
    /// Capture status was retired after admitted neutral API startup.
    Retired,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Status {
    ok: bool,
    schema_version: u8,
    request_digest: String,
    state: CaptureChannelState,
    receipt: Option<Value>,
    full_vm_seal: bool,
}

impl CaptureChannel {
    pub(super) fn new(pin: CaptureChannelPin, deadline: Instant) -> Result<Self, ForkError> {
        if Instant::now() >= deadline {
            return Err(ForkError::rejected("capture channel observation expired"));
        }
        let http = Client::builder()
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_zstd()
            .no_deflate()
            .user_agent("node")
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .pool_max_idle_per_host(0)
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| ForkError::rejected("cannot construct capture status client"))?;
        Ok(Self {
            http,
            pin,
            deadline,
        })
    }

    /// Retain this exact public pin before dispatching any preparation mutation.
    pub fn pin(&self) -> &CaptureChannelPin {
        &self.pin
    }

    /// Read only the keyed capture result within the caller's original deadline.
    pub async fn capture_status(
        &self,
        deadline: Instant,
    ) -> Result<CaptureChannelState, ForkError> {
        self.status(false, deadline).await
    }

    /// Read only the once-only neutral API result. This never starts an API.
    pub async fn bootstrap_status(
        &self,
        deadline: Instant,
    ) -> Result<CaptureChannelState, ForkError> {
        self.status(true, deadline).await
    }

    async fn status(
        &self,
        bootstrap: bool,
        deadline: Instant,
    ) -> Result<CaptureChannelState, ForkError> {
        let unavailable = || ForkError::rejected("capture channel status unavailable");
        let deadline = deadline.min(self.deadline);
        if Instant::now() >= deadline {
            return Err(unavailable());
        }
        let operation = if bootstrap {
            "bootstrap-status"
        } else {
            "capture-status"
        };
        let work = async {
            let mut response = self
                .http
                .get(format!(
                    "{}/{operation}/{}",
                    self.pin.origin, self.pin.request_digest
                ))
                .header(reqwest::header::ACCEPT, "application/json")
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .send()
                .await
                .map_err(|_| unavailable())?;
            if response.status() != reqwest::StatusCode::OK
                || response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    != Some("application/json")
                || response
                    .headers()
                    .contains_key(reqwest::header::CONTENT_ENCODING)
                || response
                    .headers()
                    .get(reqwest::header::CACHE_CONTROL)
                    .and_then(|value| value.to_str().ok())
                    != Some("no-store")
            {
                return Err(unavailable());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
                if bytes.len().saturating_add(chunk.len()) > 1024 {
                    return Err(unavailable());
                }
                bytes.extend_from_slice(&chunk);
            }
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
            if value.as_object().map(|v| v.len()) != Some(6)
                || serde_json::to_vec(&value).map_err(|_| unavailable())? != bytes
            {
                return Err(unavailable());
            }
            let status: Status = serde_json::from_value(value).map_err(|_| unavailable())?;
            let expected = if bootstrap {
                json!({"ok":true,"neutral_api_started":true,"full_vm_seal":false})
            } else {
                json!({"ok":true,"command_api_stopped":true,"parent_assets_removed":true,"full_vm_seal":false})
            };
            if !status.ok
                || status.schema_version != 1
                || status.request_digest != self.pin.request_digest
                || status.full_vm_seal
                || bootstrap && status.state == CaptureChannelState::Retired
                || if status.state == CaptureChannelState::Complete {
                    status.receipt.as_ref() != Some(&expected)
                } else {
                    status.receipt.is_some()
                }
            {
                return Err(unavailable());
            }
            if Instant::now() >= deadline {
                return Err(unavailable());
            }
            Ok(status.state)
        };
        timeout_at(deadline, work)
            .await
            .map_err(|_| unavailable())?
    }
}

pub(super) fn validate_digest(value: &str) -> Result<(), ForkError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ForkError::rejected("invalid capture request digest"));
    }
    Ok(())
}

pub(super) fn preview_origin(value: &str) -> Result<String, ForkError> {
    let refused = || ForkError::rejected("invalid capture preview URL");
    let url = Url::parse(value).map_err(|_| refused())?;
    let host = url.host_str().ok_or_else(refused)?;
    let mut labels = host.split('.').collect::<Vec<_>>();
    if labels.len() == 5 && ["us-was-1", "us-pdx-1", "eu-lon-1", "eu-fra-1"].contains(&labels[1]) {
        labels.remove(1);
    }
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || labels.len() != 4
        || labels[1..] != ["preview", "bl", "run"]
        || labels[0].is_empty()
        || labels[0].len() > 63
        || !labels[0]
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || labels[0].starts_with('-')
        || labels[0].ends_with('-')
    {
        return Err(refused());
    }
    Ok(url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests;
