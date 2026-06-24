use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use roder_web_search::{
    RenderOptions, WebSearchRequest, canonical_web_search_schema, render_web_search_response,
};

use crate::client::{
    SyntheticSearchClient, SyntheticSearchConfig, synthetic_request_id,
};

pub const SYNTHETIC_SEARCH_TOOL_NAME: &str = "synthetic_search";

#[derive(Debug)]
pub struct SyntheticSearchContributor {
    tool: Arc<SyntheticSearchTool>,
}

impl SyntheticSearchContributor {
    pub fn new(config: SyntheticSearchConfig) -> anyhow::Result<Self> {
        Ok(Self {
            tool: Arc::new(SyntheticSearchTool::new(config)?),
        })
    }
}

impl ToolContributor for SyntheticSearchContributor {
    fn id(&self) -> ToolProviderId {
        "synthetic-search".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(self.tool.clone())
    }
}

#[derive(Debug, Clone)]
pub struct SyntheticSearchTool {
    client: SyntheticSearchClient,
    config: SyntheticSearchConfig,
}

impl SyntheticSearchTool {
    pub fn new(config: SyntheticSearchConfig) -> anyhow::Result<Self> {
        let client = SyntheticSearchClient::new(config.clone())?;
        Ok(Self { client, config })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for SyntheticSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: SYNTHETIC_SEARCH_TOOL_NAME.to_string(),
            description:
                "Search the web with Synthetic and return normalized URLs, titles, snippets, and optional publication dates."
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
        let request_id = synthetic_request_id(&response.raw);
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
