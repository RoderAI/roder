use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const DEFAULT_MAX_RESULTS: u8 = 5;
pub const MAX_RESULTS_LIMIT: u8 = 20;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchProviderKind {
    Firecrawl,
    Perplexity,
    Tavily,
    Parallel,
    Synthetic,
    Custom,
}

impl WebSearchProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Firecrawl => "firecrawl",
            Self::Perplexity => "perplexity",
            Self::Tavily => "tavily",
            Self::Parallel => "parallel",
            Self::Synthetic => "synthetic",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Day,
    Week,
    Month,
    Year,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    #[default]
    Concise,
    Detailed,
}

impl ResponseFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Concise => "concise",
            Self::Detailed => "detailed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchRequest {
    pub query: String,
    #[serde(default = "default_max_results")]
    pub max_results: u8,
    #[serde(default)]
    pub include_domains: Vec<String>,
    #[serde(default)]
    pub exclude_domains: Vec<String>,
    pub freshness: Option<Freshness>,
    pub country: Option<String>,
    #[serde(default)]
    pub include_content: bool,
    #[serde(default)]
    pub response_format: ResponseFormat,
}

impl WebSearchRequest {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            max_results: DEFAULT_MAX_RESULTS,
            include_domains: Vec::new(),
            exclude_domains: Vec::new(),
            freshness: None,
            country: None,
            include_content: false,
            response_format: ResponseFormat::default(),
        }
    }

    pub fn validate_and_normalize(mut self) -> anyhow::Result<Self> {
        self.query = self.query.trim().to_string();
        if self.query.is_empty() {
            anyhow::bail!("web_search query is required");
        }
        if !(1..=MAX_RESULTS_LIMIT).contains(&self.max_results) {
            anyhow::bail!(
                "web_search max_results must be between 1 and {}",
                MAX_RESULTS_LIMIT
            );
        }
        self.include_domains = normalize_domain_list(&self.include_domains)?;
        self.exclude_domains = normalize_domain_list(&self.exclude_domains)?;
        if let Some(country) = self.country.as_mut() {
            *country = country.trim().to_ascii_uppercase();
            if country.is_empty() {
                self.country = None;
            }
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebSearchResponse {
    pub provider: String,
    pub query: String,
    pub results: Vec<WebSearchResult>,
    pub answer: Option<String>,
    pub usage: Option<WebSearchUsage>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub raw: Value,
}

impl WebSearchResponse {
    pub fn data(&self, config: &WebSearchProviderConfig) -> Value {
        let mut value = json!({
            "provider": self.provider,
            "provider_request_id": config.provider_request_id,
            "query": self.query,
            "results": self.results,
            "answer": self.answer,
            "usage": self.usage,
            "warnings": self.warnings,
        });
        if config.debug_raw_response {
            value["raw"] = self.raw.clone();
        }
        value
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebSearchResult {
    pub title: Option<String>,
    pub url: String,
    pub snippet: Option<String>,
    pub content: Option<String>,
    pub published_at: Option<String>,
    pub score: Option<f64>,
    pub source: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebSearchUsage {
    pub requests: Option<u32>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cost_usd: Option<f64>,
    pub provider_metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchProviderConfig {
    pub provider: WebSearchProviderKind,
    pub api_key_env: Option<String>,
    pub base_url: String,
    pub timeout_seconds: u64,
    pub user_agent: String,
    #[serde(default)]
    pub debug_raw_response: bool,
    pub provider_request_id: Option<String>,
}

impl WebSearchProviderConfig {
    pub fn new(provider: WebSearchProviderKind, base_url: impl Into<String>) -> Self {
        Self {
            provider,
            api_key_env: None,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            timeout_seconds: 20,
            user_agent: "roder-web-search/0.1".to_string(),
            debug_raw_response: false,
            provider_request_id: None,
        }
    }
}

pub fn canonical_web_search_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": {
                "type": "string",
                "description": "Concrete entity, technology, person, company, error, or claim to investigate."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RESULTS_LIMIT,
                "default": DEFAULT_MAX_RESULTS
            },
            "include_domains": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Domain names to prefer, without paths or URL schemes."
            },
            "exclude_domains": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Domain names to exclude, without paths or URL schemes."
            },
            "freshness": {
                "type": "string",
                "enum": ["day", "week", "month", "year"]
            },
            "country": {
                "type": "string",
                "description": "Optional provider-specific country or location preference."
            },
            "include_content": {
                "type": "boolean",
                "default": false
            },
            "response_format": {
                "type": "string",
                "enum": ["concise", "detailed"],
                "default": "concise",
                "description": "concise keeps answer and result snippets compact; detailed allows larger answer and snippet text."
            }
        },
        "required": ["query"]
    })
}

