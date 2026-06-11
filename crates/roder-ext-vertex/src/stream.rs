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
use serde_json::Value;

use crate::sse::{MAX_FRAME_BYTES, SseFrameBuffer, VertexStreamState};

const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const STREAM_IDLE_TIMEOUT_ENV: &str = "RODER_VERTEX_STREAM_IDLE_TIMEOUT_MS";

/**
 * The read timeout turns a stalled-but-TCP-alive connection into a stream
 * error so the host can recover. Vertex sends no ping frames, so the default
 * is generous enough to survive long thinking pauses between chunks.
 */
pub(crate) fn vertex_stream_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .read_timeout(vertex_stream_idle_timeout())
        .build()?)
}

fn vertex_stream_idle_timeout() -> Duration {
    std::env::var(STREAM_IDLE_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_STREAM_IDLE_TIMEOUT)
}

pub(crate) struct VertexTurnRequest {
    pub(crate) client: reqwest::Client,
    pub(crate) url: String,
    pub(crate) access_token: String,
    pub(crate) body: Value,
    pub(crate) policy: ReliabilityRequestPolicy,
}

impl VertexTurnRequest {
    fn max_attempts(&self) -> u32 {
        self.policy.provider_retry_max_attempts.max(1)
    }

    /**
     * Sends the request, retrying retryable HTTP statuses and transport
     * errors. `attempt` counts attempts across the whole turn so stream-level
     * retries in `stream_vertex_turn` share the same budget.
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
            let response = self
                .client
                .post(&self.url)
                .bearer_auth(&self.access_token)
                .json(&self.body)
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    last_error = Some(anyhow::anyhow!("Vertex AI error {status}: {text}"));
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
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Vertex AI request failed")))
    }
}

/// Sends the first request and returns the retrying event stream for the turn.
pub(crate) async fn start_vertex_stream(
    turn: VertexTurnRequest,
) -> anyhow::Result<InferenceEventStream> {
    let mut attempt = 0;
    let mut retry_events = Vec::new();
    let response = turn.send(&mut attempt, &mut retry_events).await?;
    Ok(stream_vertex_turn(turn, response, attempt, retry_events))
}

/**
 * Drives the SSE stream with provider-side retries that are only taken while
 * nothing has been emitted yet: an in-stream error payload, an empty body, or
 * a stream that dies before its first event can be replayed without
 * duplicating transcript content. Past the first event, failures surface
 * mid-stream — error payloads as a terminal `InferenceEvent::Failed`,
 * truncation and transport failures as stream errors.
 */
fn stream_vertex_turn(
    turn: VertexTurnRequest,
    first_response: reqwest::Response,
    attempts_used: u32,
    retry_events: Vec<Value>,
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
            let mut driver = VertexSseDriver::new(current);
            // The first event decides whether retrying is still duplicate-safe.
            match driver.next_event().await {
                Some(Ok(InferenceEvent::Failed(failure))) => {
                    // In-stream error payload on an HTTP 200 connection
                    // before any content.
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
async fn early_retry(turn: &VertexTurnRequest, attempt: &mut u32, cause: &str) -> Option<Value> {
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
 * whether a failed attempt is still duplicate-safe to retry. Vertex has no
 * terminal SSE frame, so the turn finalizes at EOF: with a finishReason seen
 * the accumulated usage/metadata/completion events are emitted, without one
 * the stream was truncated.
 */
struct VertexSseDriver {
    chunks: BoxStream<'static, reqwest::Result<Bytes>>,
    buffer: SseFrameBuffer,
    state: Option<VertexStreamState>,
    pending: VecDeque<InferenceEvent>,
    pending_error: Option<anyhow::Error>,
    received_frame: bool,
    source_done: bool,
    finished: bool,
}

impl VertexSseDriver {
    fn new(response: reqwest::Response) -> Self {
        Self {
            chunks: response.bytes_stream().boxed(),
            buffer: SseFrameBuffer::default(),
            state: Some(VertexStreamState::default()),
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
            if self.source_done {
                if let Some(frame) = self.buffer.take_trailing() {
                    self.push_frame(&frame, true);
                    continue;
                }
                self.finished = true;
                let state = self.state.take()?;
                if !self.received_frame || !state.saw_finish_reason() {
                    return Some(Err(anyhow::anyhow!(
                        "Vertex AI stream closed before finishReason"
                    )));
                }
                self.pending.extend(state.finish());
                continue;
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
                        if !self.push_frame(&frame, false) {
                            break;
                        }
                    }
                    if self.pending_error.is_none() && !self.buffer.within_frame_cap() {
                        self.pending_error = Some(anyhow::anyhow!(
                            "Vertex AI SSE frame exceeded {MAX_FRAME_BYTES} bytes without a frame delimiter"
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
        let Some(state) = self.state.as_mut() else {
            return false;
        };
        match state.push_frame(frame) {
            Ok(events) => {
                let failed = events
                    .iter()
                    .any(|event| matches!(event, InferenceEvent::Failed(_)));
                self.pending.extend(events);
                if failed {
                    // An error payload ends the turn; skip EOF finalization.
                    self.finished = true;
                    self.state = None;
                }
                !failed
            }
            Err(err) => {
                self.pending_error = Some(if trailing {
                    err.context("Vertex AI stream closed before finishReason")
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
        CompletionMetadata, InferenceFailure, MessageDelta, ReasoningDelta, TokenUsage,
        ToolCallCompleted,
    };
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn turn_request(url: &str, policy: ReliabilityRequestPolicy) -> VertexTurnRequest {
        VertexTurnRequest {
            client: reqwest::Client::new(),
            url: url.to_string(),
            access_token: "test-token".to_string(),
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

    fn happy_path_sse_body() -> String {
        [
            r#"data: {"responseId":"resp_1","modelVersion":"gemini-3.5-flash","candidates":[{"content":{"role":"model","parts":[{"text":"deciding what to run","thought":true}]}}]}"#,
            r#"data: {"candidates":[{"content":{"parts":[{"text":"Hel"}]}}]}"#,
            r#"data: {"candidates":[{"content":{"parts":[{"text":"lo "}]}}]}"#,
            r#"data: {"candidates":[{"content":{"parts":[{"text":"world"}]}}]}"#,
            r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"id":"call_1","name":"shell","args":{"cmd":"ls"}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":11,"candidatesTokenCount":7,"totalTokenCount":18,"cachedContentTokenCount":8}}"#,
        ]
        .map(|frame| format!("{frame}\n\n"))
        .join("")
    }

    fn happy_path_expected_events() -> Vec<InferenceEvent> {
        vec![
            InferenceEvent::ReasoningDelta(ReasoningDelta {
                text: "deciding what to run".to_string(),
            }),
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
            InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "call_1".to_string(),
                name: "shell".to_string(),
                arguments: r#"{"cmd":"ls"}"#.to_string(),
            }),
            InferenceEvent::Usage(TokenUsage::new(11, 7, 18).with_cached_prompt_tokens(8)),
            InferenceEvent::ProviderMetadata(json!({
                "candidates": [{
                    "content": { "role": "model", "parts": [
                        { "text": "deciding what to run", "thought": true },
                        { "text": "Hel" },
                        { "text": "lo " },
                        { "text": "world" },
                        { "functionCall": { "id": "call_1", "name": "shell", "args": { "cmd": "ls" } } }
                    ] },
                    "finishReason": "STOP"
                }],
                "responseId": "resp_1",
                "modelVersion": "gemini-3.5-flash",
                "usageMetadata": {
                    "promptTokenCount": 11,
                    "candidatesTokenCount": 7,
                    "totalTokenCount": 18,
                    "cachedContentTokenCount": 8
                }
            })),
            InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("tool_use".to_string()),
                provider_response_id: Some("resp_1".to_string()),
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
        let gate_at = body.find(r#""Hel"}"#).unwrap() + r#""Hel"}"#.len();
        let gate_at = body[gate_at..].find("\n\n").unwrap() + gate_at + 2;
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();
        let url = spawn_gated_sse_server(
            body.as_bytes()[..gate_at].to_vec(),
            body.as_bytes()[gate_at..].to_vec(),
            gate_rx,
        )
        .await;

        let mut stream =
            start_vertex_stream(turn_request(&url, ReliabilityRequestPolicy::default()))
                .await
                .unwrap();

        let mut events = Vec::new();
        loop {
            let event = stream.next().await.unwrap().unwrap();
            let is_first_text = matches!(
                &event,
                InferenceEvent::MessageDelta(delta) if delta.text == "Hel"
            );
            events.push(event);
            if is_first_text {
                break;
            }
        }
        gate_tx.send(()).unwrap();

        while let Some(event) = stream.next().await {
            events.push(event.unwrap());
        }
        assert_eq!(events, happy_path_expected_events());
    }

    #[tokio::test]
    async fn reassembles_frames_and_multibyte_utf8_split_across_tcp_writes() {
        let body = [
            r#"data: {"candidates":[{"content":{"parts":[{"text":"héllo "}]}}]}"#,
            r#"data: {"candidates":[{"content":{"parts":[{"text":"🦀 wörld"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":4,"totalTokenCount":5}}"#,
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

        let stream = start_vertex_stream(turn_request(&url, ReliabilityRequestPolicy::default()))
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
            }))) if reason == "stop"
        ));
    }

    #[tokio::test]
    async fn mid_stream_error_payload_yields_terminal_failed_event() {
        let body = [
            r#"data: {"candidates":[{"content":{"parts":[{"text":"partial"}]}}]}"#,
            r#"data: {"error":{"code":529,"status":"UNAVAILABLE","message":"Overloaded"}}"#,
        ]
        .map(|frame| format!("{frame}\n\n"))
        .join("");
        let url = spawn_sse_server(vec![body.into_bytes()]).await;

        let stream = start_vertex_stream(turn_request(&url, ReliabilityRequestPolicy::default()))
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
        // payload surfaces as a terminal Failed event instead.
        assert_eq!(
            events[1].as_ref().unwrap(),
            &InferenceEvent::Failed(InferenceFailure {
                message: "Vertex AI stream error (UNAVAILABLE): Overloaded".to_string(),
            })
        );
    }

    #[tokio::test]
    async fn error_payload_before_content_retries_then_streams() {
        let error_body =
            "data: {\"error\":{\"status\":\"UNAVAILABLE\",\"message\":\"Overloaded\"}}\n\n";
        let url =
            spawn_sse_server_with_bodies(vec![error_body.to_string(), happy_path_sse_body()]).await;
        let policy = fast_policy(2);

        let stream = start_vertex_stream(turn_request(&url, policy.clone()))
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
    async fn error_payload_before_content_with_exhausted_budget_yields_failed() {
        let error_body =
            "data: {\"error\":{\"status\":\"UNAVAILABLE\",\"message\":\"Overloaded\"}}\n\n";
        let url = spawn_sse_server_with_bodies(vec![error_body.to_string()]).await;

        let stream = start_vertex_stream(turn_request(&url, fast_policy(1)))
            .await
            .unwrap();
        let events = collect_events(stream).await;

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].as_ref().unwrap(),
            &InferenceEvent::Failed(InferenceFailure {
                message: "Vertex AI stream error (UNAVAILABLE): Overloaded".to_string(),
            })
        );
    }

    #[tokio::test]
    async fn empty_body_retries_then_streams() {
        let url = spawn_sse_server_with_bodies(vec![String::new(), happy_path_sse_body()]).await;
        let policy = fast_policy(2);

        let stream = start_vertex_stream(turn_request(&url, policy.clone()))
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
    async fn truncation_before_first_event_retries_then_streams() {
        // A frame that parses but emits nothing (no parts), then EOF without
        // finishReason: nothing user-visible was emitted, so a retry is
        // duplicate-safe.
        let truncated = "data: {\"candidates\":[{\"content\":{\"parts\":[]}}]}\n\n";
        let url =
            spawn_sse_server_with_bodies(vec![truncated.to_string(), happy_path_sse_body()]).await;
        let policy = fast_policy(2);

        let stream = start_vertex_stream(turn_request(&url, policy.clone()))
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
    async fn truncation_after_content_surfaces_stream_error() {
        let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"cut\"}]}}]}\n\n";
        let url = spawn_sse_server(vec![body.as_bytes().to_vec()]).await;

        let stream = start_vertex_stream(turn_request(&url, ReliabilityRequestPolicy::default()))
            .await
            .unwrap();
        let events = collect_events(stream).await;

        let error = events.last().unwrap().as_ref().unwrap_err().to_string();
        assert_eq!(error, "Vertex AI stream closed before finishReason");
    }

    #[tokio::test]
    async fn retry_recovers_after_retryable_status_then_streams() {
        let url = spawn_sse_retry_server(429, r#"{"error":"busy"}"#, happy_path_sse_body()).await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_status_codes: vec![429],
            ..fast_policy(2)
        };

        let stream = start_vertex_stream(turn_request(&url, policy.clone()))
            .await
            .unwrap();
        let events = collect_events(stream)
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();

        let mut expected = vec![InferenceEvent::ProviderMetadata(provider_retry_metadata(
            1,
            "status_429",
            &policy,
        ))];
        expected.extend(happy_path_expected_events());
        assert_eq!(events, expected);
    }

    #[tokio::test]
    async fn retry_non_retryable_status_fails_once_without_echoing_token() {
        let (url, request_count, requests) = spawn_counting_retry_server(vec![
            (400, r#"{"error":"bad request"}"#),
            (200, r#"{"should":"not run"}"#),
        ])
        .await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_status_codes: vec![429],
            ..fast_policy(3)
        };

        let err = match start_vertex_stream(turn_request(&url, policy)).await {
            Ok(_) => panic!("expected the non-retryable status to fail the request"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("Vertex AI error 400"));
        assert_eq!(request_count.load(Ordering::SeqCst), 1);
        let first_request = requests.lock().unwrap().first().cloned().unwrap();
        assert!(
            first_request.contains("authorization: Bearer test-token")
                || first_request.contains("Authorization: Bearer test-token"),
            "{first_request}"
        );
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
        format!(
            "http://{addr}/v1/projects/p/locations/l/publishers/google/models/m:streamGenerateContent?alt=sse"
        )
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
        format!(
            "http://{addr}/v1/projects/p/locations/l/publishers/google/models/m:streamGenerateContent?alt=sse"
        )
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
        format!(
            "http://{addr}/v1/projects/p/locations/l/publishers/google/models/m:streamGenerateContent?alt=sse"
        )
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
        format!(
            "http://{addr}/v1/projects/p/locations/l/publishers/google/models/m:streamGenerateContent?alt=sse"
        )
    }

    async fn spawn_counting_retry_server(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, Arc<AtomicUsize>, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let request_count = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let count = request_count.clone();
        let captured = requests.clone();
        tokio::spawn(async move {
            for (status, body) in responses {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                count.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0_u8; 16384];
                let n = stream.read(&mut buf).await.unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let reason = if status == 200 { "OK" } else { "Error" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (
            format!(
                "http://{addr}/v1/projects/p/locations/l/publishers/google/models/m:streamGenerateContent?alt=sse"
            ),
            request_count,
            requests,
        )
    }
}
