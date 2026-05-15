use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use roder_web_search::{
    HttpRequestConfig, WebSearchHttpClient, WebSearchProviderConfig, WebSearchProviderKind,
    WebSearchRequest, WebSearchResponse, WebSearchResult, WebSearchUsage,
};
use serde_json::{Map, Value, json};

const DEFAULT_BASE_URL: &str = "https://api.firecrawl.dev";
const DEFAULT_TIMEOUT_SECONDS: u64 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirecrawlSearchConfig {
    pub api_key: String,
    pub base_url: String,
    pub timeout_seconds: u64,
    pub debug_raw_response: bool,
}

impl FirecrawlSearchConfig {
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
        WebSearchProviderConfig {
            provider: WebSearchProviderKind::Firecrawl,
            api_key_env: Some("FIRECRAWL_API_KEY".to_string()),
            base_url: self.base_url.trim_end_matches('/').to_string(),
            timeout_seconds: self.timeout_seconds,
            user_agent: "roder-ext-firecrawl-search/0.1".to_string(),
            debug_raw_response: self.debug_raw_response,
            provider_request_id: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FirecrawlSearchClient {
    config: FirecrawlSearchConfig,
    http: WebSearchHttpClient,
}

impl FirecrawlSearchClient {
    pub fn new(config: FirecrawlSearchConfig) -> anyhow::Result<Self> {
        if config.api_key.trim().is_empty() {
            anyhow::bail!("Firecrawl API key is required");
        }
        let provider_config = config.provider_config();
        let http = WebSearchHttpClient::new(HttpRequestConfig::from_provider(&provider_config))?;
        Ok(Self { config, http })
    }

    pub async fn search(&self, request: WebSearchRequest) -> anyhow::Result<WebSearchResponse> {
        let request = request.validate_and_normalize()?;
        let body = firecrawl_request_body(&request, self.config.timeout_seconds);
        let raw = self
            .http
            .post_json(&self.search_url(), self.headers()?, &body)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "Firecrawl search request failed: {}",
                    redact_api_key(&error.to_string(), &self.config.api_key)
                )
            })?;
        normalize_firecrawl_response(&request, raw)
    }

    fn search_url(&self) -> String {
        format!("{}/v2/search", self.config.base_url.trim_end_matches('/'))
    }

    fn headers(&self) -> anyhow::Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let value = format!("Bearer {}", self.config.api_key);
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&value)?);
        Ok(headers)
    }
}

pub fn firecrawl_request_body(request: &WebSearchRequest, timeout_seconds: u64) -> Value {
    let mut body = json!({
        "query": request.query,
        "limit": request.max_results,
        "timeout": timeout_seconds.saturating_mul(1_000),
    });
    if !request.include_domains.is_empty() {
        body["includeDomains"] = json!(request.include_domains);
    }
    if !request.exclude_domains.is_empty() {
        body["excludeDomains"] = json!(request.exclude_domains);
    }
    if request.include_content {
        body["scrapeOptions"] = json!({
            "formats": ["markdown"],
        });
    }
    body
}

