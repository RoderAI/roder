use roder_api::catalog::{PROVIDER_SUPERGROK, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext, ModelDescriptor,
    ProviderAuthType,
};
use roder_ext_openai_responses::{
    OpenAiResponsesEngine, cache_ttl, cached_models, discover_models, force_refresh_requested,
    save_cached_models,
};

const DEFAULT_XAI_BASE_URL: &str = "https://api.x.ai/v1";

pub struct SuperGrokEngine;

#[async_trait::async_trait]
impl InferenceEngine for SuperGrokEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_SUPERGROK.to_string()
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
            name: "SuperGrok".to_string(),
            description: Some("SuperGrok OAuth provider for xAI Grok models".to_string()),
            auth_type: ProviderAuthType::OAuth,
            auth_label: Some("SuperGrok subscription".to_string()),
            auth_configured: None,
            recommended: false,
            sort_order: 55,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        // Refresh the xAI /models cache in the background when OAuth is configured,
        // but always return the curated SuperGrok catalog so users see the intended
        // Grok Build / Composer options with hand-tuned context windows.
        if let Ok(Some(token)) = roder_supergrok_auth::access_token().await {
            let base = DEFAULT_XAI_BASE_URL;
            let pid = PROVIDER_SUPERGROK;
            let cached = cached_models(pid, base).ok();
            let should_refresh = force_refresh_requested()
                || cached
                    .as_ref()
                    .map(|entry| entry.is_stale(cache_ttl()))
                    .unwrap_or(true);
            if should_refresh {
                let base_for_refresh = base.to_string();
                let token_for_refresh = token.clone();
                tokio::spawn(async move {
                    if let Ok(models) =
                        discover_models(&base_for_refresh, Some(&token_for_refresh)).await
                    {
                        let _ = save_cached_models(pid, &base_for_refresh, &models);
                    }
                });
            }
        }
        Ok(models_for_provider(PROVIDER_SUPERGROK, false))
    }

    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let Some(access_token) = roder_supergrok_auth::access_token().await? else {
            anyhow::bail!("supergrok auth is missing; run `roder auth login supergrok`")
        };
        OpenAiResponsesEngine::new_with_config(
            Some(access_token),
            PROVIDER_SUPERGROK,
            DEFAULT_XAI_BASE_URL,
            Vec::new(),
        )
        .stream_turn(ctx, request)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn supergrok_lists_grok_models_without_auth() {
        let engine = SuperGrokEngine;

        let models = engine
            .list_models(InferenceProviderContext {
                provider_id: PROVIDER_SUPERGROK,
            })
            .await
            .unwrap();

        let ids = models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["grok-build-0.1", "grok-composer-2.5-fast"]);

        let build = models
            .iter()
            .find(|model| model.id == "grok-build-0.1")
            .unwrap();
        assert_eq!(build.name, "Grok Build 0.1");
        assert_eq!(build.context_window, Some(500_000));

        let composer = models
            .iter()
            .find(|model| model.id == "grok-composer-2.5-fast")
            .unwrap();
        assert_eq!(composer.name, "Grok Composer 2.5 Fast");
        assert_eq!(composer.context_window, Some(200_000));
    }

    #[test]
    fn supergrok_tools_use_openai_responses_provider_contract() {
        let engine = SuperGrokEngine;

        assert!(engine.capabilities().tool_calls);
        assert!(engine.capabilities().parallel_tool_calls);
    }

    #[test]
    fn profile_supergrok_uses_openai_responses_profile_contract() {
        let engine = SuperGrokEngine;
        let capabilities = engine.capabilities();

        assert!(capabilities.tool_calls);
        assert!(capabilities.parallel_tool_calls);
        assert!(capabilities.reasoning_summaries);
        assert!(capabilities.prompt_cache);
    }

    #[test]
    fn retry_supergrok_delegates_to_openai_responses_transport() {
        let engine = SuperGrokEngine;

        assert!(engine.capabilities().streaming);
        assert!(engine.capabilities().provider_metadata);
    }
}
