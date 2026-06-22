use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_stream::try_stream;
use claude_code_sdk_rust::{ClaudeAgentClient, ClaudeAgentOptions, MessageResponse, StreamEvent};
use roder_api::catalog::{PROVIDER_CLAUDE_CODE, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceFailure, InferenceProviderContext,
    InferenceProviderMetadata, InferenceTurnContext, MessageDelta, ModelDescriptor,
    ProviderAuthType, ReasoningDelta, TokenUsage, ToolCallCompleted, ToolCallDelta,
};
use roder_api::transcript::TranscriptItem;
use serde_json::json;
use tokio::sync::mpsc;

use crate::options::build_options;

#[derive(Debug, Clone, Default)]
pub struct ClaudeCodeConfig {
    pub cli_path: Option<String>,
    pub permission_mode: Option<String>,
    pub setting_sources: Option<Vec<String>>,
    pub workspace: Option<PathBuf>,
    /// Reuse the persisted `claude` CLI session across turns (default: on) so
    /// the CLI keeps history server-side and auto-compacts it, and Roder only
    /// sends the new transcript tail each turn. Set to `Some(false)` to force
    /// the legacy behavior of replaying the full transcript every turn.
    pub reuse_cli_session: Option<bool>,
}

impl ClaudeCodeConfig {
    fn session_reuse_enabled(&self) -> bool {
        self.reuse_cli_session.unwrap_or(true)
    }
}

#[async_trait::async_trait]
pub trait ClaudeCodeRunner: Send + Sync {
    async fn stream(
        &self,
        options: ClaudeAgentOptions,
        prompt: String,
    ) -> anyhow::Result<mpsc::UnboundedReceiver<StreamEvent>>;
}

#[derive(Debug, Default)]
pub struct SdkClaudeCodeRunner;

#[async_trait::async_trait]
impl ClaudeCodeRunner for SdkClaudeCodeRunner {
    async fn stream(
        &self,
        options: ClaudeAgentOptions,
        prompt: String,
    ) -> anyhow::Result<mpsc::UnboundedReceiver<StreamEvent>> {
        Ok(ClaudeAgentClient::spawn_stream_message(options, prompt))
    }
}

/// Tracks the persisted `claude` CLI session so successive turns resume it
/// instead of replaying the whole transcript. `synced` holds a per-item
/// fingerprint of the transcript prefix the CLI session is already known to
/// contain (everything up to and including the last assistant message of the
/// previous turn). If a later request no longer extends that prefix — e.g.
/// Roder compacted the transcript or a brand-new conversation started — the
/// fingerprint check fails and the provider falls back to a fresh full send.
#[derive(Debug, Default)]
struct SessionContinuity {
    session_id: Option<String>,
    synced: Vec<u64>,
}

pub struct ClaudeCodeEngine {
    config: ClaudeCodeConfig,
    runner: Arc<dyn ClaudeCodeRunner>,
    continuity: Arc<Mutex<SessionContinuity>>,
}

impl ClaudeCodeEngine {
    pub fn new(config: ClaudeCodeConfig) -> Self {
        Self::new_with_runner(config, Arc::new(SdkClaudeCodeRunner))
    }

    pub fn new_with_runner(config: ClaudeCodeConfig, runner: Arc<dyn ClaudeCodeRunner>) -> Self {
        Self {
            config,
            runner,
            continuity: Arc::new(Mutex::new(SessionContinuity::default())),
        }
    }

    fn auth_configured(&self) -> bool {
        self.config
            .cli_path
            .as_deref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or_else(|| which::which("claude").is_ok())
    }
}

#[async_trait::async_trait]
impl InferenceEngine for ClaudeCodeEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_CLAUDE_CODE.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: false,
            reasoning_summaries: true,
            structured_output: false,
            image_input: true,
            prompt_cache: false,
            provider_metadata: true,
            tool_search: false,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "Claude Code".to_string(),
            description: Some("Local Claude Code CLI harness via claude-agent-sdk".to_string()),
            auth_type: ProviderAuthType::None,
            auth_label: Some("Authenticated local Claude Code CLI".to_string()),
            auth_configured: Some(self.auth_configured()),
            recommended: true,
            sort_order: 19,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_CLAUDE_CODE, false))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        validate_request(&request)?;
        let current_dir = std::env::current_dir().ok();
        let cwd = self
            .config
            .workspace
            .as_deref()
            .or_else(|| current_dir.as_deref());
        let plan = self.plan_turn(&request);
        let options = build_options(
            &self.config,
            &request,
            _ctx.tool_executor.clone(),
            cwd,
            plan.resume_session_id.as_deref(),
        )?;
        let events = self.runner.stream(options, plan.prompt).await?;
        Ok(Box::pin(map_stream_events(
            events,
            Arc::clone(&self.continuity),
            plan.synced,
        )))
    }
}

/// Outcome of deciding whether the upcoming turn resumes the persisted CLI
/// session (sending only the new transcript tail) or starts fresh (sending the
/// whole transcript).
struct TurnPlan {
    resume_session_id: Option<String>,
    prompt: String,
    /// Fingerprint of the transcript prefix the session will be known to
    /// contain once this turn is sent. Committed to `SessionContinuity` only
    /// after the turn completes successfully.
    synced: Vec<u64>,
}

