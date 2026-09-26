//! The text helper Jev uses when a task has to type something.
//!
//! Upstream Jev asks an OpenAI-compatible `/chat/completions` endpoint for the
//! value of one field. Rather than requiring a separate OpenRouter key, Roder
//! serves that from its own harness: the model the calling turn is running,
//! when it speaks chat-completions and Roder holds its key, otherwise the best
//! configured provider Roder already has.

use roder_api::catalog::{
    PROVIDER_KIND_CHAT_COMPLETIONS, PROVIDER_KIND_FIREWORKS, PROVIDER_KIND_OPENAI,
    PROVIDER_KIND_OPENROUTER, PROVIDER_KIND_XAI, ProviderCatalogEntry, built_in_providers,
    lookup_model_for_provider,
};
use roder_api::inference::ModelSelection;

/// Providers Roder prefers for the text helper when the calling turn's own
/// model cannot serve it. Ordered by how well a small, cheap, strictly
/// JSON-shaped reply is served.
/// DeepSeek and OpenRouter come first because upstream Jev sends a reasoning
/// field whose shape only those two are known to accept; the rest work when
/// they tolerate `reasoning.effort`.
const FALLBACK_PROVIDERS: &[&str] = &[
    "deepseek",
    "openrouter",
    "synthetic",
    "xai",
    "fireworks",
    "openai",
];

/// A resolved chat-completions endpoint for Jev's text helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextModel {
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) api_key: String,
    /// Where the choice came from, for the tool result.
    pub(crate) source: &'static str,
    /// Send upstream's `reasoning.enabled=false` shape instead of the default
    /// for this base URL.
    pub(crate) reasoning_none: bool,
}

/// Anything that can hand back an API key for a provider id.
pub(crate) trait KeySource {
    fn key(&self, provider: &str) -> Option<String>;
}

pub(crate) struct RoderKeys;

impl KeySource for RoderKeys {
    fn key(&self, provider: &str) -> Option<String> {
        let env_key = built_in_providers()
            .iter()
            .find(|entry| entry.id == provider)
            .and_then(|entry| entry.env_key);
        env_key
            .and_then(|key| std::env::var(key).ok())
            .filter(|value| !value.trim().is_empty())
            .or_else(|| roder_config::provider_api_key(provider))
    }
}

/// Resolve the text helper: explicit override, then the calling turn's model,
/// then whatever Roder is configured for. `None` means Jev runs without a text
/// helper and typing actions fail upstream instead of guessing a value.
pub(crate) fn resolve(
    explicit_key: Option<String>,
    explicit_base_url: Option<String>,
    explicit_model: Option<String>,
    turn_model: Option<&ModelSelection>,
    keys: &dyn KeySource,
) -> Option<TextModel> {
    if let Some(api_key) = explicit_key.filter(|key| !key.trim().is_empty()) {
        let base_url =
            explicit_base_url.unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());
        return Some(TextModel {
            reasoning_none: reasoning_none_override(&base_url),
            base_url,
            model: explicit_model.unwrap_or_else(|| "inception/mercury-2.5".to_string()),
            api_key,
            source: "explicit",
        });
    }
    if let Some(harness) = from_turn_model(turn_model, keys) {
        return Some(harness);
    }
    from_configured_providers(keys)
}

fn from_turn_model(turn_model: Option<&ModelSelection>, keys: &dyn KeySource) -> Option<TextModel> {
    let selection = turn_model?;
    let provider = chat_completions_provider(&selection.provider)?;
    // The turn may run a model this provider proxies; keep the exact id it used.
    let model = lookup_model_for_provider(provider.id, &selection.model)
        .map(|entry| entry.id.to_string())
        .unwrap_or_else(|| selection.model.clone());
    Some(TextModel {
        base_url: provider.base_url?.to_string(),
        model,
        api_key: keys.key(provider.id)?,
        source: "turn-model",
        reasoning_none: reasoning_none_override(provider.base_url?),
    })
}

fn from_configured_providers(keys: &dyn KeySource) -> Option<TextModel> {
    FALLBACK_PROVIDERS.iter().find_map(|id| {
        let provider = chat_completions_provider(id)?;
        Some(TextModel {
            base_url: provider.base_url?.to_string(),
            model: provider.default_model.to_string(),
            api_key: keys.key(provider.id)?,
            source: "roder-provider",
            reasoning_none: reasoning_none_override(provider.base_url?),
        })
    })
}

/// OpenRouter accepts `reasoning.enabled=false`; DeepSeek has its own
/// `thinking.disabled`, and upstream's default `reasoning.effort` is what the
/// rest are left with.
fn reasoning_none_override(base_url: &str) -> bool {
    base_url.contains("openrouter.ai")
}

