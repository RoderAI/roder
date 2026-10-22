//! Cursor `agent.v1.AgentService/Run` bidirectional agent-runtime client.
//!
//! Unlike the simple inference path (one request -> event stream), Cursor's
//! agent loop is *same-stream*: the server streams the model's output and emits
//! `exec_server_message` read/write/shell requests; the client executes them
//! locally and writes `exec_client_message` results back into the *same* open
//! request body, and the server continues. This client keeps the Run stream
//! open (channel-fed request body), services the exec channel, and surfaces the
//! model's text as Roder `MessageDelta`s. See
//! `docs/roder-cursor-agent-runtime-protocol.md`.

use std::path::PathBuf;
#[path = "bidi_exec.rs"]
mod exec;
use exec::service_exec;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use roder_api::inference::{
    CompletionMetadata, InferenceEvent, InferenceEventStream, MessageDelta, ReasoningDelta,
    TokenUsage, TurnToolExecutor,
};
use serde_json::json;

use crate::agentservice::AgentServiceConfig;
use crate::proto::{
    ConnectFrame, decode_server_frame, encode_cli_stream_control_frames, encode_connect_frame,
    encode_exec_control, encode_heartbeat, encode_kv_ack, take_connect_frame,
};

pub struct BidiRequest {
    pub access_token: String,
    /// Pre-encoded `AgentClientMessage{run_request}` payload (not connect-framed).
    pub run_request: Vec<u8>,
    /// Already connect-framed request-context frames (workspace context).
    pub context_frames: Vec<Vec<u8>>,
    pub workspace: PathBuf,
    pub tool_executor: Option<Arc<dyn TurnToolExecutor>>,
    pub tools: Vec<crate::proto::CursorMcpTool>,
    pub state: crate::state::CursorConversationState,
    pub usage_metadata: BidiUsageMetadata,
}

#[derive(Debug, Clone, Default)]
pub struct BidiUsageMetadata {
    pub prompt_tokens: u32,
    pub provider: String,
    pub transport: String,
    pub auth_source: String,
    pub thread_id: String,
    pub turn_id: String,
    pub model: String,
    pub conversation_id: String,
}

