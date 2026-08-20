use std::time::Duration;

const PROVIDER_REQUEST_ID_HEADERS: [&str; 4] = [
    "x-request-id",
    "request-id",
    "fireworks-request-id",
    "x-fireworks-request-id",
];
const MAX_CAUSE_CHARS: usize = 500;
const MAX_REQUEST_ID_CHARS: usize = 200;

pub(crate) struct ResponseStreamDiagnostics {
    idle_timeout: Duration,
    provider_request_id: Option<String>,
}

impl ResponseStreamDiagnostics {
    pub(crate) fn from_response(response: &reqwest::Response, idle_timeout: Duration) -> Self {
        Self {
            idle_timeout,
            provider_request_id: provider_request_id(response.headers()),
        }
    }

    pub(crate) fn read_error(&self, error: reqwest::Error) -> anyhow::Error {
        let kind = if error.is_timeout() {
            "timeout"
        } else if error.is_connect() {
            "connect"
        } else if error.is_body() {
            "body"
        } else if error.is_decode() {
            "decode"
        } else if error.is_request() {
            "request"
        } else {
            "unknown"
        };
        let cause = safe_cause(format!("{:#}", anyhow::Error::new(error)));
        let provider_request_id = self.provider_request_id.as_deref().unwrap_or("unavailable");

        anyhow::anyhow!(
            "Responses stream read failed: kind={kind}; idle_timeout_ms={}; provider_request_id={provider_request_id}; cause={cause}",
            self.idle_timeout.as_millis()
        )
    }
}

fn provider_request_id(headers: &reqwest::header::HeaderMap) -> Option<String> {
    PROVIDER_REQUEST_ID_HEADERS.iter().find_map(|name| {
        let value = headers.get(*name)?.to_str().ok()?;
        let valid = !value.is_empty()
            && value.chars().count() <= MAX_REQUEST_ID_CHARS
            && value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_.:".contains(character));
        valid.then(|| value.to_string())
    })
}

fn safe_cause(cause: String) -> String {
    let normalized = cause.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut excerpt = normalized.chars().take(MAX_CAUSE_CHARS).collect::<String>();
    if normalized.chars().count() > MAX_CAUSE_CHARS {
        excerpt.push_str(" ...");
    }
    excerpt
}

#[cfg(test)]
mod tests {
    use super::provider_request_id;
    use reqwest::header::{HeaderMap, HeaderValue};

    #[test]
    fn provider_request_id_accepts_log_safe_values_only() {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", HeaderValue::from_static("req_123.abc:456"));
        assert_eq!(
            provider_request_id(&headers).as_deref(),
            Some("req_123.abc:456")
        );

        headers.insert("x-request-id", HeaderValue::from_static("not safe"));
        assert_eq!(provider_request_id(&headers), None);
    }
}
