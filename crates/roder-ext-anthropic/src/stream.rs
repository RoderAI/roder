use std::collections::VecDeque;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;
use roder_api::inference::{InferenceEvent, InferenceEventStream};
use roder_api::reliability::{
    ReliabilityRequestPolicy, provider_retry_delay_ms, provider_retry_metadata,
    provider_retry_status_cause,
};
use roder_api::tools::ToolSpec;
use serde_json::Value;

use crate::sse::{AnthropicStreamState, MAX_FRAME_BYTES, SseFrameBuffer};

pub(crate) const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const STREAM_IDLE_TIMEOUT_ENV: &str = "RODER_ANTHROPIC_STREAM_IDLE_TIMEOUT_MS";

/**
 * The read timeout turns a stalled-but-TCP-alive connection into a stream
 * error: interactive profiles run without a turn deadline, so the host can
 * only recover when the stream itself errors. Anthropic sends periodic ping
 * frames, so a healthy stream never goes silent for the full timeout.
 */
pub(crate) fn anthropic_stream_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .read_timeout(anthropic_stream_idle_timeout())
        .build()?)
}

fn anthropic_stream_idle_timeout() -> Duration {
    std::env::var(STREAM_IDLE_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_STREAM_IDLE_TIMEOUT)
}

pub(crate) struct AnthropicTurnRequest {
    pub(crate) client: reqwest::Client,
    pub(crate) url: String,
    pub(crate) api_key: String,
    pub(crate) betas: Vec<String>,
    pub(crate) body: Value,
    pub(crate) policy: ReliabilityRequestPolicy,
}

impl AnthropicTurnRequest {
    fn max_attempts(&self) -> u32 {
        self.policy.provider_retry_max_attempts.max(1)
    }

