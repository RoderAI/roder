//! The text helper Jev uses when a task has to type something.
//!
//! Upstream Jev asks an OpenAI-compatible `/chat/completions` endpoint for the
//! value of one field. Roder serves that from its own harness instead of
//! requiring a separate OpenRouter key. The preferred writer is GPT-6 Sol at
//! low reasoning effort through the ChatGPT/Codex sign-in (the Responses API);
//! without that sign-in, the model the calling turn is running, when it speaks
//! chat-completions and Roder holds its key, otherwise the best configured
//! provider Roder already has.

use anyhow::{Context, bail};
use async_trait::async_trait;
use roder_api::catalog::{
    PROVIDER_KIND_CHAT_COMPLETIONS, PROVIDER_KIND_FIREWORKS, PROVIDER_KIND_OPENAI,
    PROVIDER_KIND_OPENROUTER, PROVIDER_KIND_XAI, PROVIDER_OPENAI, ProviderCatalogEntry,
    built_in_providers, lookup_model, lookup_model_for_provider,
};
use roder_api::inference::ModelSelection;

/// The Codex-served model that writes field values by default.
pub(crate) const CODEX_MODEL: &str = "gpt-6-sol";

/// Providers Roder prefers for the text helper when neither the Codex
/// sign-in nor the calling turn's own model can serve it. Ordered by how well
/// a small, cheap, strictly JSON-shaped reply is served.
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

/// A reasoning effort, as `JEV_TEXT_MODEL_REASONING` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Effort {
    None,
    Low,
    Medium,
    High,
}

impl Effort {
    pub(crate) fn parse(raw: &str) -> anyhow::Result<Self> {
        Ok(match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Self::None,
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            other => {
                bail!("JEV_TEXT_MODEL_REASONING must be none, low, medium or high, not {other:?}")
            }
        })
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// How the text helper reaches its model. Its `Debug` leaves the key out.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Transport {
    /// An OpenAI-compatible `/chat/completions` endpoint with an API key.
    Chat {
        base_url: String,
        api_key: String,
        /// `None` sends upstream's default reasoning field for this base URL
        /// (see `text_helper::reasoning`); a level sends that level.
        reasoning: Option<Effort>,
    },
    /// The Responses API behind the ChatGPT/Codex sign-in; the token is
    /// fetched, and refreshed, by `roder-codex-auth` on each call.
    Codex { effort: Effort },
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chat {
                base_url,
                reasoning,
                ..
            } => formatter
                .debug_struct("Chat")
                .field("base_url", base_url)
                .field("api_key", &"[redacted]")
                .field("reasoning", reasoning)
                .finish(),
            Self::Codex { effort } => formatter
                .debug_struct("Codex")
                .field("effort", effort)
                .finish(),
        }
    }
}

/// A resolved text helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextModel {
    pub(crate) model: String,
    /// Where the choice came from, for the tool result: `explicit`, `codex`,
    /// `turn-model` or `roder-provider`.
    pub(crate) source: &'static str,
    pub(crate) transport: Transport,
    /// Why this is not the default choice, for the tool result: set when a
    /// stored Codex sign-in could not be used and this model stands in.
    pub(crate) note: Option<&'static str>,
    /// What a default Codex choice falls back on if the sign-in stops
    /// producing a usable token mid-run; never set for an explicit choice.
    pub(crate) fallback: Option<Box<TextModel>>,
}

/// The note a model standing in for GPT-6 Sol carries. It never quotes the
/// token or the auth error.
pub(crate) const CODEX_UNUSABLE: &str =
    "Codex sign-in unusable; sign in again with `roder auth login codex` to use gpt-6-sol";

impl TextModel {
    /// The reasoning effort the request asks for, as reported.
    pub(crate) fn effort(&self) -> &'static str {
        match &self.transport {
            Transport::Codex { effort } => effort.as_str(),
            Transport::Chat {
                reasoning: Some(effort),
                ..
            } => effort.as_str(),
            // Upstream's defaults: DeepSeek's thinking off, else low.
            Transport::Chat { base_url, .. } if is_deepseek(base_url) => "none",
            Transport::Chat { .. } => "low",
        }
    }

    /// A one-line label for eval rows and logs; never carries the key.
    #[cfg(test)]
    pub(crate) fn label(&self) -> String {
        let endpoint = match &self.transport {
            Transport::Chat { base_url, .. } => base_url.as_str(),
            Transport::Codex { .. } => "codex responses",
        };
        let mut label = format!(
            "{} effort {} ({}, {endpoint})",
            self.model,
            self.effort(),
            self.source
        );
        if let Some(note) = self.note {
            label.push_str(&format!("; {note}"));
        }
        label
    }

    /// This model standing in for an unusable Codex sign-in.
    pub(crate) fn standing_in(mut self) -> Self {
        self.note = Some(CODEX_UNUSABLE);
        self.fallback = None;
        self
    }
}

