use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use roder_web_search::{
    HttpRequestConfig, WebSearchHttpClient, WebSearchProviderConfig, WebSearchProviderKind,
    WebSearchRequest, WebSearchResponse, WebSearchResult, WebSearchUsage,
};
use serde_json::{Map, Value, json};

const DEFAULT_BASE_URL: &str = "https://api.synthetic.new";
const DEFAULT_TIMEOUT_SECONDS: u64 = 20;
const DEFAULT_SEARCH_PATH: &str = "/v2/search";
const MAX_TEXT_LENGTH: u32 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticSearchConfig {
    pub api_key: String,
    pub base_url: String,
    pub timeout_seconds: u64,
    pub max_text_length: u32,
    pub debug_raw_response: bool,
}

impl SyntheticSearchConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            max_text_length: MAX_TEXT_LENGTH,
            debug_raw_response: false,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    pub fn with_timeout_seconds(mut self, timeout_seconds: u64) -> Self {
        self.timeout_seconds = timeout_seconds;
        self
    }

    pub fn with_max_text_length(mut self, max_text_length: u32) -> Self {
        self.max_text_length = max_text_length;
        self
    }

    pub fn with_debug_raw_response(mut self, debug_raw_response: bool) -> Self {
        self.debug_raw_response = debug_raw_response;
        self
    }

    pub fn provider_config(&self) -> WebSearchProviderConfig {
        self.provider_config_with_request_id(None)
    }

    pub fn provider_config_with_request_id(
        &self,
        provider_request_id: Option<String>,
    ) -> WebSearchProviderConfig {
        WebSearchProviderConfig {
            provider: WebSearchProviderKind::Synthetic,
            api_key_env: Some("SYNTHETIC_API_KEY".to_string()),
            base_url: self.base_url.trim_end_matches('/').to_string(),
            timeout_seconds: self.timeout_seconds,
            user_agent: "roder-ext-synthetic-search/0.1".to_string(),
            debug_raw_response: self.debug_raw_response,
            provider_request_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyntheticSearchClient {
    config: SyntheticSearchConfig,
    http: WebSearchHttpClient,
}

impl SyntheticSearchClient {
    pub fn new(config: SyntheticSearchConfig) -> anyhow::Result<Self> {
        if config.api_key.trim().is_empty() {
            anyhow::bail!("Synthetic API key is required");
        }
        let provider_config = config.provider_config();
        let http = WebSearchHttpClient::new(HttpRequestConfig::from_provider(&provider_config))?;
        Ok(Self { config, http })
    }

    pub async fn search(&self, request: WebSearchRequest) -> anyhow::Result<WebSearchResponse> {
        let request = request.validate_and_normalize()?;
        let body = synthetic_request_body(&request, &self.config);
        let raw = self
            .http
            .post_json(&self.search_url(), self.headers()?, &body)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "Synthetic search request failed: {}",
                    redact_api_key(&error.to_string(), &self.config.api_key)
                )
            })?;
        normalize_synthetic_response(&request, raw)
    }

    fn search_url(&self) -> String {
        format!(
            "{}{}",
            self.config.base_url.trim_end_matches('/'),
            DEFAULT_SEARCH_PATH
        )
    }

    fn headers(&self) -> anyhow::Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let value = format!("Bearer {}", self.config.api_key);
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&value)?);
        Ok(headers)
    }
}

pub fn synthetic_request_body(request: &WebSearchRequest, config: &SyntheticSearchConfig) -> Value {
    let mut body = json!({
        "query": request.query,
        "max_results": request.max_results,
        "max_tokens_per_page": config.max_text_length,
    });
    if !request.include_domains.is_empty() {
        body["include_domains"] = json!(request.include_domains);
    }
    if !request.exclude_domains.is_empty() {
        body["exclude_domains"] = json!(request.exclude_domains);
    }
    body
}

