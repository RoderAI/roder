use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension, ToolProviderId,
};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use roder_ext_firecrawl_search::{FirecrawlSearchClient, FirecrawlSearchConfig};
use roder_ext_parallel_search::{
    ParallelSearchClient, ParallelSearchConfig, ParallelSearchOptions,
};
use roder_ext_perplexity_search::{PerplexitySearchClient, PerplexitySearchConfig};
use roder_ext_synthetic_search::{SyntheticSearchClient, SyntheticSearchConfig};
use roder_ext_tavily_search::{TavilySearchClient, TavilySearchConfig};
use roder_web_search::{
    RenderOptions, ResponseFormat, WebSearchProviderConfig, WebSearchRequest,
    canonical_web_search_schema, render_web_search_response,
};
use semver::Version;

pub const WEB_SEARCH_TOOL_NAME: &str = "web_search";
const DEFAULT_MAX_RESULTS: u8 = 5;
const MAX_RESULTS_LIMIT: u8 = 20;

#[derive(Debug, Clone)]
pub enum WebSearchRouterProvider {
    Firecrawl(FirecrawlSearchConfig),
    Perplexity(PerplexitySearchConfig),
    Tavily(TavilySearchConfig),
    Parallel(ParallelSearchConfig),
    Synthetic(SyntheticSearchConfig),
}

impl WebSearchRouterProvider {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Firecrawl(_) => "firecrawl",
            Self::Perplexity(_) => "perplexity",
            Self::Tavily(_) => "tavily",
            Self::Parallel(_) => "parallel",
            Self::Synthetic(_) => "synthetic",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WebSearchRouterConfig {
    pub provider: WebSearchRouterProvider,
    pub max_results: u8,
}

impl WebSearchRouterConfig {
    pub fn new(provider: WebSearchRouterProvider) -> Self {
        Self {
            provider,
            max_results: DEFAULT_MAX_RESULTS,
        }
    }

    pub fn with_max_results(mut self, max_results: u8) -> Self {
        self.max_results = max_results.clamp(1, MAX_RESULTS_LIMIT);
        self
    }
}

pub struct WebSearchRouterExtension {
    config: WebSearchRouterConfig,
}

impl WebSearchRouterExtension {
    pub fn new(config: WebSearchRouterConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for WebSearchRouterExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-web-search".to_string(),
            name: "Web Search Router".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(format!(
                "Canonical web_search router backed by {}",
                self.config.provider.id()
            )),
            provides: vec![ProvidedService::ToolProvider("web-search".to_string())],
            required_capabilities: vec![CapabilityRequest::new("network.web")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(WebSearchRouterContributor::new(
            self.config.clone(),
        )?));
        Ok(())
    }
}

#[derive(Debug)]
struct WebSearchRouterContributor {
    tool: Arc<WebSearchRouterTool>,
}

impl WebSearchRouterContributor {
    fn new(config: WebSearchRouterConfig) -> anyhow::Result<Self> {
        Ok(Self {
            tool: Arc::new(WebSearchRouterTool::new(config)?),
        })
    }
}

impl ToolContributor for WebSearchRouterContributor {
    fn id(&self) -> ToolProviderId {
        "web-search".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(self.tool.clone())
    }
}

#[derive(Debug, Clone)]
struct WebSearchRouterTool {
    backend: WebSearchBackend,
    provider_id: &'static str,
    max_results: u8,
}

impl WebSearchRouterTool {
    fn new(config: WebSearchRouterConfig) -> anyhow::Result<Self> {
        let max_results = config.max_results.clamp(1, MAX_RESULTS_LIMIT);
        let provider_id = config.provider.id();
        let backend = match config.provider {
            WebSearchRouterProvider::Firecrawl(config) => {
                WebSearchBackend::Firecrawl(FirecrawlSearchClient::new(config.clone())?, config)
            }
            WebSearchRouterProvider::Perplexity(config) => {
                WebSearchBackend::Perplexity(PerplexitySearchClient::new(config.clone())?, config)
            }
            WebSearchRouterProvider::Tavily(config) => {
                WebSearchBackend::Tavily(TavilySearchClient::new(config.clone())?, config)
            }
            WebSearchRouterProvider::Parallel(config) => {
                WebSearchBackend::Parallel(ParallelSearchClient::new(config.clone())?, config)
            }
            WebSearchRouterProvider::Synthetic(config) => {
                WebSearchBackend::Synthetic(SyntheticSearchClient::new(config.clone())?, config)
            }
        };
        Ok(Self {
            backend,
            provider_id,
            max_results,
        })
    }
}

#[derive(Debug, Clone)]
enum WebSearchBackend {
    Firecrawl(FirecrawlSearchClient, FirecrawlSearchConfig),
    Perplexity(PerplexitySearchClient, PerplexitySearchConfig),
    Tavily(TavilySearchClient, TavilySearchConfig),
    Parallel(ParallelSearchClient, ParallelSearchConfig),
    Synthetic(SyntheticSearchClient, SyntheticSearchConfig),
}

#[async_trait::async_trait]
impl ToolExecutor for WebSearchRouterTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: WEB_SEARCH_TOOL_NAME.to_string(),
            description: format!(
                "Search the web with the configured {} provider and return normalized URLs, snippets, answer text, and usage metadata.",
                self.provider_id
            ),
            parameters: canonical_web_search_schema(),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let mut request: WebSearchRequest = serde_json::from_value(call.arguments.clone())?;
        if call.arguments.get("max_results").is_none() {
            request.max_results = self.max_results;
        }
        let response_format = request.response_format;

