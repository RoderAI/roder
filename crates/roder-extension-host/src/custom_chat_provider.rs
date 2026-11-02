use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext, ModelDescriptor,
    ProviderAuthType,
};
use roder_api::tools::ToolChoice;
use roder_ext_openai_chat_completions::{ChatCompletionsRequestConfig, stream_chat_completions};
use roder_ext_openai_responses::discover_models;

pub(crate) struct CustomChatProvider {
    pub id: String,
    pub name: String,
    pub api_key: Option<String>,
    pub base_url: String,
    pub models: Vec<String>,
    pub context_window: Option<u32>,
    pub tool_allowlist: Vec<String>,
}

fn restrict_tools(request: &mut AgentInferenceRequest, allowlist: &[String]) {
    if allowlist.is_empty() {
        return;
    }
    request
        .tools
        .retain(|tool| allowlist.iter().any(|name| name == &tool.name));
    if request.tools.is_empty()
        || matches!(&request.tool_choice, ToolChoice::Specific(name) if !request.tools.iter().any(|tool| &tool.name == name))
    {
        request.tool_choice = ToolChoice::None;
    }
}

#[async_trait::async_trait]
impl InferenceEngine for CustomChatProvider {
    fn id(&self) -> InferenceEngineId {
        self.id.clone()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            reasoning_summaries: true,
            structured_output: true,
            image_input: false,
            prompt_cache: false,
            provider_metadata: true,
            tool_search: false,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: self.name.clone(),
            description: Some(format!(
                "{} OpenAI-compatible Chat Completions provider",
                self.name
            )),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some("API key".into()),
            auth_configured: Some(self.api_key.is_some()),
            recommended: true,
            sort_order: 20,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        if !self.models.is_empty() {
            return Ok(self
                .models
                .iter()
                .map(|id| ModelDescriptor {
                    id: id.clone(),
                    name: id.clone(),
                    context_window: self.context_window,
                    default_reasoning: None,
                    supported_reasoning: Vec::new(),
                })
                .collect());
        }
        // Discovery uses the same OpenAI-compatible /models endpoint as custom Responses.
        // Unknown models remain selectable by name when the endpoint does not list them.
        let mut models = discover_models(&self.base_url, self.api_key.as_deref()).await?;
        for model in &mut models {
            if self.context_window.is_some() {
                model.context_window = self.context_window;
            }
        }
        Ok(models)
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        mut request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        restrict_tools(&mut request, &self.tool_allowlist);
        let config = ChatCompletionsRequestConfig::bearer(
            &self.name,
            &self.base_url,
            self.api_key.as_deref().unwrap_or_default(),
        );
        stream_chat_completions(config, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::inference::ModelSelection;
    use roder_api::tools::ToolSpec;

    fn sample_request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: "custom-chat".into(),
                model: "model".into(),
            },
            instructions: Default::default(),
            transcript: vec![],
            tools: ["read_file", "shell", "grep"]
                .into_iter()
                .map(|name| ToolSpec {
                    name: name.into(),
                    description: name.into(),
                    parameters: serde_json::json!({}),
                })
                .collect(),
            tool_choice: ToolChoice::Auto,
            reasoning: Default::default(),
            output: Default::default(),
            runtime: Default::default(),
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn allowlist_filters_only_custom_chat_request_tools() {
        let mut request = sample_request();
        restrict_tools(&mut request, &["read_file".into(), "grep".into()]);
        assert_eq!(
            request
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            ["read_file", "grep"]
        );
        assert_eq!(request.tool_choice, ToolChoice::Auto);

        let mut unrestricted = sample_request();
        restrict_tools(&mut unrestricted, &[]);
        assert_eq!(unrestricted.tools.len(), 3);
    }

    #[test]
    fn allowlist_removes_unavailable_forced_tool_choice() {
        let mut request = sample_request();
        request.tool_choice = ToolChoice::Specific("shell".into());
        restrict_tools(&mut request, &["read_file".into()]);
        assert_eq!(request.tool_choice, ToolChoice::None);
        assert_eq!(request.tools.len(), 1);

        let mut empty = sample_request();
        empty.tool_choice = ToolChoice::Any;
        restrict_tools(&mut empty, &["missing".into()]);
        assert_eq!(empty.tool_choice, ToolChoice::None);
    }
}
