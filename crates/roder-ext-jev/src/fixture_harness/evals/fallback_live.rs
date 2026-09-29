//! A live fallback model for the eval tiers: `JEV_EVAL_FALLBACK=model`
//! runs the automatic fallback after every task Jev could not finish,
//! driven by `JEV_FALLBACK_MODEL` (default `gpt-6-sol`) at
//! `JEV_FALLBACK_REASONING` (default low), through the ChatGPT/Codex
//! sign-in or `OPENAI_API_KEY`, the way Roder's own engines reach them.

use std::sync::Arc;

use async_trait::async_trait;
use roder_api::catalog::PROVIDER_CODEX;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceTurnContext, ModelDescriptor,
};
use roder_ext_openai_responses::OpenAiResponsesEngine;

use super::live::env;
use crate::fallback::model::{FallbackModel, resolve};
use crate::fallback::settings::{FallbackSettings, PinnedModel};

/// Whether this run falls back, and on what; `None` when it does not.
pub(crate) async fn live_fallback() -> Option<Arc<dyn FallbackModel>> {
    if env("JEV_EVAL_FALLBACK").as_deref() != Some("model") {
        return None;
    }
    let mut settings = FallbackSettings::from_env().expect("JEV_FALLBACK_* settings");
    settings.model.get_or_insert(PinnedModel {
        provider: None,
        model: "gpt-6-sol".into(),
    });
    let mut engines: Vec<Arc<dyn InferenceEngine>> = vec![Arc::new(CodexEngine)];
    if let Some(key) = env("OPENAI_API_KEY") {
        engines.push(Arc::new(OpenAiResponsesEngine::new(Some(key))));
    }
    let codex = matches!(roder_codex_auth::status().await, Ok(Some(_)));
    Some(resolve(&settings, None, &engines, "jev-eval", codex).expect("a fallback model"))
}

/// The ChatGPT/Codex sign-in as an inference engine, as Roder's host wires
/// it: a fresh token per turn.
struct CodexEngine;

#[async_trait]
impl InferenceEngine for CodexEngine {
    fn id(&self) -> String {
        PROVIDER_CODEX.into()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            tool_calls: true,
            image_input: true,
            ..InferenceCapabilities::coding_agent_default()
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let Some((token, account)) = roder_codex_auth::access_token().await? else {
            anyhow::bail!("codex auth is missing; run `roder auth login codex`");
        };
        let mut headers = vec![
            ("originator".to_string(), "roder".to_string()),
            ("User-Agent".to_string(), "roder/0.1.0".to_string()),
        ];
        if let Some(account) = account {
            headers.push(("ChatGPT-Account-Id".to_string(), account));
        }
        OpenAiResponsesEngine::new_with_config(
            Some(token),
            PROVIDER_CODEX,
            "https://chatgpt.com/backend-api/codex",
            headers,
        )
        .stream_turn(ctx, request)
        .await
    }
}
