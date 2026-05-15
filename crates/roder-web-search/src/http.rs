use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, RETRY_AFTER, USER_AGENT};
use serde::Deserialize;
use serde_json::Value;

use crate::types::WebSearchProviderConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryAfter {
    Seconds(u64),
    Date(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequestConfig {
    pub timeout: Duration,
    pub user_agent: String,
    pub max_retries: u8,
}

impl HttpRequestConfig {
    pub fn from_provider(config: &WebSearchProviderConfig) -> Self {
        Self {
            timeout: Duration::from_secs(config.timeout_seconds),
            user_agent: config.user_agent.clone(),
            max_retries: 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WebSearchHttpClient {
    client: reqwest::Client,
    config: HttpRequestConfig,
}

impl WebSearchHttpClient {
    pub fn new(config: HttpRequestConfig) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .user_agent(config.user_agent.clone())
            .build()?;
        Ok(Self { client, config })
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn user_agent(&self) -> &str {
        &self.config.user_agent
    }

    pub async fn post_json(
        &self,
        url: &str,
        headers: HeaderMap,
        body: &Value,
    ) -> Result<Value, RedactedHttpError> {
        let mut attempt = 0;
        loop {
            let response = self
                .client
                .post(url)
                .headers(headers.clone())
                .json(body)
                .send()
                .await
                .map_err(|source| RedactedHttpError::transport(url, &headers, source))?;
            if response.status().is_success() {
                return response
                    .json::<Value>()
                    .await
                    .map_err(|source| RedactedHttpError::transport(url, &headers, source));
            }

            let retry_after = parse_retry_after(response.headers());
            let retryable =
                response.status().as_u16() == 429 || response.status().is_server_error();
            let error = RedactedHttpError::from_response(url, &headers, response).await;
            if !retryable || attempt >= self.config.max_retries {
                return Err(error);
            }
            attempt += 1;
            if let Some(RetryAfter::Seconds(seconds)) = retry_after {
                tokio::time::sleep(Duration::from_secs(seconds.min(2))).await;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedHttpError {
    pub url: String,
    pub status: Option<u16>,
    pub retry_after: Option<RetryAfter>,
    pub body: Option<HttpErrorBody>,
    pub headers: Vec<(String, String)>,
    pub message: String,
}

impl RedactedHttpError {
    fn transport(url: &str, headers: &HeaderMap, source: reqwest::Error) -> Self {
        Self {
            url: url.to_string(),
            status: None,
            retry_after: None,
            body: None,
            headers: redact_sensitive_headers(headers),
            message: source.to_string(),
        }
    }

    async fn from_response(url: &str, headers: &HeaderMap, response: reqwest::Response) -> Self {
        let status = response.status();
        let retry_after = parse_retry_after(response.headers());
        let text = response.text().await.unwrap_or_default();
        let body = decode_json_error(&text).or_else(|| {
            (!text.trim().is_empty()).then_some(HttpErrorBody {
                code: None,
                message: text,
            })
        });
        let message = body
            .as_ref()
            .map(|body| body.message.clone())
            .unwrap_or_else(|| {
                status
                    .canonical_reason()
                    .unwrap_or("HTTP error")
                    .to_string()
            });
        Self {
            url: url.to_string(),
            status: Some(status.as_u16()),
            retry_after,
            body,
            headers: redact_sensitive_headers(headers),
            message,
        }
    }
}

impl std::fmt::Display for RedactedHttpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(status) => write!(
                formatter,
                "web search HTTP error {status}: {}",
                self.message
            ),
            None => write!(
                formatter,
                "web search HTTP transport error: {}",
                self.message
            ),
        }
    }
}

impl std::error::Error for RedactedHttpError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpErrorBody {
    pub code: Option<String>,
    pub message: String,
}

pub fn decode_json_error(text: &str) -> Option<HttpErrorBody> {
    #[derive(Deserialize)]
    struct ErrorEnvelope {
        error: Option<Value>,
        message: Option<String>,
        code: Option<String>,
    }

    let envelope: ErrorEnvelope = serde_json::from_str(text).ok()?;
    match envelope.error {
        Some(Value::Object(error)) => {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.get("error").and_then(Value::as_str))
                .unwrap_or("provider returned an error")
                .to_string();
            let code = error
                .get("code")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            Some(HttpErrorBody { code, message })
        }
        Some(Value::String(message)) => Some(HttpErrorBody {
            code: envelope.code,
            message,
        }),
        _ => envelope.message.map(|message| HttpErrorBody {
            code: envelope.code,
            message,
        }),
    }
}

pub fn redact_sensitive_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let name_text = name.as_str().to_string();
            let value_text = if is_sensitive_header(name) {
                "[redacted]".to_string()
            } else {
                value.to_str().unwrap_or("[non-utf8]").to_string()
            };
            (name_text, value_text)
        })
        .collect()
}

pub fn user_agent_header(value: &str) -> anyhow::Result<(HeaderName, HeaderValue)> {
    Ok((USER_AGENT, HeaderValue::from_str(value)?))
}

pub fn parse_retry_after(headers: &HeaderMap) -> Option<RetryAfter> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    value
        .parse::<u64>()
        .map(RetryAfter::Seconds)
        .ok()
        .or_else(|| Some(RetryAfter::Date(value.to_string())))
}

fn is_sensitive_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str().to_ascii_lowercase().as_str(),
        "authorization" | "x-api-key" | "api-key" | "cookie" | "set-cookie"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{AUTHORIZATION, HeaderMap};

    #[test]
    fn redacts_sensitive_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer secret"));
        headers.insert("x-api-key", HeaderValue::from_static("secret"));
        headers.insert(USER_AGENT, HeaderValue::from_static("roder-test"));

        let redacted = redact_sensitive_headers(&headers);
        assert!(redacted.contains(&("authorization".to_string(), "[redacted]".to_string())));
        assert!(redacted.contains(&("x-api-key".to_string(), "[redacted]".to_string())));
        assert!(redacted.contains(&("user-agent".to_string(), "roder-test".to_string())));
    }

    #[test]
    fn decodes_common_json_error_shapes() {
        let body = decode_json_error(r#"{"error":{"message":"bad key","code":"auth"}}"#).unwrap();
        assert_eq!(body.message, "bad key");
        assert_eq!(body.code.as_deref(), Some("auth"));

        let body = decode_json_error(r#"{"message":"rate limited","code":"429"}"#).unwrap();
        assert_eq!(body.message, "rate limited");
        assert_eq!(body.code.as_deref(), Some("429"));
    }

    #[test]
    fn parses_retry_after_seconds_and_dates() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("3"));
        assert_eq!(parse_retry_after(&headers), Some(RetryAfter::Seconds(3)));

        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        assert_eq!(
            parse_retry_after(&headers),
            Some(RetryAfter::Date(
                "Wed, 21 Oct 2015 07:28:00 GMT".to_string()
            ))
        );
    }
}
