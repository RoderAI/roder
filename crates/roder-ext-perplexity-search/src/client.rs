use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use roder_web_search::{
    Freshness, HttpRequestConfig, WebSearchHttpClient, WebSearchProviderConfig,
    WebSearchProviderKind, WebSearchRequest, WebSearchResponse, WebSearchResult, WebSearchUsage,
};
use serde_json::{Map, Value, json};

const DEFAULT_BASE_URL: &str = "https://api.perplexity.ai";
const DEFAULT_TIMEOUT_SECONDS: u64 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerplexitySearchConfig {
    pub api_key: String,
    pub base_url: String,
    pub timeout_seconds: u64,
    pub debug_raw_response: bool,
}

impl PerplexitySearchConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
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
            provider: WebSearchProviderKind::Perplexity,
            api_key_env: Some("PERPLEXITY_API_KEY".to_string()),
            base_url: self.base_url.trim_end_matches('/').to_string(),
            timeout_seconds: self.timeout_seconds,
            user_agent: "roder-ext-perplexity-search/0.1".to_string(),
            debug_raw_response: self.debug_raw_response,
            provider_request_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PerplexitySearchClient {
    config: PerplexitySearchConfig,
    http: WebSearchHttpClient,
}

impl PerplexitySearchClient {
    pub fn new(config: PerplexitySearchConfig) -> anyhow::Result<Self> {
        if config.api_key.trim().is_empty() {
            anyhow::bail!("Perplexity API key is required");
        }
        let provider_config = config.provider_config();
        let http = WebSearchHttpClient::new(HttpRequestConfig::from_provider(&provider_config))?;
        Ok(Self { config, http })
    }

    pub async fn search(&self, request: WebSearchRequest) -> anyhow::Result<WebSearchResponse> {
        let request = request.validate_and_normalize()?;
        let body = perplexity_request_body(&request)?;
        let raw = self
            .http
            .post_json(&self.search_url(), self.headers()?, &body)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "Perplexity search request failed: {}",
                    redact_api_key(&error.to_string(), &self.config.api_key)
                )
            })?;
        normalize_perplexity_response(&request, raw)
    }

    fn search_url(&self) -> String {
        format!("{}/search", self.config.base_url.trim_end_matches('/'))
    }

    fn headers(&self) -> anyhow::Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let value = format!("Bearer {}", self.config.api_key);
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&value)?);
        Ok(headers)
    }
}

pub fn perplexity_request_body(request: &WebSearchRequest) -> anyhow::Result<Value> {
    if !request.include_domains.is_empty() && !request.exclude_domains.is_empty() {
        anyhow::bail!(
            "Perplexity search_domain_filter supports either include_domains or exclude_domains, not both"
        );
    }

    let mut body = json!({
        "query": request.query,
        "max_results": request.max_results,
    });
    if !request.include_domains.is_empty() {
        body["search_domain_filter"] = json!(request.include_domains);
    } else if !request.exclude_domains.is_empty() {
        body["search_domain_filter"] = json!(
            request
                .exclude_domains
                .iter()
                .map(|domain| format!("-{domain}"))
                .collect::<Vec<_>>()
        );
    }
    if let Some(recency) = search_recency_filter(request.freshness) {
        body["search_recency_filter"] = json!(recency);
    }
    if let Some(country) = request.country.as_deref() {
        body["country"] = json!(country);
    }
    Ok(body)
}

pub fn normalize_perplexity_response(
    request: &WebSearchRequest,
    raw: Value,
) -> anyhow::Result<WebSearchResponse> {
    if raw.get("error").is_some() {
        let message = string_field(&raw, &["message"]).unwrap_or_else(|| {
            raw.get("error")
                .map(error_message)
                .unwrap_or_else(|| "Perplexity search failed".to_string())
        });
        anyhow::bail!("{message}");
    }

    let mut results = Vec::new();
    collect_results(&raw, &mut results);
    if results.is_empty() {
        anyhow::bail!("Perplexity search response did not contain any usable results");
    }

    Ok(WebSearchResponse {
        provider: "perplexity".to_string(),
        query: string_field(&raw, &["query"]).unwrap_or_else(|| request.query.clone()),
        results,
        answer: None,
        usage: usage_from_raw(&raw),
        warnings: warnings_from_raw(&raw),
        raw,
    })
}

pub fn perplexity_request_id(raw: &Value) -> Option<String> {
    string_field(
        raw,
        &["request_id", "requestId", "search_id", "searchId", "id"],
    )
}

fn collect_results(value: &Value, results: &mut Vec<WebSearchResult>) {
    if let Some(array) = value.as_array() {
        for item in array {
            if let Some(result) = result_from_value(item) {
                results.push(result);
            }
        }
        return;
    }

    for key in [
        "results",
        "search_results",
        "searchResults",
        "web_results",
        "webResults",
        "data",
    ] {
        if let Some(array) = value.get(key).and_then(Value::as_array) {
            for item in array {
                if let Some(result) = result_from_value(item) {
                    results.push(result);
                }
            }
        }
    }
}