fn default_max_results() -> u8 {
    DEFAULT_MAX_RESULTS
}

fn normalize_domain_list(domains: &[String]) -> anyhow::Result<Vec<String>> {
    let mut normalized = Vec::new();
    for domain in domains {
        let domain = normalize_domain(domain)?;
        if !normalized.contains(&domain) {
            normalized.push(domain);
        }
    }
    Ok(normalized)
}

fn normalize_domain(input: &str) -> anyhow::Result<String> {
    let mut domain = input.trim().to_ascii_lowercase();
    if domain.is_empty() {
        anyhow::bail!("web_search domains must not be empty");
    }
    if let Some((_, rest)) = domain.split_once("://") {
        domain = rest.to_string();
    }
    if let Some((host, _)) = domain.split_once('/') {
        domain = host.to_string();
    }
    if let Some((host, _)) = domain.split_once('?') {
        domain = host.to_string();
    }
    if let Some((host, _)) = domain.split_once('#') {
        domain = host.to_string();
    }
    domain = domain
        .trim_start_matches("www.")
        .trim_end_matches('.')
        .to_string();
    if domain.is_empty()
        || domain.contains('@')
        || domain.contains(':')
        || domain.contains(char::is_whitespace)
        || !domain.contains('.')
    {
        anyhow::bail!("web_search domain must be a domain name: {input}");
    }
    Ok(domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_request_bounds_and_query() {
        let err = WebSearchRequest::new(" ")
            .validate_and_normalize()
            .unwrap_err();
        assert!(err.to_string().contains("query is required"));

        let mut request = WebSearchRequest::new("rust reqwest timeout");
        request.max_results = 21;
        let err = request.validate_and_normalize().unwrap_err();
        assert!(err.to_string().contains("max_results"));
    }

    #[test]
    fn normalizes_domain_lists() {
        let request = WebSearchRequest {
            query: "roder".to_string(),
            max_results: 5,
            include_domains: vec![
                " HTTPS://WWW.Example.COM/docs?q=1 ".to_string(),
                "example.com".to_string(),
            ],
            exclude_domains: vec!["Sub.Example.ORG.".to_string()],
            freshness: Some(Freshness::Week),
            country: Some(" us ".to_string()),
            include_content: true,
            response_format: ResponseFormat::Concise,
        }
        .validate_and_normalize()
        .unwrap();

        assert_eq!(request.include_domains, vec!["example.com"]);
        assert_eq!(request.exclude_domains, vec!["sub.example.org"]);
        assert_eq!(request.country.as_deref(), Some("US"));
    }

    #[test]
    fn rejects_non_domain_filters() {
        let mut request = WebSearchRequest::new("roder");
        request.include_domains = vec!["localhost".to_string()];
        assert!(request.validate_and_normalize().is_err());
    }

    #[test]
    fn schema_matches_canonical_contract() {
        let schema = canonical_web_search_schema();
        assert_eq!(schema["required"], json!(["query"]));
        assert_eq!(schema["properties"]["max_results"]["maximum"], json!(20));
        assert_eq!(
            schema["properties"]["freshness"]["enum"],
            json!(["day", "week", "month", "year"])
        );
        assert_eq!(
            schema["properties"]["response_format"]["enum"],
            json!(["concise", "detailed"])
        );
    }

    #[test]
    fn raw_response_is_gated_from_data() {
        let response = crate::testing::sample_response();
        let config = WebSearchProviderConfig::new(WebSearchProviderKind::Parallel, "https://api");

        let data = response.data(&config);
        assert!(data.get("raw").is_none());

        let mut debug_config = config;
        debug_config.debug_raw_response = true;
        let data = response.data(&debug_config);
        assert_eq!(data["raw"]["request_id"], json!("req_123"));
    }
}
