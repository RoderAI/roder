use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use roder_web_search::{
    HttpRequestConfig, WebSearchHttpClient, WebSearchProviderConfig, WebSearchProviderKind,
    WebSearchRequest, WebSearchResponse, WebSearchResult, WebSearchUsage,
};
use serde_json::{Map, Value, json};

const DEFAULT_BASE_URL: &str = "https://api.parallel.ai";
const DEFAULT_TIMEOUT_SECONDS: u64 = 20;
const API_KEY_HEADER: &str = "x-api-key";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelSearchConfig {
    pub api_key: String,
    pub base_url: String,
    pub timeout_seconds: u64,
    pub mode: String,
    pub debug_raw_response: bool,
}

impl ParallelSearchConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            mode: "advanced".to_string(),
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

    pub fn with_mode(mut self, mode: impl Into<String>) -> Self {
        self.mode = mode.into();
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
            provider: WebSearchProviderKind::Parallel,
            api_key_env: Some("PARALLEL_API_KEY".to_string()),
            base_url: self.base_url.trim_end_matches('/').to_string(),
            timeout_seconds: self.timeout_seconds,
            user_agent: "roder-ext-parallel-search/0.1".to_string(),
            debug_raw_response: self.debug_raw_response,
            provider_request_id,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParallelSearchOptions {
    pub search_queries: Vec<String>,
}

impl ParallelSearchOptions {
    pub fn from_tool_arguments(arguments: &Value) -> Self {
        let search_queries = arguments
            .get("search_queries")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Self { search_queries }
    }
}

#[derive(Debug, Clone)]
pub struct ParallelSearchClient {
    config: ParallelSearchConfig,
    http: WebSearchHttpClient,
}

impl ParallelSearchClient {
    pub fn new(config: ParallelSearchConfig) -> anyhow::Result<Self> {
        if config.api_key.trim().is_empty() {
            anyhow::bail!("Parallel API key is required");
        }
        let provider_config = config.provider_config();
        let http = WebSearchHttpClient::new(HttpRequestConfig::from_provider(&provider_config))?;
        Ok(Self { config, http })
    }

    pub async fn search(
        &self,
        request: WebSearchRequest,
        options: ParallelSearchOptions,
    ) -> anyhow::Result<WebSearchResponse> {
        let request = request.validate_and_normalize()?;
        let body = parallel_request_body(&request, &self.config, &options);
        let raw = self
            .http
            .post_json(&self.search_url(), self.headers()?, &body)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "Parallel search request failed: {}",
                    redact_api_key(&error.to_string(), &self.config.api_key)
                )
            })?;
        normalize_parallel_response(&request, raw)
    }

    fn search_url(&self) -> String {
        format!("{}/v1/search", self.config.base_url.trim_end_matches('/'))
    }

    fn headers(&self) -> anyhow::Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static(API_KEY_HEADER),
            HeaderValue::from_str(&self.config.api_key)?,
        );
        Ok(headers)
    }
}

pub fn parallel_request_body(
    request: &WebSearchRequest,
    config: &ParallelSearchConfig,
    options: &ParallelSearchOptions,
) -> Value {
    let search_queries = if options.search_queries.is_empty() {
        derive_search_queries(&request.query)
    } else {
        options.search_queries.clone()
    };
    let mut body = json!({
        "objective": request.query,
        "search_queries": search_queries,
        "max_results": request.max_results,
        "mode": normalized_mode(&config.mode),
    });
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

pub fn normalize_parallel_response(
    request: &WebSearchRequest,
    raw: Value,
) -> anyhow::Result<WebSearchResponse> {
    if raw.get("error").is_some() {
        let message = string_field(&raw, &["message", "error"]).unwrap_or_else(|| {
            raw.get("error")
                .map(Value::to_string)
                .unwrap_or_else(|| "Parallel search failed".to_string())
        });
        anyhow::bail!("{message}");
    }

    let results = result_arrays(&raw)
        .into_iter()
        .flatten()
        .filter_map(result_from_value)
        .collect::<Vec<_>>();
    if results.is_empty() {
        anyhow::bail!("Parallel search response did not contain any usable results");
    }

    Ok(WebSearchResponse {
        provider: "parallel".to_string(),
        query: request.query.clone(),
        results,
        answer: string_field(&raw, &["answer", "summary"]),
        usage: usage_from_raw(&raw),
        warnings: warnings_from_raw(&raw),
        raw,
    })
}

pub fn parallel_request_id(raw: &Value) -> Option<String> {
    string_field(
        raw,
        &["search_id", "searchId", "request_id", "requestId", "id"],
    )
}

pub fn derive_search_queries(query: &str) -> Vec<String> {
    let words = query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| word.len() > 2)
        .take(8)
        .collect::<Vec<_>>();
    if words.is_empty() {
        return vec![query.trim().to_string()];
    }

    let mut queries = vec![query.trim().to_string()];
    if words.len() >= 3 {
        queries.push(words.iter().take(4).copied().collect::<Vec<_>>().join(" "));
    }
    if words.len() >= 5 {
        queries.push(
            words
                .iter()
                .skip(2)
                .take(4)
                .copied()
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    dedupe_nonempty(queries).into_iter().take(3).collect()
}

fn result_arrays(raw: &Value) -> Vec<Vec<&Value>> {
    let mut arrays = Vec::new();
    for key in ["results", "search_results", "sources", "data"] {
        if let Some(items) = raw.get(key).and_then(Value::as_array) {
            arrays.push(items.iter().collect());
        }
    }
    if let Some(items) = raw.as_array() {
        arrays.push(items.iter().collect());
    }
    arrays
}

fn result_from_value(value: &Value) -> Option<WebSearchResult> {
    let url = string_field(value, &["url", "link", "source_url", "sourceUrl"])?;
    let title = string_field(value, &["title", "name", "headline"]);
    let snippet = string_field(value, &["snippet", "excerpt", "text", "summary"])
        .or_else(|| first_string_array_item(value, &["excerpts", "snippets"]));
    let content = string_field(value, &["content", "raw_content", "rawContent", "markdown"]);
    let published_at = string_field(
        value,
        &[
            "published_at",
            "publishedAt",
            "publish_date",
            "publishDate",
            "date",
        ],
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
                | "snippet"
                | "excerpt"
                | "text"
                | "summary"
                | "excerpts"
                | "snippets"
                | "content"
                | "raw_content"
                | "rawContent"
                | "markdown"
                | "published_at"
                | "publishedAt"
                | "publish_date"
                | "publishDate"
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

fn first_string_array_item(value: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        value
            .get(*name)
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find_map(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
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

fn normalized_mode(mode: &str) -> &str {
    match mode.trim().to_ascii_lowercase().as_str() {
        "basic" => "basic",
        _ => "advanced",
    }
}

fn dedupe_nonempty(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let value = value.trim().to_string();
        if !value.is_empty() && !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

fn redact_api_key(input: &str, api_key: &str) -> String {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return input.to_string();
    }
    input.replace(api_key, "[redacted]")
}