    /**
     * Sends the request, retrying retryable HTTP statuses and transport
     * errors. `attempt` counts attempts across the whole turn so stream-level
     * retries in `stream_anthropic_turn` share the same budget.
     */
    async fn send(
        &self,
        attempt: &mut u32,
        retry_events: &mut Vec<Value>,
    ) -> anyhow::Result<reqwest::Response> {
        let attempts = self.max_attempts();
        let mut last_error = None;
        while *attempt < attempts {
            *attempt += 1;
            let mut builder = self
                .client
                .post(&self.url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01");
            if !self.betas.is_empty() {
                builder = builder.header("anthropic-beta", self.betas.join(","));
            }
            let response = builder.json(&self.body).send().await;
            match response {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    last_error = Some(anyhow::anyhow!("Anthropic error {status}: {text}"));
                    let retryable = self
                        .policy
                        .provider_retry_status_codes
                        .contains(&status.as_u16());
                    if !retryable {
                        break;
                    }
                    if *attempt < attempts {
                        retry_events.push(provider_retry_metadata(
                            *attempt,
                            &provider_retry_status_cause(status.as_u16()),
                            &self.policy,
                        ));
                        retry_sleep(&self.policy, *attempt).await;
                    }
                }
                Err(err) => {
                    last_error = Some(err.into());
                    if *attempt < attempts {
                        retry_events.push(provider_retry_metadata(
                            *attempt,
                            "transport_error",
                            &self.policy,
                        ));
                        retry_sleep(&self.policy, *attempt).await;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Anthropic request failed")))
    }
}

/// Sends the first request and returns the retrying event stream for the turn.
pub(crate) async fn start_anthropic_stream(
    turn: AnthropicTurnRequest,
    tools: Vec<ToolSpec>,
) -> anyhow::Result<InferenceEventStream> {
    let mut attempt = 0;
    let mut retry_events = Vec::new();
    let response = turn.send(&mut attempt, &mut retry_events).await?;
    Ok(stream_anthropic_turn(
        turn,
        response,
        attempt,
        retry_events,
        tools,
    ))
}

/**
 * Drives the SSE stream with provider-side retries that are only taken while
 * nothing has been emitted yet: an in-stream `error` frame, an empty body, or
 * a stream that dies before its first event can be replayed without
 * duplicating transcript content. Past the first event, failures surface
 * mid-stream — `error` frames as a terminal `InferenceEvent::Failed`,
 * truncation and transport failures as stream errors.
 */
fn stream_anthropic_turn(
    turn: AnthropicTurnRequest,
    first_response: reqwest::Response,
    attempts_used: u32,
    retry_events: Vec<Value>,
    tools: Vec<ToolSpec>,
) -> InferenceEventStream {
    Box::pin(async_stream::try_stream! {
        for event in retry_events {
            yield InferenceEvent::ProviderMetadata(event);
        }
        let mut attempt = attempts_used;
        let mut response = Some(first_response);
        'attempts: loop {
            let current = match response.take() {
                Some(current) => current,
                None => {
                    let mut send_retry_events = Vec::new();
                    let result = turn.send(&mut attempt, &mut send_retry_events).await;
                    for event in send_retry_events {
                        yield InferenceEvent::ProviderMetadata(event);
                    }
                    result?
                }
            };
            let mut driver = AnthropicSseDriver::new(current, tools.clone());
            // The first event decides whether retrying is still duplicate-safe.
            match driver.next_event().await {
                Some(Ok(InferenceEvent::Failed(failure))) => {
                    // In-stream error frame (e.g. overloaded_error on an HTTP
                    // 200 connection) before any content.
                    if let Some(retry) = early_retry(&turn, &mut attempt, "stream_error").await {
                        yield InferenceEvent::ProviderMetadata(retry);
                        continue 'attempts;
                    }
                    yield InferenceEvent::Failed(failure);
                    break 'attempts;
                }
                Some(Ok(event)) => {
                    yield event;
                    while let Some(event) = driver.next_event().await {
                        yield event?;
                    }
                    break 'attempts;
                }
                Some(Err(err)) => {
                    let cause = if driver.received_any_frame() {
                        "stream_closed_before_first_event"
                    } else {
                        "empty_provider_body"
                    };
                    let allowed =
                        driver.received_any_frame() || turn.policy.retry_empty_provider_body;
                    let retry = if allowed {
                        early_retry(&turn, &mut attempt, cause).await
                    } else {
                        None
                    };
                    if let Some(retry) = retry {
                        yield InferenceEvent::ProviderMetadata(retry);
                        continue 'attempts;
                    }
                    Err(err)?;
                }
                None => break 'attempts,
            }
        }
    })
}

/**
 * Returns retry metadata (after the backoff sleep) when attempt budget
 * remains, or None once it is exhausted.
 */
async fn early_retry(turn: &AnthropicTurnRequest, attempt: &mut u32, cause: &str) -> Option<Value> {
    if *attempt >= turn.max_attempts() {
        return None;
    }
    let metadata = provider_retry_metadata(*attempt, cause, &turn.policy);
    retry_sleep(&turn.policy, *attempt).await;
    Some(metadata)
}

async fn retry_sleep(policy: &ReliabilityRequestPolicy, attempt: u32) {
    let delay = provider_retry_delay_ms(policy, attempt);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
}

/**
 * Pulls response chunks and surfaces parsed inference events one at a time so
 * the caller can decide — before anything has been yielded to the host —
 * whether a failed attempt is still duplicate-safe to retry. Stops reading
 * once the stream state is terminal instead of waiting for the server to
 * close the connection.
 */
struct AnthropicSseDriver {
    chunks: BoxStream<'static, reqwest::Result<Bytes>>,
    buffer: SseFrameBuffer,
    state: AnthropicStreamState,
    pending: VecDeque<InferenceEvent>,
    pending_error: Option<anyhow::Error>,
    received_frame: bool,
    source_done: bool,
    finished: bool,
}

impl AnthropicSseDriver {
    fn new(response: reqwest::Response, tools: Vec<ToolSpec>) -> Self {
        Self {
            chunks: response.bytes_stream().boxed(),
            buffer: SseFrameBuffer::default(),
            state: AnthropicStreamState::new(tools),
            pending: VecDeque::new(),
            pending_error: None,
            received_frame: false,
            source_done: false,
            finished: false,
        }
    }

    fn received_any_frame(&self) -> bool {
        self.received_frame
    }

    async fn next_event(&mut self) -> Option<anyhow::Result<InferenceEvent>> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(Ok(event));
            }
            if let Some(err) = self.pending_error.take() {
                self.finished = true;
                return Some(Err(err));
            }
            if self.finished {
                return None;
            }
            if self.state.terminal {
                self.finished = true;
                return None;
            }
            if self.source_done {
                if let Some(frame) = self.buffer.take_trailing() {
                    self.push_frame(&frame, true);
                    continue;
                }
                self.finished = true;
                return Some(Err(anyhow::anyhow!(
                    "Anthropic stream closed before message_stop"
                )));
            }
            match self.chunks.next().await {
                None => self.source_done = true,
                Some(Err(err)) => {
                    self.finished = true;
                    return Some(Err(err.into()));
                }
                Some(Ok(chunk)) => {
                    self.buffer.push(&chunk);
                    while let Some(frame) = self.buffer.take_frame() {
                        if !self.push_frame(&frame, false) || self.state.terminal {
                            break;
                        }
                    }
                    if self.pending_error.is_none()
                        && !self.state.terminal
                        && !self.buffer.within_frame_cap()
                    {
                        self.pending_error = Some(anyhow::anyhow!(
                            "Anthropic SSE frame exceeded {MAX_FRAME_BYTES} bytes without a frame delimiter"
                        ));
                    }
                }
            }
        }
    }