pub async fn run_bidi_turn(
    config: AgentServiceConfig,
    request: BidiRequest,
) -> anyhow::Result<InferenceEventStream> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
    // Initial client frames: run request, request-context frames, then the CLI
    // stream-control frames the server expects to begin streaming.
    crate::agentservice::capture_cursor_frame("send", 0, &request.run_request);
    tx.send(Bytes::from(encode_connect_frame(&request.run_request)))
        .ok();
    for frame in &request.context_frames {
        tx.send(Bytes::from(frame.clone())).ok();
    }
    for frame in encode_cli_stream_control_frames() {
        tx.send(Bytes::from(frame)).ok();
    }

    let body_stream = async_stream::stream! {
        let mut rx = rx;
        while let Some(chunk) = rx.recv().await {
            yield Ok::<Bytes, std::io::Error>(chunk);
        }
    };
    // No overall request timeout: a bidi agent turn can run for minutes. Stalls
    // are bounded by the per-read idle timeout in the response loop below.
    let mut builder = reqwest::Client::builder().http2_adaptive_window(true);
    // Plaintext endpoints (local mock AgentService in tests / diagnostics)
    // cannot negotiate HTTP/2 via ALPN; force it so the same-stream bidi body
    // works. The production endpoint is always https and unaffected.
    if config.endpoint.starts_with("http://") {
        builder = builder.http2_prior_knowledge();
    }
    let client = builder.build()?;
    let traceparent = traceparent();
    let response = tokio::time::timeout(
        config.timeout,
        client
            .post(format!("{}{}", config.endpoint, config.path))
            .bearer_auth(&request.access_token)
            .header("backend-traceparent", &traceparent)
            .header("connect-accept-encoding", "identity")
            .header("connect-protocol-version", "1")
            .header("content-type", "application/connect+proto")
            .header("traceparent", traceparent)
            .header("user-agent", "connect-es/1.6.1")
            .header("x-cursor-client-type", "cli")
            .header("x-cursor-client-version", config.client_version)
            .header("x-ghost-mode", "true")
            .body(reqwest::Body::wrap_stream(body_stream))
            .send(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Cursor response headers timed out"))??;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Cursor AgentService returned HTTP {status}: {text}");
    }

    let workspace = request.workspace;
    let executor = request.tool_executor;
    let tools = request.tools;
    let mut state = request.state;
    let usage_metadata = request.usage_metadata;
    // The model can think for a while between exec steps; allow generous idle.
    let idle = Duration::from_secs(120);
    let mut bytes_stream = response.bytes_stream();

    let events = async_stream::try_stream! {
        let mut buffer: Vec<u8> = Vec::new();
        let mut done = false;
        // Keep `tx` alive for the whole turn so the request body stays open.
        let outbound = tx;
        let mut visible_tokens = 0u32;
        let mut thinking_tokens = 0u32;
        let mut usage_fields = serde_json::Map::new();

        // Periodic heartbeat so the server doesn't reset long turns (the model
        // can think for tens of seconds between/after tool calls).
        let _heartbeat = AbortOnDrop({
            let hb_tx = outbound.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(8));
                tick.tick().await; // consume immediate first tick
                loop {
                    tick.tick().await;
                    if hb_tx
                        .send(Bytes::from(encode_connect_frame(&encode_heartbeat())))
                        .is_err()
                    {
                        break;
                    }
                }
            })
        });

        // The server emits empty keepalive frames, so a stalled turn never goes
        // silent. Bound the turn on lack of *meaningful* progress (text, exec, or
        // a turn-end signal) instead of only on raw stream silence.
        let no_progress_limit = Duration::from_secs(90);
        let mut last_progress = tokio::time::Instant::now();

        while !done {
            if last_progress.elapsed() > no_progress_limit {
                Err(anyhow::anyhow!("Cursor turn stalled without meaningful progress"))?;
            }
            let remaining = no_progress_limit.saturating_sub(last_progress.elapsed());
            let chunk = tokio::time::timeout(idle.min(remaining), bytes_stream.next()).await;
            let chunk = match chunk {
                Ok(Some(Ok(c))) => c,
                Ok(Some(Err(err))) => { Err(anyhow::anyhow!(err))?; break; }
                Ok(None) => { Err(anyhow::anyhow!("Cursor stream closed before turn end"))?; break; }
                Err(_) => { Err(anyhow::anyhow!("Cursor stream timed out before turn end"))?; break; }
            };
            buffer.extend_from_slice(&chunk);

            loop {
                let frame = match take_connect_frame(&mut buffer) {
                    Ok(Some(f)) => f,
                    Ok(None) => break,
                    Err(err) => { Err(err)?; done = true; break; }
                };
                match frame {
                    ConnectFrame::EndStream(error) => {
                        if let Some(error) = error {
                            Err(anyhow::anyhow!("Cursor AgentService end-stream error: {error}"))?;
                        }
                        Err(anyhow::anyhow!("Cursor end-stream arrived before turn end"))?;
                    }
                    ConnectFrame::Payload(payload) => {
                        crate::agentservice::capture_cursor_frame("recv", 0, &payload);
                        let frame = decode_server_frame(&payload);
                        // Ack kv_server PUTs so the server persists conversation
                        // state and continues the multi-step turn.
                        if let Some((id,value))=frame.kv_put { state.put(id,value)?; }
                        if let Some((seq,id))=frame.kv_get {
                            let reply=state.get_reply(seq,&id);
                            crate::agentservice::capture_cursor_frame("send-kv",0,&reply);
                            let _=outbound.send(Bytes::from(encode_connect_frame(&reply)));
                        }
                        if let Some(checkpoint)=frame.checkpoint { state.checkpoint=checkpoint; }
                        if let Some(kv_seq) = frame.kv_seq {
                            let _ = outbound.send(Bytes::from(encode_connect_frame(&encode_kv_ack(kv_seq))));
                        }
                        for (field, value) in frame.usage_fields {
                            usage_fields.insert(format!("field_{field}"), json!(value));
                        }
                        if !frame.thinking.is_empty() {
                            last_progress = tokio::time::Instant::now();
                            thinking_tokens = thinking_tokens.saturating_add(estimate_text_tokens(&frame.thinking));
                            yield InferenceEvent::ReasoningDelta(ReasoningDelta { text: frame.thinking });
                        }
                        if !frame.text.is_empty() {
                            last_progress = tokio::time::Instant::now();
                            visible_tokens = visible_tokens.saturating_add(estimate_text_tokens(&frame.text));
                            yield InferenceEvent::MessageDelta(MessageDelta { text: frame.text, phase: None });
                        }
                        if let Some(exec) = frame.exec {
                            if let Some((result_frames, ack_seq)) =
                                service_exec(exec, &workspace, executor.as_deref(), &tools).await
                            {
                                for result_frame in &result_frames {
                                    crate::agentservice::capture_cursor_frame("send-result", 0, result_frame);
                                    let _ = outbound.send(Bytes::from(encode_connect_frame(result_frame)));
                                }
                                if let Some(seq) = ack_seq {
                                    let _ = outbound.send(Bytes::from(encode_connect_frame(&encode_exec_control(seq))));
                                }
                            }
                            // Servicing the exec (which can include a slow tool
                            // run or a user approval wait) is itself progress;
                            // without this reset a tool that takes longer than
                            // the no-progress cap ends the turn right after its
                            // result is sent, before the model can continue.
                            last_progress = tokio::time::Instant::now();
                        }
                        if frame.turn_ended {
                            done = true;
                            break;
                        }
                    }
                }
            }
        }

        let official_input = usage_fields.get("field_1").and_then(serde_json::Value::as_u64);
        let official_output = usage_fields.get("field_2").and_then(serde_json::Value::as_u64);
        let estimated = official_input.is_none() || official_output.is_none();
        let input_tokens = official_input.map(|v| v.min(u32::MAX as u64) as u32).unwrap_or(usage_metadata.prompt_tokens);
        let completion_tokens = official_output.map(|v| v.min(u32::MAX as u64) as u32).unwrap_or_else(|| visible_tokens.saturating_add(thinking_tokens));
        let reasoning_tokens = usage_fields.get("field_5").and_then(serde_json::Value::as_u64).unwrap_or(thinking_tokens as u64);
        let total_tokens = input_tokens.saturating_add(completion_tokens);
        yield InferenceEvent::Usage(TokenUsage::new(
            input_tokens,
            completion_tokens,
            total_tokens,
        ));
        yield InferenceEvent::ProviderMetadata(json!({
            "provider": usage_metadata.provider,
            "transport": usage_metadata.transport,
            "authSource": usage_metadata.auth_source,
            "threadId": usage_metadata.thread_id,
            "turnId": usage_metadata.turn_id,
            "model": usage_metadata.model,
            "conversationId": usage_metadata.conversation_id,
            "cursorState":state.metadata(),
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": completion_tokens,
                "total_tokens": total_tokens,
                "output_tokens_details": {
                    "reasoning_tokens": reasoning_tokens,
                    "visible_output_tokens": visible_tokens
                }
            },
            "usageFields": usage_fields,
            "usageFieldsSource": "cursor-agentservice-turn-end",
            "usageEstimated": estimated,
            "usageSource": if estimated { "chars_per_4_with_available_official_fields" } else { "cursor-agentservice-turn-end" },
        }));
        yield InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some("turn_ended".to_string()),
            provider_response_id: None,
        });
    };

    Ok(Box::pin(events))
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn estimate_text_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

fn traceparent() -> String {
    let trace_id = uuid::Uuid::new_v4().simple().to_string();
    let span_id = &uuid::Uuid::new_v4().simple().to_string()[..16];
    format!("00-{trace_id}-{span_id}-01")
}

#[cfg(test)]
#[path = "bidi_stream_tests.rs"]
mod bidi_stream_tests;