impl ClaudeCodeEngine {
    fn plan_turn(&self, request: &AgentInferenceRequest) -> TurnPlan {
        let fingerprints = transcript_fingerprints(&request.transcript);
        // The CLI session always contains everything up to and including the
        // previous turn's final assistant message; only items after it are new
        // input the resumed session has not seen yet.
        let boundary = delta_boundary(&request.transcript);

        if self.config.session_reuse_enabled()
            && let Ok(state) = self.continuity.lock()
        {
            let can_resume = state.session_id.is_some()
                && boundary > 0
                && is_prefix(&state.synced, &fingerprints)
                && boundary <= request.transcript.len();
            if can_resume {
                let delta = prompt_from_delta(request, boundary);
                if !delta.trim().is_empty() {
                    return TurnPlan {
                        resume_session_id: state.session_id.clone(),
                        prompt: delta,
                        // After this resume the session covers the full current
                        // transcript (prior items + this new tail).
                        synced: fingerprints,
                    };
                }
            }
        }

        // Fresh send: replay the whole transcript and start a new session. The
        // synced baseline becomes the current transcript so the next turn can
        // resume from here.
        TurnPlan {
            resume_session_id: None,
            prompt: prompt_from_request(request),
            synced: fingerprints,
        }
    }
}

fn validate_request(request: &AgentInferenceRequest) -> anyhow::Result<()> {
    if request.output.response_format.is_some() {
        anyhow::bail!("Claude Code provider does not support structured response_format yet");
    }
    Ok(())
}

fn prompt_from_request(request: &AgentInferenceRequest) -> String {
    prompt_from_items(request, &request.transcript)
}

/// Builds the prompt from only the transcript items at/after `boundary`, used
/// when resuming a persisted CLI session that already holds the earlier items.
fn prompt_from_delta(request: &AgentInferenceRequest, boundary: usize) -> String {
    let tail = request.transcript.get(boundary..).unwrap_or(&[]);
    prompt_from_items(request, tail)
}

fn prompt_from_items(request: &AgentInferenceRequest, items: &[TranscriptItem]) -> String {
    let mut parts = Vec::new();
    for item in items {
        // Provider metadata (rate-limit blobs, tool-result echoes) is internal
        // bookkeeping, not conversational input. Never replay it as a prompt.
        if matches!(item, TranscriptItem::ProviderMetadata(_)) {
            continue;
        }
        parts.push(format!("{item:?}"));
    }
    if let Some(value) = request
        .metadata
        .get("prompt")
        .and_then(|value| value.as_str())
    {
        parts.push(value.to_string());
    }
    if parts.is_empty() {
        "Continue the current Roder turn.".to_string()
    } else {
        parts.join("\n\n")
    }
}

/// Index of the first transcript item the CLI session has NOT seen yet: the
/// position just after the previous turn's final assistant message. Returns 0
/// when there is no prior assistant message (first turn -> send everything).
fn delta_boundary(transcript: &[TranscriptItem]) -> usize {
    transcript
        .iter()
        .rposition(|item| matches!(item, TranscriptItem::AssistantMessage(_)))
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn transcript_fingerprints(transcript: &[TranscriptItem]) -> Vec<u64> {
    transcript.iter().map(fingerprint_item).collect()
}

fn fingerprint_item(item: &TranscriptItem) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{item:?}").hash(&mut hasher);
    hasher.finish()
}

/// True when `prefix` is a (non-strict) leading slice of `full`.
fn is_prefix(prefix: &[u64], full: &[u64]) -> bool {
    prefix.len() <= full.len() && full[..prefix.len()] == *prefix
}