        let (response, provider_config) = match &self.backend {
            WebSearchBackend::Firecrawl(client, config) => {
                let response = client.search(request).await?;
                (response, config.provider_config())
            }
            WebSearchBackend::Perplexity(client, config) => {
                let response = client.search(request).await?;
                let request_id =
                    roder_ext_perplexity_search::client::perplexity_request_id(&response.raw);
                (response, config.provider_config_with_request_id(request_id))
            }
            WebSearchBackend::Tavily(client, config) => {
                let response = client.search(request).await?;
                let request_id = roder_ext_tavily_search::client::tavily_request_id(&response.raw);
                (response, config.provider_config_with_request_id(request_id))
            }
            WebSearchBackend::Parallel(client, config) => {
                let options = ParallelSearchOptions::from_tool_arguments(&call.arguments);
                let response = client.search(request, options).await?;
                let request_id =
                    roder_ext_parallel_search::client::parallel_request_id(&response.raw);
                (response, config.provider_config_with_request_id(request_id))
            }
            WebSearchBackend::Synthetic(client, config) => {
                let response = client.search(request).await?;
                let request_id =
                    roder_ext_synthetic_search::client::synthetic_request_id(&response.raw);
                (response, config.provider_config_with_request_id(request_id))
            }
        };
        Ok(tool_result(
            call,
            &response,
            &provider_config,
            response_format,
        ))
    }
}

