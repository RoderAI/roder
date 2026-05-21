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
    ParallelSearchClient, ParallelSearchConfig, ParallelSearchOptions, parallel_request_id,
};

pub const PARALLEL_SEARCH_TOOL_NAME: &str = "parallel_search";

#[derive(Debug)]
pub struct ParallelSearchContributor {
    tool: Arc<ParallelSearchTool>,
}

impl ParallelSearchContributor {
    pub fn new(config: ParallelSearchConfig) -> anyhow::Result<Self> {
        Ok(Self {
            tool: Arc::new(ParallelSearchTool::new(config)?),
        })
    }
}

impl ToolContributor for ParallelSearchContributor {
    fn id(&self) -> ToolProviderId {
        "parallel-search".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(self.tool.clone())
    }
}

#[derive(Debug, Clone)]
pub struct ParallelSearchTool {
    client: ParallelSearchClient,
    config: ParallelSearchConfig,
}

impl ParallelSearchTool {
    pub fn new(config: ParallelSearchConfig) -> anyhow::Result<Self> {
        let client = ParallelSearchClient::new(config.clone())?;
        Ok(Self { client, config })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for ParallelSearchTool {
    fn spec(&self) -> ToolSpec {
        let mut schema = canonical_web_search_schema();
        schema["properties"]["search_queries"] = serde_json::json!({
            "type": "array",
            "items": { "type": "string" },
            "description": "Optional provider-specific keyword searches to run alongside the objective."
        });
        ToolSpec {
            name: PARALLEL_SEARCH_TOOL_NAME.to_string(),
            description:
                "Search the web with Parallel.ai using an objective and return normalized LLM-optimized excerpts."
                    .to_string(),
            parameters: schema,
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let request: WebSearchRequest = serde_json::from_value(call.arguments.clone())?;
        let response_format = request.response_format;
        let options = ParallelSearchOptions::from_tool_arguments(&call.arguments);
        let response = self.client.search(request, options).await?;
        let request_id = parallel_request_id(&response.raw);
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