fn map_stream_events(
    mut events: mpsc::UnboundedReceiver<StreamEvent>,
    continuity: Arc<Mutex<SessionContinuity>>,
    synced: Vec<u64>,
) -> InferenceEventStream {
    Box::pin(try_stream! {
        let mut saw_partial_text = false;
        let mut accumulated_text = String::new();
        // Tracks how far into `accumulated_text` the trailing full-message echo
        // has been re-matched. Under `include_partial_messages`, the SDK streams
        // incremental `content_block_delta` chunks and then re-emits the full
        // text of every block from the final `AssistantMsg`. Those echoes replay
        // the already-streamed text from the start, one block at a time, so we
        // walk `accumulated_text` and drop each chunk that matches. Reset to 0 on
        // any genuinely new text, since echoes only ever arrive contiguously at
        // the end of the message.
        let mut echo_match_pos = 0usize;
        let mut completed = false;
        let mut last_session_id: Option<String> = None;
        let mut last_stop_reason: Option<String> = None;
        // Tool-use ids for `mcp__roder__*` tools. Those tools are executed
        // in-process by the SDK MCP handler, which routes through Roder's
        // TurnToolExecutor and emits the canonical tool-call lifecycle events.
        // The provider must NOT also surface them from the CLI stream: doing so
        // makes the runtime try to execute a tool literally named
        // `mcp__roder__read_file` (which is not registered, so it fails) and
        // produces duplicate, failing rows that trip the reliability limit.
        let mut mcp_tool_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        while let Some(event) = events.recv().await {
            match event {
                StreamEvent::ContentChunk(text) => {
                    if completed {
                        continue;
                    }
                    if !accumulated_text.is_empty()
                        && echo_match_pos < accumulated_text.len()
                        && accumulated_text[echo_match_pos..].starts_with(&text)
                    {
                        echo_match_pos += text.len();
                        continue;
                    }
                    saw_partial_text = true;
                    echo_match_pos = 0;
                    accumulated_text.push_str(&text);
                    yield InferenceEvent::MessageDelta(MessageDelta { text, phase: None });
                }
                StreamEvent::ThinkingChunk { thinking, .. } => {
                    yield InferenceEvent::ReasoningDelta(ReasoningDelta { text: thinking });
                }
                StreamEvent::ToolUseStart { id, name, input } => {
                    if name.starts_with("mcp__") {
                        // Executed in-process via the MCP handler; the executor
                        // owns the canonical tool-call events. Skip emission.
                        mcp_tool_ids.insert(id);
                        continue;
                    }
                    if !input.is_empty() {
                        yield InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                            id,
                            name,
                            arguments: serde_json::Value::Object(input).to_string(),
                        });
                    }
                }
                StreamEvent::ToolUseDelta { id, partial_input } => {
                    if mcp_tool_ids.contains(&id) {
                        continue;
                    }
                    yield InferenceEvent::ToolCallDelta(ToolCallDelta { id, arguments_delta: partial_input });
                }
                StreamEvent::ToolResult { tool_use_id, content, is_error } => {
                    yield InferenceEvent::ProviderMetadata(json!({
                        "provider": PROVIDER_CLAUDE_CODE,
                        "toolResult": {
                            "toolUseId": tool_use_id,
                            "isError": is_error.unwrap_or(false),
                            "content": content.unwrap_or(serde_json::Value::Null),
                        }
                    }));
                }
                StreamEvent::RateLimit(info) => {
                    yield InferenceEvent::ProviderMetadata(json!({
                        "provider": PROVIDER_CLAUDE_CODE,
                        "rateLimit": info,
                    }));
                }
                StreamEvent::Complete(response) => {
                    // One assistant message (or a usage-bearing message delta)
                    // finished. A turn with tool calls produces several of
                    // these — the turn only ends at TurnComplete below.
                    if completed {
                        continue;
                    }
                    if let Some(usage) = usage_from_response(&response) {
                        yield InferenceEvent::Usage(usage);
                    }
                    if response.content.is_empty() {
                        continue;
                    }
                    if !saw_partial_text {
                        yield InferenceEvent::MessageDelta(MessageDelta { text: response.content.clone(), phase: None });
                    }
                    last_session_id = Some(response.session_id);
                    last_stop_reason = response.stop_reason;
                    // Assistant-message boundary: the next message streams its
                    // own deltas and echoes, so reset the echo-dedup state.
                    accumulated_text.clear();
                    echo_match_pos = 0;
                }
                StreamEvent::TurnComplete(response) => {
                    if completed {
                        continue;
                    }
                    if let Some(usage) = usage_from_response(&response) {
                        yield InferenceEvent::Usage(usage);
                    }
                    completed = true;
                    let session_id = (!response.session_id.trim().is_empty())
                        .then_some(response.session_id)
                        .or(last_session_id.take());
                    commit_session(&continuity, session_id.as_deref(), &synced);
                    yield InferenceEvent::Completed(CompletionMetadata {
                        stop_reason: response.stop_reason.or(last_stop_reason.take()),
                        provider_response_id: session_id,
                    });
                }
                StreamEvent::Error(message) => {
                    // The turn failed (possibly a stale/invalid resume). Drop the
                    // session so the next attempt replays the full transcript.
                    clear_session(&continuity);
                    yield InferenceEvent::Failed(InferenceFailure { message: redact_error(&message) });
                }
            }
        }
        // The CLI stream ended without a result message (e.g. the process
        // died after its last assistant message). Close the turn with what
        // we saw so the runtime never hangs waiting for completion.
        if !completed && (last_session_id.is_some() || last_stop_reason.is_some()) {
            commit_session(&continuity, last_session_id.as_deref(), &synced);
            yield InferenceEvent::Completed(CompletionMetadata {
                stop_reason: last_stop_reason,
                provider_response_id: last_session_id,
            });
        }
    })
}

/// Records the resumable session id and the transcript prefix it now covers so
/// the next turn can resume instead of replaying the full transcript.
fn commit_session(
    continuity: &Arc<Mutex<SessionContinuity>>,
    session_id: Option<&str>,
    synced: &[u64],
) {
    let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) else {
        // No usable session id: force a fresh send next turn rather than
        // resuming against a session we cannot name.
        clear_session(continuity);
        return;
    };
    if let Ok(mut state) = continuity.lock() {
        state.session_id = Some(session_id.to_string());
        state.synced = synced.to_vec();
    }
}

fn clear_session(continuity: &Arc<Mutex<SessionContinuity>>) {
    if let Ok(mut state) = continuity.lock() {
        state.session_id = None;
        state.synced.clear();
    }
}