pub fn normalize_synthetic_response(
    request: &WebSearchRequest,
    raw: Value,
) -> anyhow::Result<WebSearchResponse> {
    if raw.get("error").is_some() {
        let message = error_message(&raw)
            .unwrap_or_else(|| "Synthetic search failed".to_string());
        anyhow::bail!("{message}");
    }

    let results = raw
        .get("results")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(result_from_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if results.is_empty() {
        anyhow::bail!("Synthetic search response did not contain any usable results");
    }

    Ok(WebSearchResponse {
        provider: "synthetic".to_string(),
        query: string_field(&raw, &["query"]).unwrap_or_else(|| request.query.clone()),
        results,
        answer: None,
        usage: usage_from_raw(&raw),
        warnings: warnings_from_raw(&raw),
        raw,
    })
}

pub fn synthetic_request_id(raw: &Value) -> Option<String> {
    string_field(
        raw,
        &[
            "request_id",
            "requestId",
            "search_id",
            "searchId",
            "id",
            "run_id",
        ],
    )
}

fn result_from_value(value: &Value) -> Option<WebSearchResult> {
    let url = string_field(value, &["url", "link", "source_url", "sourceUrl"])?;
    let title = string_field(value, &["title", "name", "headline"]);
    let snippet = string_field(value, &["text", "snippet", "excerpt", "summary", "content"]);
    let content = string_field(value, &["raw_content", "rawContent", "markdown"]);
    let published_at = string_field(
        value,
        &["published", "published_at", "publishedAt", "date"],
    );
    let score = number_field(value, &["score", "relevance", "rank_score", "rankScore"]);
    let source = string_field(value, &["source", "site_name", "siteName"]);

    Some(WebSearchResult {
        title,
        url,
        snippet,
        content,
        published_at,
        score,
        source,
        metadata: metadata_without_core_fields(value),
    })
}

fn usage_from_raw(raw: &Value) -> Option<WebSearchUsage> {
    let usage = raw
        .get("usage")
        .or_else(|| raw.get("usage_info"))
        .or_else(|| raw.get("usageInfo"))?;
    Some(WebSearchUsage {
        requests: number_to_u32(usage.get("requests")),
        input_tokens: number_to_u32(
            usage
                .get("input_tokens")
                .or_else(|| usage.get("inputTokens")),
        ),
        output_tokens: number_to_u32(
            usage
                .get("output_tokens")
                .or_else(|| usage.get("outputTokens")),
        ),
        cost_usd: usage.get("cost_usd").and_then(Value::as_f64),
        provider_metadata: usage.clone(),
    })
}

fn warnings_from_raw(raw: &Value) -> Vec<String> {
    raw.get("warnings")
        .and_then(Value::as_array)
        .map(|warnings| {
            warnings
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn metadata_without_core_fields(value: &Value) -> Value {
    let Some(object) = value.as_object() else {
        return json!({});
    };
    let mut metadata = Map::new();
    for (key, value) in object {
        if !matches!(
            key.as_str(),
            "url"
                | "link"
                | "source_url"
                | "sourceUrl"
                | "title"
                | "name"
                | "headline"
                | "text"
                | "snippet"
                | "excerpt"
                | "summary"
                | "content"
                | "raw_content"
                | "rawContent"
                | "markdown"
                | "published"
                | "published_at"
                | "publishedAt"
                | "date"
                | "score"
                | "relevance"
                | "rank_score"
                | "rankScore"
                | "source"
                | "site_name"
                | "siteName"
        ) {
            metadata.insert(key.clone(), value.clone());
        }
    }
    Value::Object(metadata)
}

fn string_field(value: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        value
            .get(*name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn number_field(value: &Value, names: &[&str]) -> Option<f64> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_f64))
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn error_message(raw: &Value) -> Option<String> {
    match raw.get("error")? {
        Value::String(message) => {
            let message = message.trim();
            (!message.is_empty()).then(|| message.to_string())
        }
        Value::Object(error) => string_field(&Value::Object(error.clone()), &["message", "error"]),
        _ => None,
    }
}

fn redact_api_key(input: &str, api_key: &str) -> String {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return input.to_string();
    }
    input.replace(api_key, "[redacted]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_web_search::{Freshness, ResponseFormat};

    #[test]
    fn maps_canonical_request_fields_to_synthetic_body() {
        let request = WebSearchRequest {
            query: "rust async".to_string(),
            max_results: 7,
            include_domains: vec!["example.com".to_string()],
            exclude_domains: vec!["spam.test".to_string()],
            freshness: Some(Freshness::Week),
            country: Some("US".to_string()),
            include_content: true,
            response_format: ResponseFormat::Concise,
        };
        let config = SyntheticSearchConfig::new("key");

        let body = synthetic_request_body(&request, &config);

        assert_eq!(body["query"], "rust async");
        assert_eq!(body["max_results"], 7);
        assert_eq!(body["max_tokens_per_page"], MAX_TEXT_LENGTH);
        assert_eq!(body["include_domains"], json!(["example.com"]));
        assert_eq!(body["exclude_domains"], json!(["spam.test"]));
        assert!(body.get("freshness").is_none());
        assert!(body.get("country").is_none());
    }

    #[test]
    fn normalizes_results_and_request_id() {
        let request = WebSearchRequest::new("rust");
        let raw = json!({
            "request_id": "req-1",
            "results": [
                {
                    "url": "https://example.com/roder",
                    "title": "Roder",
                    "text": "Hello Roder",
                    "published": "2026-06-22",
                    "score": 0.91
                }
            ]
        });

        let response = normalize_synthetic_response(&request, raw).unwrap();

        assert_eq!(response.provider, "synthetic");
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].url, "https://example.com/roder");
        assert_eq!(response.results[0].title.as_deref(), Some("Roder"));
        assert_eq!(response.results[0].snippet.as_deref(), Some("Hello Roder"));
        assert_eq!(
            response.results[0].published_at.as_deref(),
            Some("2026-06-22")
        );
        assert_eq!(response.results[0].score, Some(0.91));
        assert_eq!(synthetic_request_id(&response.raw).as_deref(), Some("req-1"));
    }

    #[test]
    fn rejects_empty_results() {
        let request = WebSearchRequest::new("rust");
        let err = normalize_synthetic_response(&request, json!({ "results": [] }))
            .unwrap_err()
            .to_string();

        assert!(err.contains("did not contain any usable results"));
    }

    #[test]
    fn surfaces_error_messages_without_secret() {
        let request = WebSearchRequest::new("rust");
        let raw = json!({ "error": "bad secret-test-key" });
        let err = normalize_synthetic_response(&request, raw)
            .unwrap_err()
            .to_string();

        assert!(err.contains("bad secret-test-key"));
    }

    #[test]
    fn redacts_api_key_in_errors() {
        let redacted = redact_api_key("bad secret-test-key", "secret-test-key");

        assert_eq!(redacted, "bad [redacted]");
        assert!(!redacted.contains("secret-test-key"));
    }
}
