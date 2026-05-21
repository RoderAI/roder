use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use roder_web_search::{
    Freshness, HttpRequestConfig, WebSearchHttpClient, WebSearchProviderConfig,
    WebSearchProviderKind, WebSearchRequest, WebSearchResponse, WebSearchResult, WebSearchUsage,
};
use serde_json::{Map, Value, json};

const DEFAULT_BASE_URL: &str = "https://api.tavily.com";
const DEFAULT_TIMEOUT_SECONDS: u64 = 20;
const PROJECT_HEADER: &str = "x-project-id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TavilySearchConfig {
    pub api_key: String,
    pub project_id: Option<String>,
    pub base_url: String,
    pub timeout_seconds: u64,
    pub search_depth: String,
    pub include_answer: bool,
    pub debug_raw_response: bool,
}

impl TavilySearchConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            project_id: None,
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            search_depth: "basic".to_string(),
            include_answer: true,
            debug_raw_response: false,
        }
    }

    pub fn with_project_id(mut self, project_id: impl Into<String>) -> Self {
        let project_id = project_id.into().trim().to_string();
        self.project_id = (!project_id.is_empty()).then_some(project_id);
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    pub fn with_timeout_seconds(mut self, timeout_seconds: u64) -> Self {
        self.timeout_seconds = timeout_seconds;
        self
    }

    pub fn with_search_depth(mut self, search_depth: impl Into<String>) -> Self {
        self.search_depth = search_depth.into();
        self
    }

    pub fn with_include_answer(mut self, include_answer: bool) -> Self {
        self.include_answer = include_answer;
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
            provider: WebSearchProviderKind::Tavily,
            api_key_env: Some("TAVILY_API_KEY".to_string()),
            base_url: self.base_url.trim_end_matches('/').to_string(),
            timeout_seconds: self.timeout_seconds,
            user_agent: "roder-ext-tavily-search/0.1".to_string(),
            debug_raw_response: self.debug_raw_response,
            provider_request_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TavilySearchClient {
    config: TavilySearchConfig,
    http: WebSearchHttpClient,
}

impl TavilySearchClient {
    pub fn new(config: TavilySearchConfig) -> anyhow::Result<Self> {
        if config.api_key.trim().is_empty() {
            anyhow::bail!("Tavily API key is required");
        }
        let provider_config = config.provider_config();
        let http = WebSearchHttpClient::new(HttpRequestConfig::from_provider(&provider_config))?;
        Ok(Self { config, http })
    }

    pub async fn search(&self, request: WebSearchRequest) -> anyhow::Result<WebSearchResponse> {
        let request = request.validate_and_normalize()?;
        let body = tavily_request_body(&request, &self.config);
        let raw = self
            .http
            .post_json(&self.search_url(), self.headers()?, &body)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "Tavily search request failed: {}",
                    redact_config_values(&error.to_string(), &self.config)
                )
            })?;
        normalize_tavily_response(&request, raw)
    }

    fn search_url(&self) -> String {
        format!("{}/search", self.config.base_url.trim_end_matches('/'))
    }

    fn headers(&self) -> anyhow::Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let value = format!("Bearer {}", self.config.api_key);
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&value)?);
        if let Some(project_id) = self.config.project_id.as_deref() {
            headers.insert(
                HeaderName::from_static(PROJECT_HEADER),
                HeaderValue::from_str(project_id)?,
            );
        }
        Ok(headers)
    }
}

pub fn tavily_request_body(request: &WebSearchRequest, config: &TavilySearchConfig) -> Value {
    let mut body = json!({
        "query": request.query,
        "max_results": request.max_results,
        "search_depth": normalized_search_depth(&config.search_depth),
        "include_answer": config.include_answer,
        "include_raw_content": request.include_content,
        "include_usage": true,
    });
    if let Some(time_range) = time_range_from_freshness(request.freshness) {
        body["time_range"] = json!(time_range);
    }
    if !request.include_domains.is_empty() {
        body["include_domains"] = json!(request.include_domains);
    }
    if !request.exclude_domains.is_empty() {
        body["exclude_domains"] = json!(request.exclude_domains);
    }
    if let Some(country) = request.country.as_deref() {
        body["country"] = json!(country);
    }
    body
}

pub fn normalize_tavily_response(
    request: &WebSearchRequest,
    raw: Value,
) -> anyhow::Result<WebSearchResponse> {
    if raw.get("error").is_some() {
        let message = string_field(&raw, &["error", "message"]).unwrap_or_else(|| {
            raw.get("error")
                .map(Value::to_string)
                .unwrap_or_else(|| "Tavily search failed".to_string())
        });
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
        anyhow::bail!("Tavily search response did not contain any usable results");
    }

    Ok(WebSearchResponse {
        provider: "tavily".to_string(),
        query: string_field(&raw, &["query"]).unwrap_or_else(|| request.query.clone()),
        results,
        answer: answer_from_raw(&raw),
        usage: usage_from_raw(&raw),
        warnings: warnings_from_raw(&raw),
        raw,
    })
}