pub(crate) fn is_deepseek(base_url: &str) -> bool {
    base_url.contains("api.deepseek.com/")
}

/// Anything that can hand back an API key for a provider id.
pub(crate) trait KeySource: Sync {
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

/// Whether Roder holds a ChatGPT/Codex sign-in, and whether it can produce
/// a token now.
#[async_trait]
pub(crate) trait CodexSignIn: Send + Sync {
    async fn signed_in(&self) -> bool;
    async fn usable(&self) -> bool;
}

pub(crate) struct RoderCodexSignIn;

#[async_trait]
impl CodexSignIn for RoderCodexSignIn {
    /// `roder-codex-auth`'s own test: a stored refresh token. An unreadable
    /// store counts as signed out.
    async fn signed_in(&self) -> bool {
        matches!(roder_codex_auth::status().await, Ok(Some(_)))
    }

    /// A token `roder-codex-auth` hands out, refreshed if it was about to
    /// expire. Whether the backend accepts it shows only on the first call.
    async fn usable(&self) -> bool {
        matches!(roder_codex_auth::access_token().await, Ok(Some(_)))
    }
}

/// The operator's explicit choices, from the environment.
#[derive(Debug, Default, Clone)]
pub(crate) struct Explicit {
    /// `JEV_TEXT_MODEL_API_KEY`, else `OPENROUTER_API_KEY`.
    pub(crate) key: Option<String>,
    /// `JEV_TEXT_MODEL_BASE_URL`, with an explicit key only.
    pub(crate) base_url: Option<String>,
    /// `JEV_TEXT_MODEL`.
    pub(crate) model: Option<String>,
    /// `JEV_TEXT_MODEL_REASONING`, parsed.
    pub(crate) reasoning: Option<Effort>,
}

/// Resolve the text helper: an explicit key, then an explicit model, then
/// GPT-6 Sol through the Codex sign-in, then the calling turn's model, then
/// whatever Roder is configured for. `Ok(None)` means Jev runs without a
/// text helper, and a run that must type ends `needs_input` instead of
/// guessing a value. An explicit model nothing can serve, or an effort it
/// does not take, is an error rather than a silent substitute.
///
/// The default GPT-6 Sol choice alone falls back: a stored sign-in that
/// cannot produce a token now yields the next source with
/// [`CODEX_UNUSABLE`] as its note, and a usable one carries the next source
/// as its `fallback` for a sign-in that fails mid-run. Setting
/// `JEV_TEXT_MODEL_REASONING` makes the choice explicit: no fallback.
pub(crate) async fn resolve(
    explicit: Explicit,
    turn_model: Option<&ModelSelection>,
    keys: &dyn KeySource,
    codex: &dyn CodexSignIn,
) -> anyhow::Result<Option<TextModel>> {
    if let Some(api_key) = explicit.key.filter(|key| !key.trim().is_empty()) {
        let base_url = explicit
            .base_url
            .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());
        return Ok(Some(TextModel {
            model: explicit
                .model
                .unwrap_or_else(|| "inception/mercury-2.5".to_string()),
            source: "explicit",
            note: None,
            fallback: None,
            transport: Transport::Chat {
                reasoning: explicit.reasoning.or(default_reasoning(&base_url)),
                base_url,
                api_key,
            },
        }));
    }
    if let Some(model) = explicit.model {
        return explicit_model(&model, explicit.reasoning, keys, codex)
            .await
            .map(Some);
    }
    let next = || {
        from_turn_model(turn_model, explicit.reasoning, keys)
            .or_else(|| from_configured_providers(explicit.reasoning, keys))
    };
    if codex.signed_in().await {
        if explicit.reasoning.is_some() {
            return codex_model(CODEX_MODEL, explicit.reasoning, "codex").map(Some);
        }
        if !codex.usable().await {
            // With nothing to stand in, GPT-6 Sol stays, and each fill says
            // to sign in again.
            return match next() {
                Some(model) => Ok(Some(model.standing_in())),
                None => codex_model(CODEX_MODEL, None, "codex").map(Some),
            };
        }
        let mut model = codex_model(CODEX_MODEL, None, "codex")?;
        model.fallback = next().map(Box::new);
        return Ok(Some(model));
    }
    Ok(next())
}

