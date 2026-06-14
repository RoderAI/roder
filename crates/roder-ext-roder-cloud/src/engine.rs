use std::sync::Arc;

use roder_api::catalog::{PROVIDER_RODER_CLOUD, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceProviderMetadata,
    InferenceTurnContext, MessageDelta, ModelDescriptor, ProviderAuthType, TokenUsage,
};
use roder_ext_openai_responses::OpenAiResponsesEngine;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::token::{DEFAULT_WEB_URL, RoderCloudTokenSource};

/**
 * Request keys forwarded by the roder.cloud Responses subset. Everything
 * else is pruned client-side so the request stays within the documented
 * contract (the edge also rejects `background` outright). Tools are omitted
 * on purpose: the edge drops function-call payloads from upstream output,
 * so advertising or sending tools would produce calls we can never parse.
 */
const SUPPORTED_REQUEST_KEYS: &[&str] = &[
    "model",
    "input",
    "instructions",
    "max_output_tokens",
    "temperature",
    "top_p",
];

#[derive(Debug, Deserialize)]
struct ResponsesPayload {
    #[serde(default)]
    id: String,
    #[serde(default)]
    output: Vec<OutputItem>,
    #[serde(default)]
    output_text: String,
    #[serde(default)]
    usage: UsagePayload,
}

#[derive(Debug, Deserialize)]
struct OutputItem {
    #[serde(default)]
    content: Vec<OutputContent>,
}

#[derive(Debug, Deserialize)]
struct OutputContent {
    #[serde(default)]
    text: String,
}

#[derive(Debug, Default, Deserialize)]
struct UsagePayload {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    #[serde(default)]
    error: ErrorPayload,
}

#[derive(Debug, Default, Deserialize)]
struct ErrorPayload {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct ModelsPayload {
    #[serde(default)]
    data: Vec<ModelsEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelsEntry {
    #[serde(default)]
    id: String,
}

/**
 * Inference engine for the roder.cloud hosted edge. The edge speaks a
 * synchronous (non-streaming) subset of the OpenAI Responses API behind a
 * short-lived JWT minted from the team `roder_` API key, so this engine
 * issues one synchronous request per turn and synthesizes Roder stream
 * events from the completed response.
 */
pub struct RoderCloudEngine {
    base_url: Option<String>,
    token_source: Option<Arc<RoderCloudTokenSource>>,
    web_url: String,
    client: reqwest::Client,
}

impl RoderCloudEngine {
    pub fn new(api_key: Option<String>, base_url: Option<String>, web_url: Option<String>) -> Self {
        let web_url = nonempty(web_url).unwrap_or_else(|| DEFAULT_WEB_URL.to_string());
        let token_source = nonempty(api_key)
            .map(|api_key| Arc::new(RoderCloudTokenSource::new(web_url.clone(), api_key)));
        Self {
            base_url: nonempty(base_url).map(|url| url.trim_end_matches('/').to_string()),
            token_source,
            web_url,
            client: reqwest::Client::new(),
        }
    }

    fn require_base_url(&self) -> anyhow::Result<&str> {
        self.base_url.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "roder.cloud inference URL is not configured; set RODER_CLOUD_BASE_URL or \
                 [providers.roder-cloud].base_url (local dev: http://127.0.0.1:8080/v1)"
            )
        })
    }

    fn require_token_source(&self) -> anyhow::Result<&Arc<RoderCloudTokenSource>> {
        self.token_source.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "Roder Cloud API key is missing; create one at {}/teams and set \
                 RODER_CLOUD_API_KEY or configure it from the provider menu",
                self.web_url
            )
        })
    }

    /// Map the canonical request onto the roder.cloud Responses subset.
    /// Public so tests and eval harnesses can snapshot the exact payload.
    ///
    /// `input` is sent as a plain string: the godex edge forwards it verbatim
    /// to heterogeneous upstreams (OpenRouter free routes, Anthropic Messages,
    /// Google generateContent), and a flat string is the only shape every
    /// route accepts today.
    pub fn map_request(request: &AgentInferenceRequest) -> Value {
        let mapped = OpenAiResponsesEngine::map_request(request);
        let mut body = json!({ "stream": false });
        if let Value::Object(mapped) = mapped {
            for key in SUPPORTED_REQUEST_KEYS {
                if let Some(value) = mapped.get(*key) {
                    body[*key] = value.clone();
                }
            }
        }
        body["input"] = json!(flatten_input(request));
        body
    }

    async fn send_turn(&self, body: &Value) -> anyhow::Result<ResponsesPayload> {
        let base_url = self.require_base_url()?;
        let token_source = self.require_token_source()?;
        let url = format!("{base_url}/responses");
        let mut token = token_source.token().await?;
        for attempt in 0..2 {
            let response = self
                .client
                .post(&url)
                .bearer_auth(&token)
                .json(body)
                .send()
                .await
                .map_err(|err| anyhow::anyhow!("roder.cloud request failed at {url}: {err}"))?;
            let status = response.status();
            let bytes = response.bytes().await.unwrap_or_default();
            if status.is_success() {
                return serde_json::from_slice::<ResponsesPayload>(&bytes).map_err(|err| {
                    anyhow::anyhow!("roder.cloud returned malformed response JSON: {err}")
                });
            }
            let error = parse_error(&bytes);
            // An expired or rotated JWT surfaces as 401 invalid_token; one
            // forced re-exchange is enough because the key itself is
            // validated during the exchange.
            if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                token = token_source.refresh().await?;
                continue;
            }
            anyhow::bail!(turn_error_message(status, &error, &self.web_url));
        }
        unreachable!("retry loop returns or bails");
    }
}