pub fn tavily_request_id(raw: &Value) -> Option<String> {
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
    let url = string_field(value, &["url", "link"])?;
    let title = string_field(value, &["title", "name"]);
    let snippet = string_field(value, &["content", "snippet", "description"]);
    let content = string_field(value, &["raw_content", "rawContent"]);
    let published_at = string_field(value, &["published_date", "publishedDate", "date"]);
    let score = number_field(value, &["score", "relevance_score", "relevanceScore"]);
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

fn answer_from_raw(raw: &Value) -> Option<String> {
    match raw.get("answer") {
        Some(Value::String(answer)) => non_empty(answer),
        Some(Value::Object(answer)) => string_field(
            &Value::Object(answer.clone()),
            &["text", "answer", "content", "summary"],
        ),
        _ => string_field(raw, &["summary"]),
    }
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
                | "title"
                | "name"
                | "content"
                | "snippet"
                | "description"
                | "raw_content"
                | "rawContent"
                | "published_date"
                | "publishedDate"
                | "date"
                | "score"
                | "relevance_score"
                | "relevanceScore"
                | "source"
                | "site_name"
                | "siteName"
        ) {
            metadata.insert(key.clone(), value.clone());
        }
    }
    Value::Object(metadata)
}

fn time_range_from_freshness(freshness: Option<Freshness>) -> Option<&'static str> {
    match freshness {
        Some(Freshness::Day) => Some("day"),
        Some(Freshness::Week) => Some("week"),
        Some(Freshness::Month) => Some("month"),
        Some(Freshness::Year) => Some("year"),
        None => None,
    }
}

fn normalized_search_depth(search_depth: &str) -> &str {
    match search_depth.trim().to_ascii_lowercase().as_str() {
        "advanced" => "advanced",
        _ => "basic",
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

fn redact_config_values(input: &str, config: &TavilySearchConfig) -> String {
    let mut redacted = redact_value(input, &config.api_key);
    if let Some(project_id) = config.project_id.as_deref() {
        redacted = redact_value(&redacted, project_id);
    }
    redacted
}

fn redact_value(input: &str, value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return input.to_string();
    }
    input.replace(value, "[redacted]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_canonical_request_fields_to_tavily_body() {
        let request = WebSearchRequest {
            query: "rust async".to_string(),
            max_results: 7,
            include_domains: vec!["example.com".to_string()],
            exclude_domains: vec!["spam.test".to_string()],
            freshness: Some(Freshness::Week),
            country: Some("US".to_string()),
            include_content: true,
            response_format: roder_web_search::ResponseFormat::Concise,
        };
        let config = TavilySearchConfig::new("key").with_search_depth("advanced");

        let body = tavily_request_body(&request, &config);

        assert_eq!(body["query"], "rust async");
        assert_eq!(body["max_results"], 7);
        assert_eq!(body["search_depth"], "advanced");
        assert_eq!(body["time_range"], "week");
        assert_eq!(body["include_domains"], json!(["example.com"]));
        assert_eq!(body["exclude_domains"], json!(["spam.test"]));
        assert_eq!(body["country"], "US");
        assert_eq!(body["include_answer"], true);
        assert_eq!(body["include_raw_content"], true);
        assert_eq!(body["include_usage"], true);
    }

    #[test]
    fn default_search_depth_is_basic() {
        let request = WebSearchRequest::new("rust");
        let config = TavilySearchConfig::new("key");

        let body = tavily_request_body(&request, &config);

        assert_eq!(body["search_depth"], "basic");
    }

    #[test]
    fn normalizes_tavily_response() {
        let request = WebSearchRequest::new("ai search")
            .validate_and_normalize()
            .unwrap();
        let response = normalize_tavily_response(
            &request,
            json!({
                "request_id": "req_123",
                "query": "ai search",
                "answer": "Tavily answer",
                "results": [{
                    "url": "https://example.com/web",
                    "title": "Web",
                    "content": "Web result",
                    "raw_content": "# Web",
                    "score": 0.9,
                    "favicon": "https://example.com/favicon.ico",
                    "images": ["https://example.com/image.png"]
                }],
                "usage": { "credits": 2, "search_depth": "advanced" },
                "warnings": ["partial"]
            }),
        )
        .unwrap();

        assert_eq!(response.provider, "tavily");
        assert_eq!(response.answer.as_deref(), Some("Tavily answer"));
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].content.as_deref(), Some("# Web"));
        assert_eq!(response.results[0].score, Some(0.9));
        assert_eq!(
            response.results[0].metadata["favicon"],
            "https://example.com/favicon.ico"
        );
        assert_eq!(
            response.results[0].metadata["images"],
            json!(["https://example.com/image.png"])
        );
        assert_eq!(response.usage.unwrap().provider_metadata["credits"], 2);
        assert_eq!(tavily_request_id(&response.raw).as_deref(), Some("req_123"));
    }
}