/// `JEV_TEXT_MODEL` without a key: the provider that serves the model in
/// Roder's catalog. An OpenAI model goes through the Codex sign-in when
/// Roder holds one, else through an OpenAI API key.
async fn explicit_model(
    model: &str,
    reasoning: Option<Effort>,
    keys: &dyn KeySource,
    codex: &dyn CodexSignIn,
) -> anyhow::Result<TextModel> {
    let entry = lookup_model(model).with_context(|| {
        format!("JEV_TEXT_MODEL={model} is not a model in Roder's catalog; set JEV_TEXT_MODEL_API_KEY and JEV_TEXT_MODEL_BASE_URL to use it through another endpoint")
    })?;
    if entry.provider == PROVIDER_OPENAI && codex.signed_in().await {
        return codex_model(entry.id, reasoning, "explicit");
    }
    let provider = chat_completions_provider(entry.provider).with_context(|| {
        format!(
            "JEV_TEXT_MODEL={model} is served by {}, which cannot answer the text helper (sign in with `roder auth login codex` for OpenAI models)",
            entry.provider
        )
    })?;
    let api_key = keys.key(provider.id).with_context(|| {
        format!(
            "JEV_TEXT_MODEL={model} needs a {} key, and Roder holds none",
            provider.id
        )
    })?;
    let base_url = provider.base_url.unwrap_or_default().to_string();
    Ok(TextModel {
        model: entry.id.to_string(),
        source: "explicit",
        note: None,
        fallback: None,
        transport: Transport::Chat {
            reasoning: reasoning.or(default_reasoning(&base_url)),
            base_url,
            api_key,
        },
    })
}

/// A Codex-served model at `reasoning`, default low; an effort the catalog
/// says the model does not take is refused.
fn codex_model(
    model: &str,
    reasoning: Option<Effort>,
    source: &'static str,
) -> anyhow::Result<TextModel> {
    let effort = reasoning.unwrap_or(Effort::Low);
    if let Some(entry) = lookup_model(model) {
        anyhow::ensure!(
            entry
                .supported_reasoning
                .iter()
                .any(|option| option.effort == effort.as_str()),
            "{model} does not take reasoning effort {}; it takes {}",
            effort.as_str(),
            entry
                .supported_reasoning
                .iter()
                .map(|option| option.effort)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(TextModel {
        model: model.to_string(),
        source,
        note: None,
        fallback: None,
        transport: Transport::Codex { effort },
    })
}

fn from_turn_model(
    turn_model: Option<&ModelSelection>,
    reasoning: Option<Effort>,
    keys: &dyn KeySource,
) -> Option<TextModel> {
    let selection = turn_model?;
    let provider = chat_completions_provider(&selection.provider)?;
    // The turn may run a model this provider proxies; keep the exact id it used.
    let model = lookup_model_for_provider(provider.id, &selection.model)
        .map(|entry| entry.id.to_string())
        .unwrap_or_else(|| selection.model.clone());
    let base_url = provider.base_url?.to_string();
    Some(TextModel {
        model,
        source: "turn-model",
        note: None,
        fallback: None,
        transport: Transport::Chat {
            api_key: keys.key(provider.id)?,
            reasoning: reasoning.or(default_reasoning(&base_url)),
            base_url,
        },
    })
}

fn from_configured_providers(reasoning: Option<Effort>, keys: &dyn KeySource) -> Option<TextModel> {
    FALLBACK_PROVIDERS.iter().find_map(|id| {
        let provider = chat_completions_provider(id)?;
        let base_url = provider.base_url?.to_string();
        Some(TextModel {
            model: provider.default_model.to_string(),
            source: "roder-provider",
            note: None,
            fallback: None,
            transport: Transport::Chat {
                api_key: keys.key(provider.id)?,
                reasoning: reasoning.or(default_reasoning(&base_url)),
                base_url,
            },
        })
    })
}

/// OpenRouter accepts `reasoning.enabled=false`, so Roder turns reasoning off
/// there; everywhere else upstream's per-endpoint default stands (DeepSeek's
/// `thinking.disabled`, else `reasoning.effort: low`).
fn default_reasoning(base_url: &str) -> Option<Effort> {
    base_url.contains("openrouter.ai").then_some(Effort::None)
}

/// Providers that expose an OpenAI-style `/chat/completions` endpoint with an
/// API key. OAuth-only harnesses (`claude-code`, `supergrok`, `codex`, the
/// last served by the Responses path instead), native non-OpenAI transports
/// (Anthropic, Gemini) and Cursor — whose provider path is a protobuf
/// AgentService, not chat completions — cannot serve this path.
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
mod tests;
