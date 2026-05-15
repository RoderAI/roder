use std::sync::Arc;

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
use roder_ext_tavily_search::{TavilySearchClient, TavilySearchConfig};
use roder_web_search::{
    RenderOptions, WebSearchProviderConfig, WebSearchRequest, canonical_web_search_schema,
    render_web_search_response,
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
}

impl WebSearchRouterProvider {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Firecrawl(_) => "firecrawl",
            Self::Perplexity(_) => "perplexity",
            Self::Tavily(_) => "tavily",
            Self::Parallel(_) => "parallel",
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
            required_capabilities: vec![],
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
        };
        Ok(tool_result(call, &response, &provider_config))
    }
}

fn tool_result(
    call: ToolCall,
    response: &roder_web_search::WebSearchResponse,
    provider_config: &WebSearchProviderConfig,
) -> ToolResult {
    ToolResult {
        id: call.id,
        name: call.name,
        text: render_web_search_response(response, RenderOptions::default()),
        data: response.data(provider_config),
        is_error: false,
    }
}
