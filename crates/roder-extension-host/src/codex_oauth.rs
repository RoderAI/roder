use super::*;

pub(super) struct CodexOAuthInferenceEngine;

#[async_trait::async_trait]
impl InferenceEngine for CodexOAuthInferenceEngine {
    fn requires_native_compaction(&self) -> bool {
        true
    }

    /// The Responses engine this one delegates to decides.
    fn tool_result_image_input(&self, model: &str) -> bool {
        roder_ext_openai_responses::forwards_tool_result_images(PROVIDER_CODEX, model)
    }

    fn id(&self) -> roder_api::extension::InferenceEngineId {
        PROVIDER_CODEX.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            reasoning_summaries: true,
            structured_output: true,
            image_input: true,
            prompt_cache: true,
            provider_metadata: true,
            tool_search: false,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "Codex".to_string(),
            description: Some("ChatGPT account provider for Codex models".to_string()),
            auth_type: ProviderAuthType::OAuth,
            auth_label: Some("ChatGPT Plus/Pro".to_string()),
            auth_configured: None,
            recommended: true,
            sort_order: 10,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_codex(false))
    }

    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        codex_responses_engine()
            .await?
            .stream_turn(ctx, request)
            .await
    }

    async fn compact_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<Option<InferenceEventStream>> {
        codex_responses_engine()
            .await?
            .compact_turn(ctx, request)
            .await
    }
}

async fn codex_responses_engine() -> anyhow::Result<OpenAiResponsesEngine> {
    let Some((access_token, account_id)) = roder_codex_auth::access_token().await? else {
        anyhow::bail!(
            "codex auth is missing; run `{}`",
            roder_api::cli_identity::auth_login_command("codex")
        )
    };
    let mut headers = vec![
        ("originator".to_string(), "roder".to_string()),
        ("User-Agent".to_string(), "roder/0.1.0".to_string()),
    ];
    if let Some(account_id) = account_id {
        headers.push(("ChatGPT-Account-Id".to_string(), account_id));
    }
    Ok(OpenAiResponsesEngine::new_with_config(
        Some(access_token),
        PROVIDER_CODEX,
        "https://chatgpt.com/backend-api/codex",
        headers,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_requires_native_compaction_without_loading_credentials() {
        assert!(CodexOAuthInferenceEngine.requires_native_compaction());
    }

    #[test]
    fn oauth_forwards_tool_result_images_like_the_responses_engine_it_delegates_to() {
        assert!(CodexOAuthInferenceEngine.tool_result_image_input("gpt-5.5"));
    }
}
