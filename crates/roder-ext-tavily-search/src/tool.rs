use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use roder_web_search::{
    RenderOptions, WebSearchRequest, canonical_web_search_schema, render_web_search_response,
};

use crate::client::{TavilySearchClient, TavilySearchConfig, tavily_request_id};

pub const TAVILY_SEARCH_TOOL_NAME: &str = "tavily_search";

#[derive(Debug)]
pub struct TavilySearchContributor {
    tool: Arc<TavilySearchTool>,
}

impl TavilySearchContributor {
    pub fn new(config: TavilySearchConfig) -> anyhow::Result<Self> {
        Ok(Self {
            tool: Arc::new(TavilySearchTool::new(config)?),
        })
    }
}

impl ToolContributor for TavilySearchContributor {
    fn id(&self) -> ToolProviderId {
        "tavily-search".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(self.tool.clone())
    }
}

#[derive(Debug, Clone)]
pub struct TavilySearchTool {
    client: TavilySearchClient,
    config: TavilySearchConfig,
}

impl TavilySearchTool {
    pub fn new(config: TavilySearchConfig) -> anyhow::Result<Self> {
        let client = TavilySearchClient::new(config.clone())?;
        Ok(Self { client, config })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for TavilySearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TAVILY_SEARCH_TOOL_NAME.to_string(),
            description:
                "Search the web with Tavily and return normalized answers, URLs, snippets, scores, and optional raw page content."
                    .to_string(),
            parameters: canonical_web_search_schema(),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let request: WebSearchRequest = serde_json::from_value(call.arguments)?;
        let response_format = request.response_format;
        let response = self.client.search(request).await?;
        let request_id = tavily_request_id(&response.raw);
        let provider_config = self.config.provider_config_with_request_id(request_id);
        let text = render_web_search_response(
            &response,
            RenderOptions::for_response_format(response_format),
        );
        let mut data = response.data(&provider_config);
        data["response_format"] = serde_json::json!(response_format.as_str());

        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text,
            data,
            is_error: false,
        })
    }
}