fn result_from_value(value: &Value) -> Option<WebSearchResult> {
    let url = string_field(value, &["url", "link"])?;
    let title = string_field(value, &["title", "name"]);
    let snippet = string_field(
        value,
        &["snippet", "description", "summary", "text", "content"],
    );
    let published_at = string_field(
        value,
        &[
            "date",
            "published_at",
            "publishedAt",
            "published_date",
            "publishedDate",
        ],
    );
    let score = number_field(value, &["score", "relevance", "rank_score", "rankScore"]);
    let source = string_field(value, &["source", "site_name", "siteName"]);

    Some(WebSearchResult {
        title,
        url,
        snippet,
        content: None,
        published_at,
        score,
        source,
        metadata: metadata_without_core_fields(value),
    })
}

fn usage_from_raw(raw: &Value) -> Option<WebSearchUsage> {
    let usage = raw.get("usage").or_else(|| raw.get("usage_info"))?;
    Some(WebSearchUsage {
        requests: number_to_u32(
            usage
                .get("searches")
                .or_else(|| usage.get("requests"))
                .or_else(|| usage.get("num_searches")),
        ),
        input_tokens: number_to_u32(
            usage
                .get("input_tokens")
                .or_else(|| usage.get("prompt_tokens"))
                .or_else(|| usage.get("inputTokens")),
        ),
        output_tokens: number_to_u32(
            usage
                .get("output_tokens")
                .or_else(|| usage.get("completion_tokens"))
                .or_else(|| usage.get("outputTokens")),
        ),
        cost_usd: usage
            .get("cost_usd")
            .or_else(|| usage.get("costUsd"))
            .and_then(Value::as_f64),
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
                | "title"
                | "name"
                | "snippet"
                | "description"
                | "summary"
                | "text"
                | "content"
                | "date"
                | "published_at"
                | "publishedAt"
                | "published_date"
                | "publishedDate"
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

fn search_recency_filter(freshness: Option<Freshness>) -> Option<&'static str> {
    match freshness {
        Some(Freshness::Day) => Some("day"),
        Some(Freshness::Week) => Some("week"),
        Some(Freshness::Month) => Some("month"),
        Some(Freshness::Year) => Some("year"),
        None => None,
    }
}

fn string_field(value: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str).and_then(non_empty))
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
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

fn error_message(value: &Value) -> String {
    match value {
        Value::Object(error) => error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.get("error").and_then(Value::as_str))
            .unwrap_or("Perplexity search failed")
            .to_string(),
        Value::String(message) => message.to_string(),
        _ => "Perplexity search failed".to_string(),
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

    #[test]
    fn maps_canonical_fields_to_raw_search_body() {
        let request = WebSearchRequest {
            query: "rust async".to_string(),
            max_results: 7,
            include_domains: vec!["example.com".to_string()],
            exclude_domains: Vec::new(),
            freshness: Some(Freshness::Week),
            country: Some("US".to_string()),
            include_content: true,
        };

        let body = perplexity_request_body(&request).unwrap();

        assert_eq!(body["query"], "rust async");
        assert_eq!(body["max_results"], 7);
        assert_eq!(body["search_domain_filter"], json!(["example.com"]));
        assert_eq!(body["search_recency_filter"], "week");
        assert_eq!(body["country"], "US");
        assert!(body.get("include_content").is_none());
    }

    #[test]
    fn exclude_domains_are_mapped_to_negative_domain_filter() {
        let mut request = WebSearchRequest::new("blocked");
        request.exclude_domains = vec!["spam.test".to_string()];

        let body = perplexity_request_body(&request).unwrap();

        assert_eq!(body["search_domain_filter"], json!(["-spam.test"]));
    }

    #[test]
    fn include_and_exclude_domains_cannot_be_mixed() {
        let mut request = WebSearchRequest::new("mixed");
        request.include_domains = vec!["example.com".to_string()];
        request.exclude_domains = vec!["spam.test".to_string()];

        let error = perplexity_request_body(&request).unwrap_err().to_string();

        assert!(error.contains("either include_domains or exclude_domains"));
    }

    #[test]
    fn normalizes_ranked_search_response() {
        let request = WebSearchRequest::new("ai search")
            .validate_and_normalize()
            .unwrap();
        let response = normalize_perplexity_response(
            &request,
            json!({
                "request_id": "req_123",
                "results": [{
                    "url": "https://example.com/web",
                    "title": "Web",
                    "snippet": "Web result",
                    "date": "2026-05-14",
                    "last_updated": "2026-05-15",
                    "score": 0.9,
                    "citations": ["https://example.com/source"]
                }],
                "usage": {
                    "searches": 1,
                    "input_tokens": 12,
                    "output_tokens": 34
                },
                "warnings": ["partial"]
            }),
        )
        .unwrap();

        assert_eq!(response.provider, "perplexity");
        assert!(response.answer.is_none());
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].snippet.as_deref(), Some("Web result"));
        assert_eq!(
            response.results[0].published_at.as_deref(),
            Some("2026-05-14")
        );
        assert_eq!(response.results[0].metadata["last_updated"], "2026-05-15");
        assert_eq!(
            response.results[0].metadata["citations"],
            json!(["https://example.com/source"])
        );
        assert_eq!(response.usage.unwrap().input_tokens, Some(12));
        assert_eq!(
            perplexity_request_id(&response.raw).as_deref(),
            Some("req_123")
        );
    }
}