fn usage_from_response(response: &MessageResponse) -> Option<TokenUsage> {
    let usage = response.usage.as_ref()?;
    let prompt_tokens = number_from_usage(
        usage,
        &[
            "input_tokens",
            "inputTokens",
            "prompt_tokens",
            "promptTokens",
            "cache_creation_input_tokens",
            "cacheCreationInputTokens",
            "cache_read_input_tokens",
            "cacheReadInputTokens",
        ],
    )
    .unwrap_or(0);
    let cached_prompt_tokens =
        number_from_usage(usage, &["cache_read_input_tokens", "cacheReadInputTokens"]);
    let cache_creation_prompt_tokens = number_from_usage(
        usage,
        &["cache_creation_input_tokens", "cacheCreationInputTokens"],
    );
    let completion_tokens = number_from_usage(
        usage,
        &[
            "output_tokens",
            "outputTokens",
            "completion_tokens",
            "completionTokens",
        ],
    )
    .unwrap_or(0);
    let total_tokens = number_from_usage(usage, &["total_tokens", "totalTokens"])
        .unwrap_or_else(|| prompt_tokens.saturating_add(completion_tokens));
    if prompt_tokens == 0 && completion_tokens == 0 && total_tokens == 0 {
        return None;
    }
    Some(
        TokenUsage::new(prompt_tokens, completion_tokens, total_tokens)
            .with_cached_prompt_tokens(cached_prompt_tokens.unwrap_or(0))
            .with_cache_creation_prompt_tokens(cache_creation_prompt_tokens.unwrap_or(0)),
    )
}

fn number_from_usage(
    usage: &std::collections::HashMap<String, serde_json::Value>,
    keys: &[&str],
) -> Option<u32> {
    let total = usage.iter().fold(0u32, |total, (key, value)| {
        total.saturating_add(number_from_usage_value(key, value, keys))
    });
    (total > 0).then_some(total)
}

fn number_from_usage_value(key: &str, value: &serde_json::Value, keys: &[&str]) -> u32 {
    let direct = if keys.iter().any(|candidate| candidate == &key) {
        json_u32(value).unwrap_or(0)
    } else {
        0
    };
    let nested = value
        .as_object()
        .map(|object| {
            object.iter().fold(0u32, |total, (key, value)| {
                total.saturating_add(number_from_usage_value(key, value, keys))
            })
        })
        .unwrap_or(0);
    direct.saturating_add(nested)
}

fn json_u32(value: &serde_json::Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| value.as_i64().and_then(|value| u32::try_from(value).ok()))
        .or_else(|| {
            value
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .and_then(|value| u32::try_from(value as u64).ok())
        })
        .or_else(|| value.as_str().and_then(|value| value.parse::<u32>().ok()))
}