pub fn normalize_firecrawl_response(
    request: &WebSearchRequest,
    raw: Value,
) -> anyhow::Result<WebSearchResponse> {
    if raw.get("success").and_then(Value::as_bool) == Some(false) {
        let message = raw
            .get("error")
            .and_then(Value::as_str)
            .or_else(|| raw.get("message").and_then(Value::as_str))
            .unwrap_or("Firecrawl search failed");
        anyhow::bail!("{message}");
    }

    let mut results = Vec::new();
    collect_results(&raw, &mut results);
    if results.is_empty() {
        anyhow::bail!("Firecrawl search response did not contain any usable results");
    }

    Ok(WebSearchResponse {
        provider: "firecrawl".to_string(),
        query: request.query.clone(),
        results,
        answer: string_field(&raw, &["answer", "summary"]),
        usage: usage_from_raw(&raw),
        warnings: warnings_from_raw(&raw),
        raw,
    })
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
        "data", "results", "web", "search", "organic", "news", "images", "research",
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
    let url = string_field(
        value,
        &[
            "url",
            "link",
            "sourceUrl",
            "source_url",
            "pageUrl",
            "imageUrl",
            "image_url",
        ],
    )?;
    let title = string_field(value, &["title", "name", "headline"]);
    let snippet = string_field(
        value,
        &[
            "description",
            "snippet",
            "text",
            "excerpt",
            "summary",
            "caption",
        ],
    );
    let content = string_field(value, &["markdown", "content", "raw_content"]);
    let published_at = string_field(
        value,
        &[
            "publishedDate",
            "published_date",
            "publishedAt",
            "published_at",
            "date",
        ],
    );
    let score = number_field(value, &["score", "relevance", "rankScore"]);
    let source = string_field(value, &["source", "siteName", "site_name"]);

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
    let usage = raw.get("usage").or_else(|| raw.get("creditsUsed"))?;
    Some(WebSearchUsage {
        requests: number_to_u32(usage.get("requests")).or_else(|| number_to_u32(Some(usage))),
        input_tokens: number_to_u32(usage.get("input_tokens")),
        output_tokens: number_to_u32(usage.get("output_tokens")),
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
                | "sourceUrl"
                | "source_url"
                | "pageUrl"
                | "imageUrl"
                | "image_url"
                | "title"
                | "name"
                | "headline"
                | "description"
                | "snippet"
                | "text"
                | "excerpt"
                | "summary"
                | "caption"
                | "markdown"
                | "content"
                | "raw_content"
                | "publishedDate"
                | "published_date"
                | "publishedAt"
                | "published_at"
                | "date"
                | "score"
                | "relevance"
                | "rankScore"
                | "source"
                | "siteName"
                | "site_name"
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

fn redact_api_key(input: &str, api_key: &str) -> String {
    let key = api_key.trim();
    if key.is_empty() {
        return input.to_string();
    }
    input.replace(key, "[redacted]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_canonical_request_fields_to_firecrawl_body() {
        let request = WebSearchRequest {
            query: "rust async".to_string(),
            max_results: 7,
            include_domains: vec!["example.com".to_string()],
            exclude_domains: vec!["spam.test".to_string()],
            freshness: None,
            country: None,
            include_content: true,
        };

        let body = firecrawl_request_body(&request, 12);

        assert_eq!(body["query"], "rust async");
        assert_eq!(body["limit"], 7);
        assert_eq!(body["includeDomains"], json!(["example.com"]));
        assert_eq!(body["excludeDomains"], json!(["spam.test"]));
        assert_eq!(body["timeout"], 12_000);
        assert_eq!(body["scrapeOptions"]["formats"], json!(["markdown"]));
    }

    #[test]
    fn normalizes_mixed_firecrawl_result_shapes() {
        let request = WebSearchRequest::new("ai search")
            .validate_and_normalize()
            .unwrap();
        let response = normalize_firecrawl_response(
            &request,
            json!({
                "success": true,
                "data": [{
                    "url": "https://example.com/web",
                    "title": "Web",
                    "description": "Web result",
                    "markdown": "# Web",
                    "publishedDate": "2026-05-01",
                    "score": 0.9
                }],
                "images": [{
                    "imageUrl": "https://example.com/image.png",
                    "caption": "Image result"
                }],
                "news": [{
                    "link": "https://example.com/news",
                    "headline": "News",
                    "snippet": "News result",
                    "date": "2026-05-02"
                }],
                "research": [{
                    "sourceUrl": "https://example.com/paper",
                    "name": "Paper",
                    "summary": "Research result"
                }],
                "creditsUsed": 1,
                "warnings": ["partial"]
            }),
        )
        .unwrap();

        assert_eq!(response.provider, "firecrawl");
        assert_eq!(response.results.len(), 4);
        assert_eq!(response.results[0].content.as_deref(), Some("# Web"));
        assert!(
            response
                .results
                .iter()
                .any(|result| result.url == "https://example.com/image.png")
        );
        assert!(
            response
                .results
                .iter()
                .any(|result| result.published_at.as_deref() == Some("2026-05-02"))
        );
        assert_eq!(response.usage.unwrap().requests, Some(1));
        assert_eq!(response.warnings, vec!["partial"]);
    }
}