fn tool_result(
    call: ToolCall,
    response: &roder_web_search::WebSearchResponse,
    provider_config: &WebSearchProviderConfig,
    response_format: ResponseFormat,
) -> ToolResult {
    let mut data = response.data(provider_config);
    data["response_format"] = serde_json::json!(response_format.as_str());
    ToolResult {
        id: call.id,
        name: call.name,
        text: render_web_search_response(
            response,
            RenderOptions::for_response_format(response_format),
        ),
        data,
        is_error: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::policy_mode::PolicyMode;
    use roder_api::tools::{ToolExecutionContext, ToolExecutor};
    use roder_ext_synthetic_search::SyntheticSearchConfig;
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    #[test]
    fn response_format_controls_web_search_result_text() {
        let mut response = roder_web_search::testing::sample_response();
        response.answer = Some("answer ".repeat(1_200));
        response.results[0].snippet = Some("snippet ".repeat(800));
        let config = WebSearchProviderConfig::new(
            roder_web_search::WebSearchProviderKind::Parallel,
            "https://api.example",
        );
        let call = ToolCall {
            id: "call-web".to_string(),
            name: WEB_SEARCH_TOOL_NAME.to_string(),
            raw_arguments: "{}".to_string(),
            arguments: serde_json::json!({}),
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
        };

        let concise = tool_result(call.clone(), &response, &config, ResponseFormat::Concise);
        let detailed = tool_result(call, &response, &config, ResponseFormat::Detailed);

        assert_eq!(concise.data["response_format"], "concise");
        assert_eq!(detailed.data["response_format"], "detailed");
        assert!(concise.text.len() < detailed.text.len());
        assert!(concise.text.contains("..."));
    }

    #[tokio::test]
    async fn router_routes_to_synthetic_backend() {
        let server = SyntheticMockServer::start(vec![SyntheticMockResponse::json(
            200,
            json!({
                "request_id": "req-router",
                "results": [{
                    "url": "https://example.com/roder",
                    "title": "Roder",
                    "text": "Roder result"
                }]
            }),
        )])
        .await;
        let tool = WebSearchRouterTool::new(WebSearchRouterConfig {
            provider: WebSearchRouterProvider::Synthetic(
                SyntheticSearchConfig::new("secret-test-key").with_base_url(server.base_url()),
            ),
            max_results: 3,
        })
        .unwrap();
        let call = ToolCall {
            id: "call-router".to_string(),
            name: WEB_SEARCH_TOOL_NAME.to_string(),
            raw_arguments: "{}".to_string(),
            arguments: json!({ "query": "roder" }),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
        };

        let result = tool
            .execute(
                ToolExecutionContext::new("thread-1", "turn-1", PolicyMode::Default),
                call,
            )
            .await
            .unwrap();

        assert_eq!(result.name, "web_search");
        assert!(result.text.contains("https://example.com/roder"));
        assert_eq!(result.data["provider"], "synthetic");
        assert_eq!(result.data["provider_request_id"], "req-router");
    }

    fn _ensure_request_type_in_scope() {
        // Anchor WebSearchRequest/Freshness types so they remain used by the
        // router surface even if future tests stop referencing them directly.
        let _ = std::mem::size_of::<roder_web_search::WebSearchRequest>();
        let _ = roder_web_search::Freshness::Day;
    }

    #[derive(Debug, Clone)]
    struct SyntheticMockResponse {
        status: u16,
        body: String,
    }

    impl SyntheticMockResponse {
        fn json(status: u16, body: Value) -> Self {
            Self {
                status,
                body: body.to_string(),
            }
        }
    }

    #[derive(Debug, Clone)]
    struct SyntheticCapturedRequest {
        #[allow(dead_code)]
        headers: Vec<(String, String)>,
        #[allow(dead_code)]
        body: String,
    }

    #[derive(Debug)]
    struct SyntheticMockServer {
        address: std::net::SocketAddr,
        #[allow(dead_code)]
        requests: std::sync::Arc<Mutex<Vec<SyntheticCapturedRequest>>>,
    }

    impl SyntheticMockServer {
        async fn start(responses: Vec<SyntheticMockResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let responses = std::sync::Arc::new(Mutex::new(std::collections::VecDeque::from(responses)));
            let requests = std::sync::Arc::new(Mutex::new(Vec::new()));
            let server_responses = responses.clone();
            let server_requests = requests.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let responses = server_responses.clone();
                    let requests = server_requests.clone();
                    tokio::spawn(async move {
                        let mut buffer = vec![0; 8192];
                        let mut read = 0;
                        loop {
                            let n = stream.read(&mut buffer[read..]).await.unwrap();
                            if n == 0 {
                                return;
                            }
                            read += n;
                            if find_header_end(&buffer[..read]).is_some() {
                                break;
                            }
                            if read == buffer.len() {
                                buffer.resize(buffer.len() * 2, 0);
                            }
                        }
                        let header_end = find_header_end(&buffer[..read]).unwrap();
                        let headers_text =
                            String::from_utf8_lossy(&buffer[..header_end]).to_string();
                        let content_length = parse_content_length(&headers_text);
                        while read < header_end + 4 + content_length {
                            if read == buffer.len() {
                                buffer.resize(buffer.len() * 2, 0);
                            }
                            let n = stream.read(&mut buffer[read..]).await.unwrap();
                            if n == 0 {
                                break;
                            }
                            read += n;
                        }
                        let body_start = header_end + 4;
                        let body = String::from_utf8_lossy(
                            &buffer[body_start..(body_start + content_length).min(read)],
                        )
                        .to_string();
                        requests.lock().await.push(SyntheticCapturedRequest {
                            headers: parse_headers(&headers_text),
                            body,
                        });
                        let response = responses.lock().await.pop_front().unwrap_or_else(|| {
                            SyntheticMockResponse::json(500, json!({"message":"no response"}))
                        });
                        let reason = if response.status == 200 { "OK" } else { "Error" };
                        let raw = format!(
                            "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                            response.status,
                            reason,
                            response.body.len(),
                            response.body
                        );
                        stream.write_all(raw.as_bytes()).await.unwrap();
                    });
                }
            });
            Self { address, requests }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.address)
        }
    }

    fn find_header_end(buffer: &[u8]) -> Option<usize> {
        buffer.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn parse_content_length(headers: &str) -> usize {
        headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())
                    .flatten()
            })
            .unwrap_or_default()
    }

    fn parse_headers(headers: &str) -> Vec<(String, String)> {
        headers
            .lines()
            .skip(1)
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.to_string(), value.trim().to_string()))
            })
            .collect()
    }
}