fn redact_error(message: &str) -> String {
    let mut redacted = message.to_string();
    for marker in ["ANTHROPIC_API_KEY", "CLAUDE_CODE", "Bearer "] {
        if redacted.contains(marker) {
            redacted = redacted.replace(marker, "[redacted]");
        }
    }
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_code_sdk_rust::types::ContentBlock;
    use futures::StreamExt;
    use roder_api::inference::{
        HostedWebSearchConfig, InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig,
        RuntimeHints,
    };
    use roder_api::tools::ToolChoice;

    #[derive(Default)]
    struct FakeRunner {
        events: Vec<StreamEvent>,
    }

    #[async_trait::async_trait]
    impl ClaudeCodeRunner for FakeRunner {
        async fn stream(
            &self,
            _options: ClaudeAgentOptions,
            _prompt: String,
        ) -> anyhow::Result<mpsc::UnboundedReceiver<StreamEvent>> {
            let (tx, rx) = mpsc::unbounded_channel();
            for event in self.events.clone() {
                tx.send(event).unwrap();
            }
            Ok(rx)
        }
    }

    fn request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: PROVIDER_CLAUDE_CODE.to_string(),
                model: "sonnet".to_string(),
            },
            instructions: InstructionBundle::default(),
            transcript: Vec::new(),
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig::default(),
            runtime: RuntimeHints {
                hosted_web_search: HostedWebSearchConfig::disabled(),
                ..RuntimeHints::default()
            },
            metadata: json!({"prompt": "hello"}),
        }
    }

    #[tokio::test]
    async fn provider_streams_text_and_completion() {
        let mut usage = std::collections::HashMap::new();
        usage.insert("input_tokens".to_string(), json!(3));
        usage.insert("output_tokens".to_string(), json!(5));
        let engine = ClaudeCodeEngine::new_with_runner(
            ClaudeCodeConfig::default(),
            Arc::new(FakeRunner {
                events: vec![
                    StreamEvent::ContentChunk("hello".to_string()),
                    StreamEvent::Complete(MessageResponse {
                        content: "hello".to_string(),
                        blocks: vec![ContentBlock::Text {
                            text: "hello".to_string(),
                        }],
                        model: "sonnet".to_string(),
                        stop_reason: Some("end_turn".to_string()),
                        session_id: "session-1".to_string(),
                        usage: Some(usage),
                    }),
                    StreamEvent::TurnComplete(MessageResponse {
                        content: String::new(),
                        blocks: Vec::new(),
                        model: String::new(),
                        stop_reason: Some("end_turn".to_string()),
                        session_id: "session-1".to_string(),
                        usage: None,
                    }),
                ],
            }),
        );
        let events = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread",
                    turn_id: "turn",
                    tool_executor: None,
                },
                request(),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(matches!(events[0], InferenceEvent::MessageDelta(_)));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, InferenceEvent::Usage(_)))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, InferenceEvent::Completed(_)))
        );
    }

    #[tokio::test]
    async fn provider_keeps_partial_text_and_uses_later_usage_completion() {
        let mut usage = std::collections::HashMap::new();
        usage.insert("input_tokens".to_string(), json!(4));
        usage.insert("cache_read_input_tokens".to_string(), json!(6));
        usage.insert("output_tokens".to_string(), json!(8));
        let engine = ClaudeCodeEngine::new_with_runner(
            ClaudeCodeConfig::default(),
            Arc::new(FakeRunner {
                events: vec![
                    StreamEvent::ContentChunk("one ".to_string()),
                    StreamEvent::ContentChunk("two".to_string()),
                    StreamEvent::Complete(MessageResponse {
                        content: String::new(),
                        blocks: Vec::new(),
                        model: String::new(),
                        stop_reason: Some("end_turn".to_string()),
                        session_id: "session-usage".to_string(),
                        usage: Some(usage),
                    }),
                    StreamEvent::ContentChunk("one two".to_string()),
                    StreamEvent::Complete(MessageResponse {
                        content: "one two".to_string(),
                        blocks: Vec::new(),
                        model: "sonnet".to_string(),
                        stop_reason: Some("end_turn".to_string()),
                        session_id: "session-final".to_string(),
                        usage: None,
                    }),
                    StreamEvent::TurnComplete(MessageResponse {
                        content: String::new(),
                        blocks: Vec::new(),
                        model: String::new(),
                        stop_reason: Some("end_turn".to_string()),
                        session_id: "session-final".to_string(),
                        usage: None,
                    }),
                ],
            }),
        );
        let events = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread",
                    turn_id: "turn",
                    tool_executor: None,
                },
                request(),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let rendered = events
            .iter()
            .filter_map(|event| match event {
                InferenceEvent::MessageDelta(delta) => Some(delta.text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(rendered, "one two");
        assert!(events.iter().any(|event| matches!(event, InferenceEvent::Usage(usage) if usage.prompt_tokens == 10 && usage.completion_tokens == 8)));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, InferenceEvent::Completed(_)))
        );
    }

    /// Mirrors the real SDK sequence under `include_partial_messages(true)`:
    /// the CLI first streams incremental `content_block_delta` chunks, then the
    /// final `AssistantMsg` re-emits the FULL text of every text block. When an
    /// assistant message contains more than one text block (text -> tool_use ->
    /// text), the redundant full-block echoes must not be rendered again.
    #[tokio::test]
    async fn provider_does_not_duplicate_multi_block_commentary() {
        let engine = ClaudeCodeEngine::new_with_runner(
            ClaudeCodeConfig::default(),
            Arc::new(FakeRunner {
                events: vec![
                    // Incremental deltas as the message streams in.
                    StreamEvent::ContentChunk("First ".to_string()),
                    StreamEvent::ContentChunk("block.".to_string()),
                    StreamEvent::ContentChunk("Second ".to_string()),
                    StreamEvent::ContentChunk("block.".to_string()),
                    // Final AssistantMsg re-emits the full text of each block.
                    StreamEvent::ContentChunk("First block.".to_string()),
                    StreamEvent::ContentChunk("Second block.".to_string()),
                    StreamEvent::Complete(MessageResponse {
                        content: "First block.Second block.".to_string(),
                        blocks: Vec::new(),
                        model: "sonnet".to_string(),
                        stop_reason: Some("end_turn".to_string()),
                        session_id: "session-final".to_string(),
                        usage: None,
                    }),
                    StreamEvent::TurnComplete(MessageResponse {
                        content: String::new(),
                        blocks: Vec::new(),
                        model: String::new(),
                        stop_reason: Some("end_turn".to_string()),
                        session_id: "session-final".to_string(),
                        usage: None,
                    }),
                ],
            }),
        );
        let events = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread",
                    turn_id: "turn",
                    tool_executor: None,
                },
                request(),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let rendered = events
            .iter()
            .filter_map(|event| match event {
                InferenceEvent::MessageDelta(delta) => Some(delta.text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(rendered, "First block.Second block.");
    }

    /// Fable 5 (and Opus 4.8) narrate before tool calls. A turn then contains
    /// MULTIPLE assistant messages: "I'll read the file." -> tool_use ->
    /// "The answer is X." Each assistant message emits its own Complete; the
    /// turn must only end at TurnComplete, and the post-tool text must not be
    /// dropped.
    #[tokio::test]
    async fn provider_streams_text_after_tool_use_until_turn_complete() {
        let engine = ClaudeCodeEngine::new_with_runner(
            ClaudeCodeConfig::default(),
            Arc::new(FakeRunner {
                events: vec![
                    // First assistant message: pre-tool narration.
                    StreamEvent::ContentChunk("I'll read the file.".to_string()),
                    // Echo of the full block from the final AssistantMsg.
                    StreamEvent::ContentChunk("I'll read the file.".to_string()),
                    StreamEvent::Complete(MessageResponse {
                        content: "I'll read the file.".to_string(),
                        blocks: Vec::new(),
                        model: "claude-fable-5".to_string(),
                        stop_reason: None,
                        session_id: "session-1".to_string(),
                        usage: None,
                    }),
                    // Tool runs (mcp tool -- suppressed lifecycle).
                    StreamEvent::ToolUseStart {
                        id: "toolu_1".to_string(),
                        name: "mcp__roder__read_file".to_string(),
                        input: serde_json::Map::new(),
                    },
                    // Second assistant message: the actual answer.
                    StreamEvent::ContentChunk("The magic word is pomegranate.".to_string()),
                    StreamEvent::ContentChunk("The magic word is pomegranate.".to_string()),
                    StreamEvent::Complete(MessageResponse {
                        content: "The magic word is pomegranate.".to_string(),
                        blocks: Vec::new(),
                        model: "claude-fable-5".to_string(),
                        stop_reason: Some("end_turn".to_string()),
                        session_id: "session-1".to_string(),
                        usage: None,
                    }),
                    StreamEvent::TurnComplete(MessageResponse {
                        content: String::new(),
                        blocks: Vec::new(),
                        model: String::new(),
                        stop_reason: Some("end_turn".to_string()),
                        session_id: "session-1".to_string(),
                        usage: None,
                    }),
                ],
            }),
        );
        let events = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread",
                    turn_id: "turn",
                    tool_executor: None,
                },
                request(),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let rendered = events
            .iter()
            .filter_map(|event| match event {
                InferenceEvent::MessageDelta(delta) => Some(delta.text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(
            rendered, "I'll read the file.The magic word is pomegranate.",
            "post-tool assistant text must stream and echoes must dedupe"
        );
        let completions = events
            .iter()
            .filter(|event| matches!(event, InferenceEvent::Completed(_)))
            .count();
        assert_eq!(completions, 1, "turn must complete exactly once");
    }

    #[tokio::test]
    async fn provider_completes_turn_when_stream_ends_without_result() {
        let engine = ClaudeCodeEngine::new_with_runner(
            ClaudeCodeConfig::default(),
            Arc::new(FakeRunner {
                events: vec![
                    StreamEvent::ContentChunk("partial".to_string()),
                    StreamEvent::Complete(MessageResponse {
                        content: "partial".to_string(),
                        blocks: Vec::new(),
                        model: "claude-fable-5".to_string(),
                        stop_reason: Some("end_turn".to_string()),
                        session_id: "session-dead".to_string(),
                        usage: None,
                    }),
                    // CLI dies here -- no TurnComplete.
                ],
            }),
        );
        let events = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread",
                    turn_id: "turn",
                    tool_executor: None,
                },
                request(),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, InferenceEvent::Completed(_))),
            "stream end without a result message must still complete the turn"
        );
    }

    #[tokio::test]
    async fn provider_maps_tool_events() {
        let engine = ClaudeCodeEngine::new_with_runner(
            ClaudeCodeConfig::default(),
            Arc::new(FakeRunner {
                events: vec![
                    StreamEvent::ToolUseStart {
                        id: "toolu_1".to_string(),
                        name: "Read".to_string(),
                        input: serde_json::Map::from_iter([(
                            "path".to_string(),
                            json!("crates/roder-ext-claude-code"),
                        )]),
                    },
                    StreamEvent::ToolUseDelta {
                        id: "toolu_1".to_string(),
                        partial_input: "{\"file_path\"".to_string(),
                    },
                ],
            }),
        );
        let events = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread",
                    turn_id: "turn",
                    tool_executor: None,
                },
                request(),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            matches!(&events[0], InferenceEvent::ToolCallCompleted(call) if call.arguments.contains("roder-ext-claude-code"))
        );
        assert!(matches!(events[1], InferenceEvent::ToolCallDelta(_)));
    }

    /// `mcp__roder__*` tools are executed in-process by the SDK MCP handler,
    /// which routes through Roder's executor and emits the canonical tool-call
    /// events. The provider must NOT re-surface those tool calls from the CLI
    /// stream, or the runtime would try to execute an unregistered tool named
    /// `mcp__roder__read_file` and record a spurious failure.
    #[tokio::test]
    async fn provider_suppresses_mcp_tool_lifecycle_events() {
        let engine = ClaudeCodeEngine::new_with_runner(
            ClaudeCodeConfig::default(),
            Arc::new(FakeRunner {
                events: vec![
                    StreamEvent::ToolUseStart {
                        id: "toolu_mcp".to_string(),
                        name: "mcp__roder__read_file".to_string(),
                        input: serde_json::Map::from_iter([(
                            "path".to_string(),
                            json!("README.md"),
                        )]),
                    },
                    StreamEvent::ToolUseDelta {
                        id: "toolu_mcp".to_string(),
                        partial_input: "{\"path\"".to_string(),
                    },
                    StreamEvent::Complete(MessageResponse {
                        content: "done".to_string(),
                        blocks: Vec::new(),
                        model: "sonnet".to_string(),
                        stop_reason: Some("end_turn".to_string()),
                        session_id: "session".to_string(),
                        usage: None,
                    }),
                ],
            }),
        );
        let events = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread",
                    turn_id: "turn",
                    tool_executor: None,
                },
                request(),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            !events.iter().any(|event| matches!(
                event,
                InferenceEvent::ToolCallCompleted(_) | InferenceEvent::ToolCallDelta(_)
            )),
            "mcp__ tool lifecycle events must not be surfaced from the CLI stream"
        );
    }

    #[test]
    fn usage_parser_reports_cached_prompt_tokens_as_subset() {
        let usage = std::collections::HashMap::from([
            ("input_tokens".to_string(), json!(123)),
            ("cache_creation_input_tokens".to_string(), json!(7)),
            ("cache_read_input_tokens".to_string(), json!(11)),
            ("output_tokens".to_string(), json!(13)),
        ]);
        let response = MessageResponse {
            content: String::new(),
            blocks: Vec::new(),
            model: "sonnet".to_string(),
            stop_reason: Some("end_turn".to_string()),
            session_id: "session".to_string(),
            usage: Some(usage),
        };

        let usage = usage_from_response(&response).unwrap();
        assert_eq!(usage.prompt_tokens, 141);
        assert_eq!(usage.cached_prompt_tokens, 11);
        assert_eq!(usage.cache_creation_prompt_tokens, 7);
        assert_eq!(usage.completion_tokens, 13);
        assert_eq!(usage.total_tokens, 154);
    }

    #[test]
    fn usage_parser_includes_cache_only_input_tokens_in_prompt_total() {
        let usage = std::collections::HashMap::from([
            ("cache_creation_input_tokens".to_string(), json!(7)),
            ("cache_read_input_tokens".to_string(), json!(11)),
            ("output_tokens".to_string(), json!(13)),
        ]);
        let response = MessageResponse {
            content: String::new(),
            blocks: Vec::new(),
            model: "sonnet".to_string(),
            stop_reason: Some("end_turn".to_string()),
            session_id: "session".to_string(),
            usage: Some(usage),
        };

        let usage = usage_from_response(&response).unwrap();
        assert_eq!(usage.prompt_tokens, 18);
        assert_eq!(usage.cached_prompt_tokens, 11);
        assert_eq!(usage.cache_creation_prompt_tokens, 7);
        assert_eq!(usage.completion_tokens, 13);
        assert_eq!(usage.total_tokens, 31);
    }

    #[test]
    fn usage_parser_accepts_output_only_delta_usage() {
        let response = MessageResponse {
            content: String::new(),
            blocks: Vec::new(),
            model: "sonnet".to_string(),
            stop_reason: Some("end_turn".to_string()),
            session_id: "session".to_string(),
            usage: Some(std::collections::HashMap::from([(
                "output_tokens".to_string(),
                json!(42),
            )])),
        };

        let usage = usage_from_response(&response).unwrap();
        assert_eq!(usage.prompt_tokens, 0);
        assert_eq!(usage.completion_tokens, 42);
        assert_eq!(usage.total_tokens, 42);
    }

    #[test]
    fn usage_parser_accepts_nested_camel_case_model_usage() {
        let response = MessageResponse {
            content: String::new(),
            blocks: Vec::new(),
            model: "sonnet".to_string(),
            stop_reason: Some("end_turn".to_string()),
            session_id: "session".to_string(),
            usage: Some(std::collections::HashMap::from([(
                "modelUsage".to_string(),
                json!({
                    "inputTokens": "100",
                    "cacheReadInputTokens": 25.0,
                    "outputTokens": 7,
                }),
            )])),
        };

        let usage = usage_from_response(&response).unwrap();
        assert_eq!(usage.prompt_tokens, 125);
        assert_eq!(usage.cached_prompt_tokens, 25);
        assert_eq!(usage.completion_tokens, 7);
        assert_eq!(usage.total_tokens, 132);
    }

    use roder_api::transcript::{AssistantMessage, UserMessage};
    use std::sync::Mutex as StdMutex;

    /// `(resume_session_id, prompt)` captured for each CLI invocation.
    type RecordedCall = (Option<String>, String);

    /// Records the `resume` id and prompt of every CLI invocation and always
    /// completes the turn with the configured session id.
    #[derive(Default)]
    struct RecordingRunner {
        session_id: String,
        calls: Arc<StdMutex<Vec<RecordedCall>>>,
    }

    #[async_trait::async_trait]
    impl ClaudeCodeRunner for RecordingRunner {
        async fn stream(
            &self,
            options: ClaudeAgentOptions,
            prompt: String,
        ) -> anyhow::Result<mpsc::UnboundedReceiver<StreamEvent>> {
            self.calls
                .lock()
                .unwrap()
                .push((options.resume.clone(), prompt));
            let (tx, rx) = mpsc::unbounded_channel();
            tx.send(StreamEvent::Complete(MessageResponse {
                content: "ok".to_string(),
                blocks: Vec::new(),
                model: "sonnet".to_string(),
                stop_reason: Some("end_turn".to_string()),
                session_id: self.session_id.clone(),
                usage: None,
            }))
            .unwrap();
            tx.send(StreamEvent::TurnComplete(MessageResponse {
                content: String::new(),
                blocks: Vec::new(),
                model: String::new(),
                stop_reason: Some("end_turn".to_string()),
                session_id: self.session_id.clone(),
                usage: None,
            }))
            .unwrap();
            Ok(rx)
        }
    }

    fn request_with_transcript(transcript: Vec<TranscriptItem>) -> AgentInferenceRequest {
        AgentInferenceRequest {
            transcript,
            metadata: json!({}),
            ..request()
        }
    }

    async fn drain(engine: &ClaudeCodeEngine, request: AgentInferenceRequest) {
        let _ = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread",
                    turn_id: "turn",
                    tool_executor: None,
                },
                request,
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
    }

    #[tokio::test]
    async fn second_turn_resumes_session_and_sends_only_the_new_tail() {
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let engine = ClaudeCodeEngine::new_with_runner(
            ClaudeCodeConfig::default(),
            Arc::new(RecordingRunner {
                session_id: "session-1".to_string(),
                calls: Arc::clone(&calls),
            }),
        );

        // Turn 1: only a user message -> fresh send, no resume.
        drain(
            &engine,
            request_with_transcript(vec![TranscriptItem::UserMessage(UserMessage::text(
                "first question",
            ))]),
        )
        .await;

        // Turn 2: prior exchange plus a new user message. The session already
        // holds everything up to the assistant reply, so only the new user
        // message should be sent, and the call must resume session-1.
        drain(
            &engine,
            request_with_transcript(vec![
                TranscriptItem::UserMessage(UserMessage::text("first question")),
                TranscriptItem::AssistantMessage(AssistantMessage {
                    text: "first answer".to_string(),
                    phase: None,
                }),
                TranscriptItem::UserMessage(UserMessage::text("second question")),
            ]),
        )
        .await;

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        // Turn 1: fresh, replays the whole transcript.
        assert_eq!(calls[0].0, None);
        assert!(calls[0].1.contains("first question"));
        // Turn 2: resumes and sends ONLY the new tail.
        assert_eq!(calls[1].0.as_deref(), Some("session-1"));
        assert!(calls[1].1.contains("second question"));
        assert!(
            !calls[1].1.contains("first answer"),
            "resumed turn must not replay prior assistant output: {}",
            calls[1].1
        );
        assert!(
            !calls[1].1.contains("first question"),
            "resumed turn must not replay the prior user message: {}",
            calls[1].1
        );
    }

    #[tokio::test]
    async fn compacted_transcript_falls_back_to_a_fresh_send() {
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let engine = ClaudeCodeEngine::new_with_runner(
            ClaudeCodeConfig::default(),
            Arc::new(RecordingRunner {
                session_id: "session-1".to_string(),
                calls: Arc::clone(&calls),
            }),
        );

        drain(
            &engine,
            request_with_transcript(vec![
                TranscriptItem::UserMessage(UserMessage::text("original question")),
                TranscriptItem::AssistantMessage(AssistantMessage {
                    text: "original answer".to_string(),
                    phase: None,
                }),
            ]),
        )
        .await;

        // Roder compacted the head: the prefix no longer matches what the
        // session was synced to, so the next turn must NOT resume.
        drain(
            &engine,
            request_with_transcript(vec![
                TranscriptItem::ContextCompaction(roder_api::transcript::ContextCompactionRecord {
                    summary: "summary of earlier turns".to_string(),
                }),
                TranscriptItem::UserMessage(UserMessage::text("next question")),
            ]),
        )
        .await;

        let calls = calls.lock().unwrap();
        assert_eq!(calls[1].0, None, "compacted transcript must send fresh");
        assert!(calls[1].1.contains("summary of earlier turns"));
    }

    #[tokio::test]
    async fn session_reuse_can_be_disabled_via_config() {
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let engine = ClaudeCodeEngine::new_with_runner(
            ClaudeCodeConfig {
                reuse_cli_session: Some(false),
                ..ClaudeCodeConfig::default()
            },
            Arc::new(RecordingRunner {
                session_id: "session-1".to_string(),
                calls: Arc::clone(&calls),
            }),
        );

        drain(
            &engine,
            request_with_transcript(vec![TranscriptItem::UserMessage(UserMessage::text("one"))]),
        )
        .await;
        drain(
            &engine,
            request_with_transcript(vec![
                TranscriptItem::UserMessage(UserMessage::text("one")),
                TranscriptItem::AssistantMessage(AssistantMessage {
                    text: "answer".to_string(),
                    phase: None,
                }),
                TranscriptItem::UserMessage(UserMessage::text("two")),
            ]),
        )
        .await;

        let calls = calls.lock().unwrap();
        assert!(
            calls.iter().all(|(resume, _)| resume.is_none()),
            "reuse disabled: no turn should resume a session"
        );
        // Full transcript replayed every turn.
        assert!(calls[1].1.contains("answer"));
    }

    #[test]
    fn claude_code_engine_supports_images() {
        let engine = ClaudeCodeEngine::new(ClaudeCodeConfig::default());
        assert!(engine.capabilities().image_input);

        let models = roder_api::catalog::models_for_provider(PROVIDER_CLAUDE_CODE, false);
        assert!(!models.is_empty());
        for model in models {
            let entry = roder_api::catalog::lookup_model_for_provider(PROVIDER_CLAUDE_CODE, &model.id).unwrap();
            assert!(entry.supports_images, "model {} must support images", model.id);
        }
    }
}
