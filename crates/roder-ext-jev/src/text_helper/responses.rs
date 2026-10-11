//! The text helper over the Responses API, through the ChatGPT/Codex sign-in.
//!
//! The request is built by `roder-ext-openai-responses` from the same system
//! prompt and field context the chat-completions path sends, and the token
//! comes from `roder-codex-auth`, which refreshes it when it is about to
//! expire; nothing here reads or stores credentials itself. The reply is held
//! to a strict JSON schema, `{"text": string | null}`, and then to the same
//! checks as every other reply (`parse_value`, `grounded`).

use anyhow::Context;
use async_trait::async_trait;
use roder_api::catalog::PROVIDER_CODEX;
use roder_api::inference::{
    AgentInferenceRequest, InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig,
    RuntimeHints,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_openai_responses::OpenAiResponsesEngine;
use serde_json::{Value, json};

use crate::engine::{JevStatus, JevStop};
use crate::http::{JsonPoster, PostFailure};
use crate::prompts::TEXT_VALUE;
use crate::python_json;
use crate::text_model::Effort;

/// Where Roder's Codex provider sends Responses requests.
pub(crate) const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

/// The headers Roder's Codex provider sends besides the token (see
/// `roder-extension-host`'s `codex_oauth`).
const ORIGINATOR: &str = "roder";
const USER_AGENT: &str = "roder/0.1.0";

/// The reply's schema: exactly one key, `text`, a string or null. The
/// backend holds the model to it; `parse_value` checks it again.
pub(crate) fn value_format() -> Value {
    json!({
        "type": "json_schema",
        "name": "field_value",
        "strict": true,
        "schema": {
            "type": "object",
            "properties": {"text": {"type": ["string", "null"]}},
            "required": ["text"],
            "additionalProperties": false,
        },
    })
}

/// The Responses body for one field. `roder-ext-openai-responses` maps the
/// request as it would for a turn; two things a one-shot, unstored call
/// never uses are then dropped: the encrypted reasoning a later turn would
/// replay (`include`) and the reasoning summary, which nobody reads.
pub(crate) fn request_body(context: &Value, model: &str, effort: Effort) -> Value {
    let request = AgentInferenceRequest {
        model: ModelSelection {
            provider: PROVIDER_CODEX.into(),
            model: model.into(),
        },
        instructions: InstructionBundle {
            system: Some(TEXT_VALUE.into()),
            ..InstructionBundle::default()
        },
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(
            python_json::dumps(context),
        ))],
        tools: Vec::new(),
        tool_choice: ToolChoice::None,
        reasoning: ReasoningConfig {
            enabled: true,
            level: Some(effort.as_str().into()),
        },
        output: OutputConfig {
            response_format: Some(value_format()),
            ..OutputConfig::default()
        },
        runtime: RuntimeHints::default(),
        metadata: Value::Null,
    };
    let mut body = OpenAiResponsesEngine::map_request(&request);
    if let Some(object) = body.as_object_mut() {
        object.remove("include");
        object.remove("context_management");
    }
    body["reasoning"] = json!({"effort": effort.as_str()});
    body
}

/// The reply's text: every `output_text` part of its messages, in order.
/// A reply cut short (`incomplete`) has no usable value.
pub(crate) fn output_text(response: &Value) -> Option<String> {
    if response["status"].as_str() == Some("incomplete") {
        return None;
    }
    let text = response["output"]
        .as_array()?
        .iter()
        .filter(|item| item["type"] == "message")
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter(|part| part["type"] == "output_text")
        .filter_map(|part| part["text"].as_str())
        .collect::<String>();
    (!text.is_empty()).then_some(text)
}

/// A Codex access token and the account it belongs to. `Debug` leaves the
/// token out.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CodexToken {
    pub(crate) access: String,
    pub(crate) account_id: Option<String>,
}

impl std::fmt::Debug for CodexToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodexToken")
            .field("access", &"[redacted]")
            .field("account_id", &self.account_id.as_ref().map(|_| "[set]"))
            .finish()
    }
}

/// Where the token comes from.
#[async_trait]
pub(crate) trait CodexAuth: Send + Sync {
    /// The current token, refreshed if it is about to expire; `None` when
    /// Roder holds no sign-in.
    async fn token(&self) -> anyhow::Result<Option<CodexToken>>;
}

/// Roder's own Codex sign-in, through `roder-codex-auth`.
pub(crate) struct RoderCodexAuth;

#[async_trait]
impl CodexAuth for RoderCodexAuth {
    async fn token(&self) -> anyhow::Result<Option<CodexToken>> {
        Ok(roder_codex_auth::access_token()
            .await?
            .map(|(access, account_id)| CodexToken { access, account_id }))
    }
}