/// Providers that expose an OpenAI-style `/chat/completions` endpoint with an
/// API key. OAuth-only harnesses (`claude-code`, `supergrok`, `codex`), native
/// non-OpenAI transports (Anthropic, Gemini) and Cursor — whose provider path
/// is a protobuf AgentService, not chat completions — cannot serve Jev's text
/// helper.
fn chat_completions_provider(id: &str) -> Option<&'static ProviderCatalogEntry> {
    built_in_providers()
        .iter()
        .find(|entry| entry.id == id)
        .filter(|entry| {
            matches!(
                entry.kind,
                PROVIDER_KIND_CHAT_COMPLETIONS
                    | PROVIDER_KIND_OPENAI
                    | PROVIDER_KIND_OPENROUTER
                    | PROVIDER_KIND_XAI
                    | PROVIDER_KIND_FIREWORKS
            )
        })
        .filter(|entry| entry.base_url.is_some() && entry.env_key.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeKeys(HashMap<&'static str, &'static str>);

    impl FakeKeys {
        fn new(pairs: &[(&'static str, &'static str)]) -> Self {
            Self(pairs.iter().copied().collect())
        }
    }

    impl KeySource for FakeKeys {
        fn key(&self, provider: &str) -> Option<String> {
            self.0.get(provider).map(|value| value.to_string())
        }
    }

    #[test]
    fn explicit_override_is_used_verbatim() {
        let resolved = resolve(
            Some("sk-explicit".into()),
            Some("https://example.test/v1".into()),
            Some("tiny-writer".into()),
            None,
            &FakeKeys::new(&[]),
        )
        .unwrap();
        assert_eq!(resolved.base_url, "https://example.test/v1");
        assert_eq!(resolved.model, "tiny-writer");
        assert_eq!(resolved.source, "explicit");
    }

    #[test]
    fn turn_model_serves_the_text_helper_when_roder_holds_its_key() {
        let selection = ModelSelection {
            provider: "deepseek".into(),
            model: "deepseek-chat".into(),
        };
        let resolved = resolve(
            None,
            None,
            None,
            Some(&selection),
            &FakeKeys::new(&[("deepseek", "sk-deepseek")]),
        )
        .unwrap();
        assert_eq!(resolved.model, "deepseek-chat");
        assert_eq!(resolved.base_url, "https://api.deepseek.com/v1");
        assert_eq!(resolved.api_key, "sk-deepseek");
        assert_eq!(resolved.source, "turn-model");
    }

    #[test]
    fn turn_model_on_a_native_transport_does_not_serve_the_text_helper() {
        // Cursor speaks protobuf AgentService, so it cannot answer
        // /chat/completions even though Roder holds its key.
        let selection = ModelSelection {
            provider: "cursor".into(),
            model: "claude-opus-5".into(),
        };
        let resolved = resolve(
            None,
            None,
            None,
            Some(&selection),
            &FakeKeys::new(&[("cursor", "sk-cursor"), ("xai", "sk-xai")]),
        )
        .unwrap();
        assert_eq!(resolved.source, "roder-provider");
        assert_eq!(resolved.api_key, "sk-xai");
    }

    #[test]
    fn oauth_only_turn_model_falls_back_to_a_configured_provider() {
        let selection = ModelSelection {
            provider: "claude-code".into(),
            model: "claude-opus-4-8".into(),
        };
        let resolved = resolve(
            None,
            None,
            None,
            Some(&selection),
            &FakeKeys::new(&[("deepseek", "sk-deepseek")]),
        )
        .unwrap();
        assert_eq!(resolved.source, "roder-provider");
        assert_eq!(resolved.api_key, "sk-deepseek");
    }

    #[test]
    fn fallback_order_prefers_deepseek_over_later_providers() {
        let resolved = resolve(
            None,
            None,
            None,
            None,
            &FakeKeys::new(&[("openai", "sk-openai"), ("deepseek", "sk-deepseek")]),
        )
        .unwrap();
        assert_eq!(resolved.api_key, "sk-deepseek");
        assert_eq!(resolved.base_url, "https://api.deepseek.com/v1");
    }

    #[test]
    fn no_configured_provider_means_no_text_helper() {
        assert!(resolve(None, None, None, None, &FakeKeys::new(&[])).is_none());
    }

    #[test]
    fn non_chat_completions_providers_are_rejected() {
        assert!(chat_completions_provider("anthropic").is_none());
        assert!(chat_completions_provider("claude-code").is_none());
        assert!(chat_completions_provider("supergrok").is_none());
        assert!(chat_completions_provider("cursor").is_none());
        assert!(chat_completions_provider("deepseek").is_some());
        assert!(chat_completions_provider("openrouter").is_some());
    }
}