    /**
     * Parses one frame into pending events; failures are queued behind
     * already-pending events so nothing parsed earlier is dropped. A parse
     * failure on the trailing frame is reported as truncation: the only way a
     * frame ends without its delimiter is the stream dying mid-frame.
     */
    fn push_frame(&mut self, frame: &str, trailing: bool) -> bool {
        self.received_frame = true;
        match self.state.push_frame(frame) {
            Ok(events) => {
                self.pending.extend(events);
                true
            }
            Err(err) => {
                self.pending_error = Some(if trailing {
                    err.context("Anthropic stream closed before message_stop")
                } else {
                    err
                });
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::inference::{
        CompletionMetadata, InferenceFailure, MessageDelta, TokenUsage, ToolCallCompleted,
        ToolCallDelta, ToolCallStarted,
    };
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn turn_request(url: &str, policy: ReliabilityRequestPolicy) -> AnthropicTurnRequest {
        AnthropicTurnRequest {
            client: reqwest::Client::new(),
            url: url.to_string(),
            api_key: "secret".to_string(),
            betas: Vec::new(),
            body: json!({}),
            policy,
        }
    }

    fn fast_policy(attempts: u32) -> ReliabilityRequestPolicy {
        ReliabilityRequestPolicy {
            provider_retry_max_attempts: attempts,
            provider_retry_initial_backoff_ms: 0,
            ..ReliabilityRequestPolicy::default()
        }
    }

    fn shell_tool() -> ToolSpec {
        ToolSpec {
            name: "shell".to_string(),
            description: "Run a shell command".to_string(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    fn happy_path_sse_body() -> String {
        [
            r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-6","stop_reason":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":1,"cache_read_input_tokens":8,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"ping"}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo "}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"shell","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"cmd\":"}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"ls\"}"}}"#,
            r#"data: {"type":"content_block_stop","index":1}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":7}}"#,
            r#"data: {"type":"message_stop"}"#,
        ]
        .map(|frame| format!("{frame}\n\n"))
        .join("")
    }

    fn happy_path_expected_events() -> Vec<InferenceEvent> {
        vec![
            InferenceEvent::MessageDelta(MessageDelta {
                text: "Hel".to_string(),
                phase: None,
            }),
            InferenceEvent::MessageDelta(MessageDelta {
                text: "lo ".to_string(),
                phase: None,
            }),
            InferenceEvent::MessageDelta(MessageDelta {
                text: "world".to_string(),
                phase: None,
            }),
            InferenceEvent::ToolCallStarted(ToolCallStarted {
                id: "toolu_1".to_string(),
                name: "shell".to_string(),
            }),
            InferenceEvent::ToolCallDelta(ToolCallDelta {
                id: "toolu_1".to_string(),
                arguments_delta: r#"{"cmd":"#.to_string(),
            }),
            InferenceEvent::ToolCallDelta(ToolCallDelta {
                id: "toolu_1".to_string(),
                arguments_delta: r#""ls"}"#.to_string(),
            }),
            InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "toolu_1".to_string(),
                name: "shell".to_string(),
                arguments: r#"{"cmd":"ls"}"#.to_string(),
            }),
            InferenceEvent::Usage(
                TokenUsage::new(11, 7, 18)
                    .with_cached_prompt_tokens(8)
                    .with_cache_creation_prompt_tokens(1),
            ),
            InferenceEvent::ProviderMetadata(json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "model": "claude-sonnet-4-6",
                "stop_reason": "tool_use",
                "stop_sequence": null,
                "content": [
                    { "type": "text", "text": "Hello world" },
                    { "type": "tool_use", "id": "toolu_1", "name": "shell", "input": { "cmd": "ls" } }
                ],
                "usage": {
                    "input_tokens": 2,
                    "cache_creation_input_tokens": 1,
                    "cache_read_input_tokens": 8,
                    "output_tokens": 7
                }
            })),
            InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("tool_use".to_string()),
                provider_response_id: Some("msg_1".to_string()),
            }),
        ]
    }

    async fn collect_events(stream: InferenceEventStream) -> Vec<anyhow::Result<InferenceEvent>> {
        stream.collect::<Vec<_>>().await
    }

    #[tokio::test]
    async fn streams_multi_frame_happy_path_incrementally() {
        let body = happy_path_sse_body();
        // Hold back everything after the first text delta until the test has
        // observed that delta, proving events flow before the response ends.
        let gate_at = body.find(r#""Hel"}}"#).unwrap() + r#""Hel"}}"#.len() + 2;
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();
        let url = spawn_gated_sse_server(
            body.as_bytes()[..gate_at].to_vec(),
            body.as_bytes()[gate_at..].to_vec(),
            gate_rx,
        )
        .await;

        let mut stream = start_anthropic_stream(
            turn_request(&url, ReliabilityRequestPolicy::default()),
            vec![shell_tool()],
        )
        .await
        .unwrap();

        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(
            first,
            InferenceEvent::MessageDelta(MessageDelta {
                text: "Hel".to_string(),
                phase: None,
            })
        );
        gate_tx.send(()).unwrap();

        let mut events = vec![first];
        while let Some(event) = stream.next().await {
            events.push(event.unwrap());
        }
        assert_eq!(events, happy_path_expected_events());
        let text_deltas = events
            .iter()
            .filter(|event| matches!(event, InferenceEvent::MessageDelta(_)))
            .count();
        assert!(
            text_deltas >= 3,
            "expected >=3 text deltas, got {text_deltas}"
        );
    }

    #[tokio::test]
    async fn reassembles_frames_and_multibyte_utf8_split_across_tcp_writes() {
        let body = [
            r#"data: {"type":"message_start","message":{"id":"msg_2","content":[],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"héllo "}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"🦀 wörld"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":4}}"#,
            r#"data: {"type":"message_stop"}"#,
        ]
        .map(|frame| format!("{frame}\r\n\r\n"))
        .join("");
        // Split mid-frame AND mid-crab (4-byte scalar) across TCP writes.
        let split = body.find("🦀").unwrap() + 2;
        let url = spawn_sse_server(vec![
            body.as_bytes()[..split].to_vec(),
            body.as_bytes()[split..].to_vec(),
        ])
        .await;

        let stream = start_anthropic_stream(
            turn_request(&url, ReliabilityRequestPolicy::default()),
            Vec::new(),
        )
        .await
        .unwrap();
        let events = collect_events(stream).await;

        let text = events
            .iter()
            .filter_map(|event| match event {
                Ok(InferenceEvent::MessageDelta(delta)) => Some(delta.text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(text, "héllo 🦀 wörld");
        assert!(matches!(
            events.last(),
            Some(Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some(reason),
                ..
            }))) if reason == "end_turn"
        ));
    }

    #[tokio::test]
    async fn mid_stream_error_frame_yields_terminal_failed_event() {
        let body = [
            r#"data: {"type":"message_start","message":{"id":"msg_3","content":[],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"partial"}}"#,
            r#"event: error
data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ]
        .map(|frame| format!("{frame}\n\n"))
        .join("");
        let url = spawn_sse_server(vec![body.into_bytes()]).await;

        let stream = start_anthropic_stream(
            turn_request(&url, ReliabilityRequestPolicy::default()),
            Vec::new(),
        )
        .await
        .unwrap();
        let events = collect_events(stream).await;

        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].as_ref().unwrap(),
            &InferenceEvent::MessageDelta(MessageDelta {
                text: "partial".to_string(),
                phase: None,
            })
        );
        // After content has streamed a retry would duplicate it, so the error
        // frame surfaces as a terminal Failed event instead.
        assert_eq!(
            events[1].as_ref().unwrap(),
            &InferenceEvent::Failed(InferenceFailure {
                message: "Anthropic stream error (overloaded_error): Overloaded".to_string(),
            })
        );
    }

    #[tokio::test]
    async fn error_frame_before_content_retries_then_streams() {
        let error_body = "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
        let url =
            spawn_sse_server_with_bodies(vec![error_body.to_string(), happy_path_sse_body()]).await;
        let policy = fast_policy(2);

        let stream = start_anthropic_stream(turn_request(&url, policy.clone()), vec![shell_tool()])
            .await
            .unwrap();
        let events = collect_events(stream)
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();

        let mut expected = vec![InferenceEvent::ProviderMetadata(provider_retry_metadata(
            1,
            "stream_error",
            &policy,
        ))];
        expected.extend(happy_path_expected_events());
        assert_eq!(events, expected);
    }

    #[tokio::test]
    async fn error_frame_before_content_with_exhausted_budget_yields_failed() {
        let error_body = "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
        let url = spawn_sse_server_with_bodies(vec![error_body.to_string()]).await;

        let stream = start_anthropic_stream(turn_request(&url, fast_policy(1)), Vec::new())
            .await
            .unwrap();
        let events = collect_events(stream).await;

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].as_ref().unwrap(),
            &InferenceEvent::Failed(InferenceFailure {
                message: "Anthropic stream error (overloaded_error): Overloaded".to_string(),
            })
        );
    }

    #[tokio::test]
    async fn empty_body_retries_then_streams() {
        let url = spawn_sse_server_with_bodies(vec![String::new(), happy_path_sse_body()]).await;
        let policy = fast_policy(2);

        let stream = start_anthropic_stream(turn_request(&url, policy.clone()), vec![shell_tool()])
            .await
            .unwrap();
        let events = collect_events(stream)
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();

        let mut expected = vec![InferenceEvent::ProviderMetadata(provider_retry_metadata(
            1,
            "empty_provider_body",
            &policy,
        ))];
        expected.extend(happy_path_expected_events());
        assert_eq!(events, expected);
    }

    #[tokio::test]
    async fn empty_body_does_not_retry_when_policy_disables_it() {
        let url = spawn_sse_server_with_bodies(vec![String::new(), happy_path_sse_body()]).await;
        let policy = ReliabilityRequestPolicy {
            retry_empty_provider_body: false,
            ..fast_policy(3)
        };

        let stream = start_anthropic_stream(turn_request(&url, policy), Vec::new())
            .await
            .unwrap();
        let events = collect_events(stream).await;

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].as_ref().unwrap_err().to_string(),
            "Anthropic stream closed before message_stop"
        );
    }

    #[tokio::test]
    async fn truncation_before_first_event_retries_then_streams() {
        // message_start parses but emits nothing; the stream then dies before
        // any user-visible event, so a retry is duplicate-safe.
        let truncated = "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_5\",\"content\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n";
        let url =
            spawn_sse_server_with_bodies(vec![truncated.to_string(), happy_path_sse_body()]).await;
        let policy = fast_policy(2);

        let stream = start_anthropic_stream(turn_request(&url, policy.clone()), vec![shell_tool()])
            .await
            .unwrap();
        let events = collect_events(stream)
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();

        let mut expected = vec![InferenceEvent::ProviderMetadata(provider_retry_metadata(
            1,
            "stream_closed_before_first_event",
            &policy,
        ))];
        expected.extend(happy_path_expected_events());
        assert_eq!(events, expected);
    }

    #[tokio::test]
    async fn truncation_after_content_surfaces_retryable_stream_error() {
        let body = [
            r#"data: {"type":"message_start","message":{"id":"msg_4","content":[],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"cut"}}"#,
        ]
        .map(|frame| format!("{frame}\n\n"))
        .join("");
        let url = spawn_sse_server(vec![body.into_bytes()]).await;

        let stream = start_anthropic_stream(
            turn_request(&url, ReliabilityRequestPolicy::default()),
            Vec::new(),
        )
        .await
        .unwrap();
        let events = collect_events(stream).await;

        let error = events.last().unwrap().as_ref().unwrap_err().to_string();
        // Phrase recognized by roder-core's provider_stream_retry_cause.
        assert_eq!(error, "Anthropic stream closed before message_stop");
    }

    #[tokio::test]
    async fn completed_turn_ends_without_waiting_for_connection_close() {
        let url = spawn_lingering_sse_server(happy_path_sse_body()).await;

        let stream = start_anthropic_stream(
            turn_request(&url, ReliabilityRequestPolicy::default()),
            vec![shell_tool()],
        )
        .await
        .unwrap();
        let events = tokio::time::timeout(Duration::from_secs(5), collect_events(stream))
            .await
            .expect("stream should complete at message_stop without server close");

        assert!(matches!(
            events.last(),
            Some(Ok(InferenceEvent::Completed(_)))
        ));
    }

    #[tokio::test]
    async fn stalled_stream_errors_after_read_timeout() {
        let truncated = "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_6\",\"content\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n";
        let url = spawn_stalling_sse_server(truncated.to_string()).await;
        let turn = AnthropicTurnRequest {
            client: reqwest::Client::builder()
                .read_timeout(Duration::from_millis(200))
                .build()
                .unwrap(),
            url,
            api_key: "secret".to_string(),
            betas: Vec::new(),
            body: json!({}),
            policy: fast_policy(1),
        };

        let stream = start_anthropic_stream(turn, Vec::new()).await.unwrap();
        let events = tokio::time::timeout(Duration::from_secs(5), collect_events(stream))
            .await
            .expect("read timeout should end the stalled stream");

        assert!(events.last().unwrap().is_err());
    }

    #[tokio::test]
    async fn retry_recovers_after_retryable_status_then_streams() {
        let url = spawn_sse_retry_server(429, r#"{"error":"busy"}"#, happy_path_sse_body()).await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_status_codes: vec![429],
            ..fast_policy(2)
        };

        let stream = start_anthropic_stream(turn_request(&url, policy.clone()), vec![shell_tool()])
            .await
            .unwrap();
        let events = collect_events(stream)
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();

        let retry_metadata = provider_retry_metadata(1, "status_429", &policy);
        assert_eq!(retry_metadata["kind"], "reliability_retry_attempt");
        assert_eq!(retry_metadata["cause"], "status_429");
        let mut expected = vec![InferenceEvent::ProviderMetadata(retry_metadata)];
        expected.extend(happy_path_expected_events());
        assert_eq!(events, expected);
    }

    #[tokio::test]
    async fn retry_non_retryable_status_fails_once() {
        let (url, request_count) = spawn_counting_retry_server(vec![
            (400, r#"{"error":"bad request"}"#),
            (
                200,
                r#"{"id":"msg_1","content":[{"type":"text","text":"should-not-run"}]}"#,
            ),
        ])
        .await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_status_codes: vec![429],
            ..fast_policy(3)
        };

        let err = match start_anthropic_stream(turn_request(&url, policy), Vec::new()).await {
            Ok(_) => panic!("expected the non-retryable status to fail the request"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("Anthropic error 400"));
        assert_eq!(request_count.load(Ordering::SeqCst), 1);
    }

    const SSE_RESPONSE_HEAD: &[u8] =
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";

    /// Serves one SSE response, writing the body in the given raw byte chunks
    /// (which may split frames and multi-byte characters) with a flush between
    /// each.
    async fn spawn_sse_server(chunks: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0_u8; 16384];
            let _ = stream.read(&mut buf).await.unwrap();
            stream.write_all(SSE_RESPONSE_HEAD).await.unwrap();
            for chunk in chunks {
                stream.write_all(&chunk).await.unwrap();
                stream.flush().await.unwrap();
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        format!("http://{addr}/v1/messages")
    }

    /// Serves one SSE response per body, each on its own connection.
    async fn spawn_sse_server_with_bodies(bodies: Vec<String>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for body in bodies {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = [0_u8; 16384];
                let _ = stream.read(&mut buf).await.unwrap();
                stream.write_all(SSE_RESPONSE_HEAD).await.unwrap();
                stream.write_all(body.as_bytes()).await.unwrap();
            }
        });
        format!("http://{addr}/v1/messages")
    }

    /// Serves one SSE response in two parts, holding the second part until the
    /// gate fires.
    async fn spawn_gated_sse_server(
        first: Vec<u8>,
        rest: Vec<u8>,
        gate: tokio::sync::oneshot::Receiver<()>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0_u8; 16384];
            let _ = stream.read(&mut buf).await.unwrap();
            stream.write_all(SSE_RESPONSE_HEAD).await.unwrap();
            stream.write_all(&first).await.unwrap();
            stream.flush().await.unwrap();
            gate.await.unwrap();
            stream.write_all(&rest).await.unwrap();
        });
        format!("http://{addr}/v1/messages")
    }

    /// Serves a full SSE body, then holds the connection open without closing.
    async fn spawn_lingering_sse_server(body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0_u8; 16384];
            let _ = stream.read(&mut buf).await.unwrap();
            stream.write_all(SSE_RESPONSE_HEAD).await.unwrap();
            stream.write_all(body.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        format!("http://{addr}/v1/messages")
    }

    /// Writes a partial SSE body, then stalls without closing the connection.
    async fn spawn_stalling_sse_server(partial_body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0_u8; 16384];
            let _ = stream.read(&mut buf).await.unwrap();
            stream.write_all(SSE_RESPONSE_HEAD).await.unwrap();
            stream.write_all(partial_body.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        format!("http://{addr}/v1/messages")
    }

    /// Responds to the first request with an HTTP error status and to the
    /// second with an SSE stream.
    async fn spawn_sse_retry_server(
        status: u16,
        error_body: &'static str,
        sse_body: String,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0_u8; 16384];
            let _ = stream.read(&mut buf).await.unwrap();
            let response = format!(
                "HTTP/1.1 {status} Error\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{error_body}",
                error_body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            drop(stream);

            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = stream.read(&mut buf).await.unwrap();
            stream.write_all(SSE_RESPONSE_HEAD).await.unwrap();
            stream.write_all(sse_body.as_bytes()).await.unwrap();
        });
        format!("http://{addr}/v1/messages")
    }

    async fn spawn_counting_retry_server(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let request_count = Arc::new(AtomicUsize::new(0));
        let count = request_count.clone();
        tokio::spawn(async move {
            for (status, body) in responses {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                count.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf).await.unwrap();
                let reason = if status == 200 { "OK" } else { "Error" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (format!("http://{addr}/v1/messages"), request_count)
    }
}