fn parse_error(bytes: &[u8]) -> ErrorPayload {
    serde_json::from_slice::<ErrorEnvelope>(bytes)
        .map(|envelope| envelope.error)
        .unwrap_or_else(|_| ErrorPayload {
            code: String::new(),
            message: String::from_utf8_lossy(bytes).trim().to_string(),
        })
}

fn turn_error_message(status: reqwest::StatusCode, error: &ErrorPayload, web_url: &str) -> String {
    let code = error.code.as_str();
    let message = error.message.as_str();
    match code {
        "quota_exceeded" => format!(
            "roder.cloud quota exceeded ({message}); monthly request/token limits reset at the \
             start of the month — manage limits at {web_url}"
        ),
        "model_not_allowed" => format!(
            "roder.cloud rejected the model ({message}); enable it for your team at \
             {web_url}/teams under Models"
        ),
        "invalid_token" | "missing_token" => format!(
            "roder.cloud authentication failed ({code}: {message}); check RODER_CLOUD_API_KEY"
        ),
        "" => format!("roder.cloud error (HTTP {status}): {message}"),
        _ => format!("roder.cloud error ({code}, HTTP {status}): {message}"),
    }
}

/// Flatten the transcript into the `role: text` lines the godex `/v1/messages`
/// shim also uses. A lone user message is sent raw for cleaner prompts.
fn flatten_input(request: &AgentInferenceRequest) -> String {
    use roder_api::transcript::TranscriptItem;
    let lines = request
        .transcript
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::UserMessage(message) => Some(("user", message.text.as_str())),
            TranscriptItem::AssistantMessage(message) => Some(("assistant", message.text.as_str())),
            TranscriptItem::ContextCompaction(compaction) => {
                Some(("summary", compaction.summary.as_str()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    match lines.as_slice() {
        [("user", text)] => (*text).to_string(),
        lines => lines
            .iter()
            .map(|(role, text)| format!("{role}: {text}"))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn output_text(payload: &ResponsesPayload) -> String {
    if !payload.output_text.is_empty() {
        return payload.output_text.clone();
    }
    payload
        .output
        .iter()
        .flat_map(|item| item.content.iter())
        .map(|content| content.text.as_str())
        .collect::<Vec<_>>()
        .join("")
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[async_trait::async_trait]
impl InferenceEngine for RoderCloudEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_RODER_CLOUD.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            // The engine surfaces an event stream even though the edge is
            // synchronous: deltas are synthesized from the completed body.
            streaming: true,
            tool_calls: false,
            parallel_tool_calls: false,
            reasoning_summaries: false,
            structured_output: false,
            image_input: false,
            prompt_cache: false,
            provider_metadata: false,
            tool_search: false,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "Roder Cloud".to_string(),
            description: Some(
                "roder.cloud hosted models (roder_ API key with JWT exchange)".to_string(),
            ),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some("RODER_CLOUD_API_KEY".to_string()),
            auth_configured: Some(self.token_source.is_some()),
            recommended: false,
            sort_order: 19,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        let catalog = models_for_provider(PROVIDER_RODER_CLOUD, false);
        let (Some(base_url), Some(token_source)) = (self.base_url.as_deref(), &self.token_source)
        else {
            return Ok(catalog);
        };
        let Ok(token) = token_source.token().await else {
            return Ok(catalog);
        };
        let url = format!("{base_url}/models");
        let response = self.client.get(&url).bearer_auth(&token).send().await;
        let Ok(response) = response else {
            return Ok(catalog);
        };
        if !response.status().is_success() {
            return Ok(catalog);
        }
        let Ok(payload) = response.json::<ModelsPayload>().await else {
            return Ok(catalog);
        };
        let models = payload
            .data
            .into_iter()
            .filter(|entry| !entry.id.trim().is_empty())
            .map(|entry| {
                catalog
                    .iter()
                    .find(|model| model.id == entry.id)
                    .cloned()
                    .unwrap_or(ModelDescriptor {
                        id: entry.id.clone(),
                        name: entry.id,
                        context_window: None,
                        default_reasoning: None,
                        supported_reasoning: Vec::new(),
                    })
            })
            .collect::<Vec<_>>();
        if models.is_empty() {
            return Ok(catalog);
        }
        Ok(models)
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let body = Self::map_request(&request);
        let payload = self.send_turn(&body).await?;
        let text = output_text(&payload);
        let mut events = Vec::new();
        if !text.is_empty() {
            events.push(Ok(InferenceEvent::MessageDelta(MessageDelta {
                text,
                phase: None,
            })));
        }
        let usage = TokenUsage::new(
            payload.usage.input_tokens,
            payload.usage.output_tokens,
            payload.usage.total_tokens,
        );
        if !usage.is_empty() {
            events.push(Ok(InferenceEvent::Usage(usage)));
        }
        events.push(Ok(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some("stop".to_string()),
            provider_response_id: (!payload.id.is_empty()).then(|| payload.id.clone()),
        })));
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

#[cfg(test)]
mod tests;
