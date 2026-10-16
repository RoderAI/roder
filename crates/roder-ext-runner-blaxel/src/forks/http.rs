use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{BlaxelForkClient, ForkError, Outcome};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

impl BlaxelForkClient {
    /// Never retries a request. In particular, a lost mutation response must be
    /// reconciled using its pre-recorded name/ID rather than replayed blindly.
    pub(super) async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Option<T>, ForkError> {
        let mutation = method != Method::GET;
        let permits_absence = method == Method::GET || method == Method::DELETE;
        let failure = || {
            if mutation {
                ForkError::unknown("provider response unavailable")
            } else {
                ForkError::rejected("provider read unavailable")
            }
        };
        let mut request = self
            .http
            .request(method, format!("{}/{path}", self.base_url))
            .bearer_auth(self.token.as_str())
            .header("X-Blaxel-Workspace", &self.workspace)
            .header("Blaxel-Version", "2026-04-28");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await.map_err(|_| failure())?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND && permits_absence {
            return Ok(None);
        }
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            return Err(ForkError {
                outcome: if mutation
                    && (status.is_server_error()
                        || status == StatusCode::REQUEST_TIMEOUT
                        || status.is_redirection())
                {
                    Outcome::Unknown
                } else {
                    Outcome::Rejected
                },
                status: Some(status.as_u16()),
                retry_after,
                message: "provider request failed",
                fork_receipt: None,
            });
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(failure());
            }
            bytes.extend_from_slice(&chunk);
        }
        // DELETE often returns 204. Its caller requests serde_json::Value.
        if bytes.is_empty() && status == StatusCode::NO_CONTENT {
            return serde_json::from_value(Value::Null)
                .map(Some)
                .map_err(|_| failure());
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| failure())
    }
}