/// The sign-in cannot produce a token the backend accepts: none stored, a
/// refresh refused, or the same token refused twice. A default choice of
/// the Codex model falls back on it (see `TextHelper`); an explicit one ends
/// the run with the wrapped stop.
#[derive(Debug)]
pub(crate) struct SignInUnusable(JevStop);

impl SignInUnusable {
    fn stop(reason: String) -> anyhow::Error {
        Self(JevStop::new(JevStatus::Error, reason)).into()
    }

    pub(crate) fn is(error: &anyhow::Error) -> bool {
        error.downcast_ref::<Self>().is_some()
    }
}

impl std::fmt::Display for SignInUnusable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for SignInUnusable {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// The token to send, or the run's end when there is none. The auth
/// crate's own error is not quoted: it can carry the token endpoint's reply.
async fn current_token(auth: &dyn CodexAuth) -> anyhow::Result<CodexToken> {
    match auth.token().await {
        Ok(Some(token)) => Ok(token),
        Ok(None) => Err(SignInUnusable::stop(format!(
            "Text helper has no Codex sign-in; sign in again with `{}`. Nothing typed.",
            roder_api::cli_identity::auth_login_command("codex")
        ))),
        Err(_) => Err(SignInUnusable::stop(format!(
            "Text helper could not refresh the Codex sign-in; sign in again with `{}`. Nothing typed.",
            roder_api::cli_identity::auth_login_command("codex")
        ))),
    }
}

/// POST one Responses request with the current token. A 401 fetches the
/// token again and, when another process has refreshed it meanwhile, sends
/// once more with the new one; the same token refused twice ends the run.
pub(crate) async fn post(
    http: &JsonPoster,
    auth: &dyn CodexAuth,
    base_url: &str,
    body: &Value,
) -> anyhow::Result<Value> {
    let url = format!("{}/responses", base_url.trim_end_matches('/'));
    let mut token = current_token(auth).await?;
    let mut refreshed = false;
    loop {
        let mut headers = vec![
            ("originator", ORIGINATOR.to_string()),
            ("User-Agent", USER_AGENT.to_string()),
        ];
        if let Some(account) = &token.account_id {
            headers.push(("ChatGPT-Account-Id", account.clone()));
        }
        match http.post_with(&url, &token.access, &headers, body).await {
            Ok(response) => return Ok(response),
            Err(PostFailure::Status(401, _)) if !refreshed => {
                refreshed = true;
                let next = current_token(auth).await?;
                if next == token {
                    return Err(refused());
                }
                token = next;
            }
            Err(PostFailure::Status(401, _)) => return Err(refused()),
            Err(failure) => {
                let message = match &failure {
                    PostFailure::Connection => {
                        "Text helper (Codex) connection failed; nothing typed.".to_string()
                    }
                    PostFailure::Status(status, None) => {
                        format!("Text helper (Codex) returned HTTP {status}; nothing typed.")
                    }
                    PostFailure::Status(status, Some(detail)) => format!(
                        "Text helper (Codex) returned HTTP {status} ({detail}); nothing typed."
                    ),
                    PostFailure::InvalidBody => {
                        "Text helper (Codex) sent an unreadable reply; nothing typed.".to_string()
                    }
                    PostFailure::Failed(detail) => {
                        format!("Text helper (Codex) failed ({detail}); nothing typed.")
                    }
                };
                return Err(failure.stop(message));
            }
        }
    }
}

fn refused() -> anyhow::Error {
    SignInUnusable::stop(format!(
        "Text helper's Codex sign-in was refused (HTTP 401); sign in again with `{}`. Nothing typed.",
        roder_api::cli_identity::auth_login_command("codex")
    ))
}

/// The usage a Responses reply reports, as the run sums it:
/// `input_tokens` and `output_tokens` are read directly; reasoning tokens
/// are already part of `output_tokens`, cached ones of `input_tokens`. Only
/// the counts are kept: the Codex backend adds a per-item `attribution`
/// breakdown several times their size.
pub(crate) fn usage(response: &Value) -> Value {
    let mut kept = serde_json::Map::new();
    for key in [
        "input_tokens",
        "input_tokens_details",
        "output_tokens",
        "output_tokens_details",
        "total_tokens",
    ] {
        if let Some(value) = response["usage"].get(key) {
            kept.insert(key.into(), value.clone());
        }
    }
    Value::Object(kept)
}

/// The message a reply without text gives.
pub(crate) fn no_text(response: &Value) -> anyhow::Result<String> {
    output_text(response).context(super::NO_VALUE)
}
