use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use roder_web_search::{
    RenderOptions, WebSearchRequest, canonical_web_search_schema, render_web_search_response,
};

use crate::client::{PerplexitySearchClient, PerplexitySearchConfig, perplexity_request_id};

pub const PERPLEXITY_SEARCH_TOOL_NAME: &str = "perplexity_search";

#[derive(Debug)]
pub struct PerplexitySearchContributor {
    tool: Arc<PerplexitySearchTool>,
}

impl PerplexitySearchContributor {
    pub fn new(config: PerplexitySearchConfig) -> anyhow::Result<Self> {
        Ok(Self {
            tool: Arc::new(PerplexitySearchTool::new(config)?),
        })
    }
}

impl ToolContributor for PerplexitySearchContributor {
    fn id(&self) -> ToolProviderId {
        "perplexity-search".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(self.tool.clone())
    }
}

#[derive(Debug, Clone)]
pub struct PerplexitySearchTool {
    client: PerplexitySearchClient,
    config: PerplexitySearchConfig,
}

impl PerplexitySearchTool {
    pub fn new(config: PerplexitySearchConfig) -> anyhow::Result<Self> {
        let client = PerplexitySearchClient::new(config.clone())?;
        Ok(Self { client, config })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for PerplexitySearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: PERPLEXITY_SEARCH_TOOL_NAME.to_string(),
            description:
                "Search the web with Perplexity's raw Search API and return normalized ranked URLs, snippets, dates, citations, and usage metadata."
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
        let request_id = perplexity_request_id(&response.raw);
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
