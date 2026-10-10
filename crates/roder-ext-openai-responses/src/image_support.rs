//! Which Responses providers put images in front of the model.
//!
//! One answer serves the request mapping (user images and tool-result images)
//! and `InferenceEngine::tool_result_image_input`, so the runtime stops
//! offering screenshots exactly when the replay would drop them. The existing
//! assumption is kept: every profile forwards images except xAI, which follows
//! the model catalog. OpenRouter, Fireworks and custom providers are assumed to
//! take images whatever the model.

use super::*;

impl ResponsesProviderProfile {
    /// The profile an engine gets from its provider id.
    pub(super) fn for_provider_id(provider_id: &str) -> Self {
        match provider_id {
            PROVIDER_XAI | PROVIDER_SUPERGROK => Self::Xai,
            PROVIDER_OPENROUTER => Self::OpenRouter,
            PROVIDER_FIREWORKS => Self::Fireworks,
            _ => Self::OpenAi,
        }
    }

    /// Whether the replay sends images for `model` instead of falling back to
    /// text.
    pub(super) fn supports_images(self, provider: &str, model: &str) -> bool {
        if matches!(self, Self::Xai) {
            lookup_model_for_provider(provider, model)
                .map(|entry| entry.supports_images)
                .unwrap_or(true)
        } else {
            true
        }
    }
}

/// Whether the Responses replay forwards tool-result images for `model` on the
/// provider `provider_id`. Engines that delegate to [`OpenAiResponsesEngine`]
/// (Codex, SuperGrok) call this so they answer exactly as it does.
pub fn forwards_tool_result_images(provider_id: &str, model: &str) -> bool {
    ResponsesProviderProfile::for_provider_id(provider_id).supports_images(provider_id, model)
}
