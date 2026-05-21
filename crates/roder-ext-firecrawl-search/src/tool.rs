use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use roder_web_search::{
    RenderOptions, WebSearchRequest, canonical_web_search_schema, render_web_search_response,
};

use crate::client::{FirecrawlSearchClient, FirecrawlSearchConfig};

pub const FIRECRAWL_SEARCH_TOOL_NAME: &str = "firecrawl_search";

#[derive(Debug)]
pub struct FirecrawlSearchContributor {
    tool: Arc<FirecrawlSearchTool>,
}

impl FirecrawlSearchContributor {
    pub fn new(config: FirecrawlSearchConfig) -> anyhow::Result<Self> {
        Ok(Self {
            tool: Arc::new(FirecrawlSearchTool::new(config)?),
        })
    }
}

impl ToolContributor for FirecrawlSearchContributor {
    fn id(&self) -> ToolProviderId {
        "firecrawl-search".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(self.tool.clone())
    }
}

#[derive(Debug, Clone)]
pub struct FirecrawlSearchTool {
    client: FirecrawlSearchClient,
    config: FirecrawlSearchConfig,
}

impl FirecrawlSearchTool {
    pub fn new(config: FirecrawlSearchConfig) -> anyhow::Result<Self> {
        let client = FirecrawlSearchClient::new(config.clone())?;
        Ok(Self { client, config })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for FirecrawlSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: FIRECRAWL_SEARCH_TOOL_NAME.to_string(),
            description:
                "Search the web with Firecrawl and return normalized URLs, snippets, and optional page markdown."
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
        let provider_config = self.config.provider_config();
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
