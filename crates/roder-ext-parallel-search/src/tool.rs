use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use roder_web_search::{
    RenderOptions, WebSearchRequest, canonical_web_search_schema, render_web_search_response,
};
use serde_json::json;

use crate::client::{
    ParallelExtractRequest, ParallelSearchClient, ParallelSearchConfig, ParallelSearchOptions,
    parallel_extract_request_id, parallel_request_id, render_parallel_extract_response,
};

pub const PARALLEL_SEARCH_TOOL_NAME: &str = "parallel_search";
pub const PARALLEL_EXTRACT_TOOL_NAME: &str = "parallel_extract";

#[derive(Debug)]
pub struct ParallelSearchContributor {
    search_tool: Arc<ParallelSearchTool>,
    extract_tool: Arc<ParallelExtractTool>,
}

impl ParallelSearchContributor {
    pub fn new(config: ParallelSearchConfig) -> anyhow::Result<Self> {
        let client = Arc::new(ParallelSearchClient::new(config.clone())?);
        Ok(Self {
            search_tool: Arc::new(ParallelSearchTool {
                client: client.clone(),
                config: config.clone(),
            }),
            extract_tool: Arc::new(ParallelExtractTool { client, config }),
        })
    }
}

impl ToolContributor for ParallelSearchContributor {
    fn id(&self) -> ToolProviderId {
        "parallel-search".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(self.search_tool.clone())?;
        registry.register(self.extract_tool.clone())
    }
}

#[derive(Debug, Clone)]
pub struct ParallelSearchTool {
    client: Arc<ParallelSearchClient>,
    config: ParallelSearchConfig,
}

impl ParallelSearchTool {
    pub fn new(config: ParallelSearchConfig) -> anyhow::Result<Self> {
        let client = Arc::new(ParallelSearchClient::new(config.clone())?);
        Ok(Self { client, config })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for ParallelSearchTool {
    fn spec(&self) -> ToolSpec {
        let mut schema = canonical_web_search_schema();
        schema["properties"]["search_queries"] = json!({
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
        data["response_format"] = json!(response_format.as_str());

        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text,
            data,
            is_error: false,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ParallelExtractTool {
    client: Arc<ParallelSearchClient>,
    config: ParallelSearchConfig,
}

impl ParallelExtractTool {
    pub fn new(config: ParallelSearchConfig) -> anyhow::Result<Self> {
        let client = Arc::new(ParallelSearchClient::new(config.clone())?);
        Ok(Self { client, config })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for ParallelExtractTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: PARALLEL_EXTRACT_TOOL_NAME.to_string(),
            description: "Extract LLM-optimized markdown excerpts (or full content) from public URLs with Parallel.ai.".to_string(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "urls": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "One or more public http(s) URLs to extract (max 20)."
                    },
                    "url": {
                        "type": "string",
                        "description": "Single URL convenience field when only one page is needed."
                    },
                    "objective": {
                        "type": "string",
                        "description": "Natural-language goal used to focus excerpts on relevant content."
                    },
                    "query": {
                        "type": "string",
                        "description": "Alias for objective."
                    },
                    "search_queries": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional keyword queries that further focus excerpts."
                    },
                    "full_content": {
                        "type": "boolean",
                        "description": "When true, also return full page markdown content.",
                        "default": false
                    },
                    "include_content": {
                        "type": "boolean",
                        "description": "Alias for full_content.",
                        "default": false
                    },
                    "max_chars_total": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Upper bound on total excerpt characters across all URLs."
                    },
                    "max_chars_per_result": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Upper bound on characters per URL for excerpts or full content."
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Optional session id linking this extract to a prior Parallel search."
                    }
                }
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let request = ParallelExtractRequest::from_tool_arguments(&call.arguments)?;
        let response = self.client.extract(request).await?;
        let request_id =
            parallel_extract_request_id(&response.raw).or_else(|| response.extract_id.clone());
        let provider_config = self.config.provider_config_with_request_id(request_id);
        let text = render_parallel_extract_response(&response);
        let data = response.data(&provider_config);
        let is_error = response.results.is_empty() && !response.errors.is_empty();

        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text,
            data,
            is_error,
        })
    }
}
