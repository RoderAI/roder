use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use roder_web_search::{
    HttpRequestConfig, WebSearchHttpClient, WebSearchProviderConfig, WebSearchProviderKind,
    WebSearchRequest, WebSearchResponse, WebSearchResult, WebSearchUsage,
};
use serde_json::{Map, Value, json};

const DEFAULT_BASE_URL: &str = "https://api.parallel.ai";
const DEFAULT_TIMEOUT_SECONDS: u64 = 20;
const API_KEY_HEADER: &str = "x-api-key";
const MAX_EXTRACT_URLS: usize = 20;

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
        Self {
            search_queries: string_array_field(arguments, "search_queries"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelExtractRequest {
    pub urls: Vec<String>,
    pub objective: Option<String>,
    pub search_queries: Vec<String>,
    pub max_chars_total: Option<u32>,
    pub session_id: Option<String>,
    pub full_content: bool,
    pub max_chars_per_result: Option<u32>,
}

impl ParallelExtractRequest {
    pub fn from_tool_arguments(arguments: &Value) -> anyhow::Result<Self> {
        let mut urls = string_array_field(arguments, "urls");
        if urls.is_empty() {
            if let Some(url) = arguments
                .get("url")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                urls.push(url.to_string());
            }
        }
        urls = dedupe_nonempty(urls);
        if urls.is_empty() {
            anyhow::bail!("parallel_extract requires at least one url");
        }
        if urls.len() > MAX_EXTRACT_URLS {
            anyhow::bail!("parallel_extract supports at most {MAX_EXTRACT_URLS} urls");
        }
        for url in &urls {
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                anyhow::bail!("parallel_extract urls must start with http:// or https://");
            }
        }

        let objective = arguments
            .get("objective")
            .or_else(|| arguments.get("query"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let max_chars_total = arguments
            .get("max_chars_total")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        let max_chars_per_result = arguments
            .get("max_chars_per_result")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        let session_id = arguments
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let full_content = arguments
            .get("full_content")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || arguments
                .get("include_content")
                .and_then(Value::as_bool)
                .unwrap_or(false);

        Ok(Self {
            urls,
            objective,
            search_queries: string_array_field(arguments, "search_queries"),
            max_chars_total,
            session_id,
            full_content,
            max_chars_per_result,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelExtractResult {
    pub url: String,
    pub title: Option<String>,
    pub publish_date: Option<String>,
    pub excerpts: Vec<String>,
    pub full_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelExtractError {
    pub url: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelExtractResponse {
    pub provider: String,
    pub extract_id: Option<String>,
    pub session_id: Option<String>,
    pub results: Vec<ParallelExtractResult>,
    pub errors: Vec<ParallelExtractError>,
    pub warnings: Vec<String>,
    pub usage: Option<WebSearchUsage>,
    pub raw: Value,
}

impl ParallelExtractResponse {
    pub fn data(&self, config: &WebSearchProviderConfig) -> Value {
        let mut value = json!({
            "provider": self.provider,
            "provider_request_id": config.provider_request_id.clone().or_else(|| self.extract_id.clone()),
            "extract_id": self.extract_id,
            "session_id": self.session_id,
            "results": self.results.iter().map(|result| json!({
                "url": result.url,
                "title": result.title,
                "publish_date": result.publish_date,
                "excerpts": result.excerpts,
                "full_content": result.full_content,
            })).collect::<Vec<_>>(),
            "errors": self.errors.iter().map(|error| json!({
                "url": error.url,
                "message": error.message,
            })).collect::<Vec<_>>(),
            "warnings": self.warnings,
            "usage": self.usage,
        });
        if config.debug_raw_response {
            value["raw"] = self.raw.clone();
        }
        value
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

    pub async fn extract(
        &self,
        request: ParallelExtractRequest,
    ) -> anyhow::Result<ParallelExtractResponse> {
        let body = parallel_extract_request_body(&request);
        let raw = self
            .http
            .post_json(&self.extract_url(), self.headers()?, &body)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "Parallel extract request failed: {}",
                    redact_api_key(&error.to_string(), &self.config.api_key)
                )
            })?;
        normalize_parallel_extract_response(raw)
    }

    fn search_url(&self) -> String {
        format!("{}/v1/search", self.config.base_url.trim_end_matches('/'))
    }

    fn extract_url(&self) -> String {
        format!("{}/v1/extract", self.config.base_url.trim_end_matches('/'))
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
        "mode": normalized_mode(&config.mode),
    });

    // Parallel V1 nests result limits and domain filters under advanced_settings.
    // Top-level max_results / include_domains are rejected with 422.
    let mut advanced = Map::new();
    advanced.insert("max_results".to_string(), json!(request.max_results));

    let mut source_policy = Map::new();
    if !request.include_domains.is_empty() {
        source_policy.insert(
            "include_domains".to_string(),
            json!(request.include_domains),
        );
    }
    if !request.exclude_domains.is_empty() {
        source_policy.insert(
            "exclude_domains".to_string(),
            json!(request.exclude_domains),
        );
    }
    if !source_policy.is_empty() {
        advanced.insert("source_policy".to_string(), Value::Object(source_policy));
    }
    if let Some(country) = request
        .country
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // API field is ISO 3166-1 alpha-2 `location`, not `country`.
        advanced.insert(
            "location".to_string(),
            json!(country.to_ascii_lowercase()),
        );
    }
    body["advanced_settings"] = Value::Object(advanced);

    body
}

pub fn parallel_extract_request_body(request: &ParallelExtractRequest) -> Value {
    let mut body = json!({
        "urls": request.urls,
    });
    if let Some(objective) = request.objective.as_deref() {
        body["objective"] = json!(objective);
    }
    if !request.search_queries.is_empty() {
        body["search_queries"] = json!(request.search_queries);
    }
    if let Some(max_chars_total) = request.max_chars_total {
        body["max_chars_total"] = json!(max_chars_total);
    }
    if let Some(session_id) = request.session_id.as_deref() {
        body["session_id"] = json!(session_id);
    }

    let mut advanced = Map::new();
    if request.full_content {
        if let Some(max_chars_per_result) = request.max_chars_per_result {
            advanced.insert(
                "full_content".to_string(),
                json!({ "max_chars_per_result": max_chars_per_result }),
            );
        } else {
            advanced.insert("full_content".to_string(), json!(true));
        }
    } else if let Some(max_chars_per_result) = request.max_chars_per_result {
        advanced.insert(
            "excerpt_settings".to_string(),
            json!({ "max_chars_per_result": max_chars_per_result }),
        );
    }
    if !advanced.is_empty() {
        body["advanced_settings"] = Value::Object(advanced);
    }

    body
}

pub fn normalize_parallel_response(
    request: &WebSearchRequest,
    raw: Value,
) -> anyhow::Result<WebSearchResponse> {
    if let Some(message) = parallel_error_message(&raw) {
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

pub fn parallel_extract_request_id(raw: &Value) -> Option<String> {
    string_field(
        raw,
        &[
            "extract_id",
            "extractId",
            "request_id",
            "requestId",
            "id",
        ],
    )
}

pub fn normalize_parallel_extract_response(raw: Value) -> anyhow::Result<ParallelExtractResponse> {
    if let Some(message) = parallel_error_message(&raw) {
        anyhow::bail!("{message}");
    }

    let results = raw
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(extract_result_from_value)
        .collect::<Vec<_>>();
    let errors = extract_errors_from_raw(&raw);
    if results.is_empty() && errors.is_empty() {
        anyhow::bail!("Parallel extract response did not contain any usable results");
    }

    Ok(ParallelExtractResponse {
        provider: "parallel".to_string(),
        extract_id: parallel_extract_request_id(&raw),
        session_id: string_field(&raw, &["session_id", "sessionId"]),
        results,
        errors,
        warnings: warnings_from_raw(&raw),
        usage: usage_from_raw(&raw),
        raw,
    })
}

pub fn render_parallel_extract_response(response: &ParallelExtractResponse) -> String {
    let mut lines = Vec::new();
    if let Some(extract_id) = response.extract_id.as_deref() {
        lines.push(format!("Extract ID: {extract_id}"));
    }
    if let Some(session_id) = response.session_id.as_deref() {
        lines.push(format!("Session ID: {session_id}"));
    }
    if !response.results.is_empty() {
        lines.push(format!("Results ({})", response.results.len()));
        for (index, result) in response.results.iter().enumerate() {
            let title = result
                .title
                .as_deref()
                .filter(|value| !value.is_empty())
                .unwrap_or("(untitled)");
            lines.push(format!("{}. {title}", index + 1));
            lines.push(format!("   URL: {}", result.url));
            if let Some(publish_date) = result.publish_date.as_deref() {
                lines.push(format!("   Published: {publish_date}"));
            }
            if !result.excerpts.is_empty() {
                lines.push("   Excerpts:".to_string());
                for excerpt in &result.excerpts {
                    for excerpt_line in excerpt.lines() {
                        lines.push(format!("     {excerpt_line}"));
                    }
                    lines.push(String::new());
                }
            }
            if let Some(full_content) = result.full_content.as_deref() {
                lines.push("   Full content:".to_string());
                for content_line in full_content.lines() {
                    lines.push(format!("     {content_line}"));
                }
                lines.push(String::new());
            }
        }
    }
    if !response.errors.is_empty() {
        lines.push(format!("Errors ({})", response.errors.len()));
        for error in &response.errors {
            match error.url.as_deref() {
                Some(url) => lines.push(format!("- {url}: {}", error.message)),
                None => lines.push(format!("- {}", error.message)),
            }
        }
    }
    if !response.warnings.is_empty() {
        lines.push(format!("Warnings: {}", response.warnings.join("; ")));
    }
    lines.join("\n").trim().to_string()
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
        .or_else(|| joined_string_array_items(value, &["excerpts", "snippets"]));
    let content = string_field(
        value,
        &[
            "content",
            "raw_content",
            "rawContent",
            "markdown",
            "full_content",
            "fullContent",
        ],
    );
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

fn extract_result_from_value(value: &Value) -> Option<ParallelExtractResult> {
    let url = string_field(value, &["url", "link", "source_url", "sourceUrl"])?;
    let title = string_field(value, &["title", "name", "headline"]);
    let publish_date = string_field(
        value,
        &[
            "publish_date",
            "publishDate",
            "published_at",
            "publishedAt",
            "date",
        ],
    );
    let excerpts = value
        .get("excerpts")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let full_content = string_field(
        value,
        &[
            "full_content",
            "fullContent",
            "content",
            "markdown",
            "raw_content",
            "rawContent",
        ],
    );
    if excerpts.is_empty() && full_content.is_none() {
        return None;
    }
    Some(ParallelExtractResult {
        url,
        title,
        publish_date,
        excerpts,
        full_content,
    })
}

fn extract_errors_from_raw(raw: &Value) -> Vec<ParallelExtractError> {
    raw.get("errors")
        .and_then(Value::as_array)
        .map(|errors| {
            errors
                .iter()
                .filter_map(|error| match error {
                    Value::String(message) => {
                        let message = message.trim();
                        (!message.is_empty()).then(|| ParallelExtractError {
                            url: None,
                            message: message.to_string(),
                        })
                    }
                    Value::Object(_) => {
                        let message = string_field(error, &["message", "error", "detail"])
                            .unwrap_or_else(|| error.to_string());
                        Some(ParallelExtractError {
                            url: string_field(error, &["url", "link"]),
                            message,
                        })
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parallel_error_message(raw: &Value) -> Option<String> {
    if raw.get("error").is_none() && raw.get("type").and_then(Value::as_str) != Some("error") {
        return None;
    }
    raw.get("error")
        .and_then(|error| string_field(error, &["message"]))
        .or_else(|| string_field(raw, &["message"]))
        .or_else(|| {
            raw.get("error")
                .map(Value::to_string)
                .map(|value| value.trim_matches('"').to_string())
        })
        .or_else(|| Some("Parallel request failed".to_string()))
}

fn string_array_field(value: &Value, name: &str) -> Vec<String> {
    value
        .get(name)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn usage_from_raw(raw: &Value) -> Option<WebSearchUsage> {
    let usage = raw
        .get("usage")
        .or_else(|| raw.get("usage_info"))
        .or_else(|| raw.get("usageInfo"))?;

    // Parallel returns usage as [{ "name": "sku_...", "count": N }, ...].
    // Object shapes are still accepted for fixtures and forward-compat.
    if let Some(items) = usage.as_array() {
        let requests = items
            .iter()
            .filter_map(|item| number_to_u32(item.get("count")))
            .fold(0u32, u32::saturating_add);
        return Some(WebSearchUsage {
            requests: (requests > 0).then_some(requests),
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
            provider_metadata: usage.clone(),
        });
    }

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
                .filter_map(|warning| match warning {
                    Value::String(message) => {
                        let message = message.trim();
                        (!message.is_empty()).then(|| message.to_string())
                    }
                    Value::Object(object) => object
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|message| !message.is_empty())
                        .map(ToOwned::to_owned),
                    _ => None,
                })
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
                | "full_content"
                | "fullContent"
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

fn joined_string_array_items(value: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        let items = value.get(*name)?.as_array()?;
        let joined = items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        (!joined.is_empty()).then_some(joined)
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
        "turbo" | "fast" => "turbo",
        "basic" => "basic",
        // Legacy alias used by some configs/docs before turbo/basic/advanced.
        "one-shot" | "oneshot" | "one_shot" => "basic",
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
