use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, StreamExt};
use tokio::time::Instant;
use uuid::Uuid;

use crate::proto::{
    ConnectFrame, CursorHistoryMessage, CursorToolCall, decode_agent_server_message,
    encode_agent_client_message_with_history, encode_cli_stream_control_frames,
    encode_connect_frame, take_connect_frame,
};

pub const DEFAULT_AGENT_SERVICE_URL: &str = "https://agentn.global.api5.cursor.sh";
pub const DEFAULT_AGENT_SERVICE_PATH: &str = "/agent.v1.AgentService/Run";
pub const DEFAULT_CLIENT_VERSION: &str = "cli-2026.05.24-dda726e";

#[derive(Debug, Clone)]
pub struct AgentServiceConfig {
    pub endpoint: String,
    pub path: String,
    pub client_version: String,
    pub timeout: Duration,
    pub idle_timeout: Duration,
}

impl Default for AgentServiceConfig {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_AGENT_SERVICE_URL.to_string(),
            path: DEFAULT_AGENT_SERVICE_PATH.to_string(),
            client_version: DEFAULT_CLIENT_VERSION.to_string(),
            timeout: Duration::from_secs(60),
            idle_timeout: Duration::from_millis(2500),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentServiceRequest {
    pub access_token: String,
    pub prompt: String,
    pub model: String,
    pub context_frames: Vec<Vec<u8>>,
    pub history: Vec<CursorHistoryMessage>,
}

#[derive(Debug, Clone)]
pub enum AgentServiceEvent {
    Text(String),
    Thinking(String),
    ToolCalls(Vec<CursorToolCall>),
    UsageFields(std::collections::BTreeMap<u32, u64>),
    Completed(String),
}

pub struct AgentServiceStream {
    pub request_id: String,
    pub conversation_id: String,
    pub events: std::pin::Pin<Box<dyn Stream<Item = anyhow::Result<AgentServiceEvent>> + Send>>,
}

pub async fn stream_agent_service(
    config: AgentServiceConfig,
    request: AgentServiceRequest,
) -> anyhow::Result<AgentServiceStream> {
    let request_id = Uuid::new_v4().to_string();
    let conversation_id = Uuid::new_v4().to_string();
    let message_id = Uuid::new_v4().to_string();
    let mut frames = vec![encode_connect_frame(&encode_agent_client_message_with_history(
        &request.prompt,
        &request.model,
        &conversation_id,
        &message_id,
        &request.history,
    ))];
    for frame in &request.context_frames {
        frames.push(frame.clone());
    }
    for frame in encode_cli_stream_control_frames() {
        frames.push(frame);
    }
    for (index, frame) in frames.iter().enumerate() {
        capture_cursor_frame("send", index, frame);
    }
    let request_body = reqwest::Body::wrap_stream(async_stream::stream! {
        for frame in frames {
            yield Ok::<Bytes, std::io::Error>(Bytes::from(frame));
        }
        std::future::pending::<()>().await;
    });

    let client = reqwest::Client::builder()
        .http2_adaptive_window(true)
        .timeout(config.timeout + config.idle_timeout)
        .build()?;
    let traceparent = traceparent();
    let response = client
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
        .header("x-original-request-id", &request_id)
        .header("x-request-id", &request_id)
        .body(request_body)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Cursor AgentService returned HTTP {status}: {body}");
    }

    let timeout = config.timeout;
    let idle_duration = config.idle_timeout;
    let mut stream = response.bytes_stream();
    let events = Box::pin(async_stream::stream! {
        let mut buffer = Vec::new();
        let hard_timeout = tokio::time::sleep(timeout);
        let idle_timeout = tokio::time::sleep(timeout);
        tokio::pin!(hard_timeout);
        tokio::pin!(idle_timeout);
        let mut idle_armed = false;

        let mut done = false;
        while !done {
            tokio::select! {
                _ = &mut hard_timeout => {
                    yield Err(anyhow::anyhow!("timed out waiting for Cursor AgentService response"));
                    done = true;
                }
                _ = &mut idle_timeout, if idle_armed => {
                    yield Ok(AgentServiceEvent::Completed("idle_timeout".to_string()));
                    done = true;
                }
                chunk = stream.next() => {
                    let Some(chunk) = chunk else {
                        yield Ok(AgentServiceEvent::Completed("end_stream".to_string()));
                        done = true;
                        continue;
                    };
                    match chunk {
                        Ok(chunk) => buffer.extend_from_slice(&chunk),
                        Err(err) => {
                            yield Err(err.into());
                            done = true;
                            continue;
                        }
                    }
                    let mut completed = None;
                    loop {
                        let frame = match take_connect_frame(&mut buffer) {
                            Ok(Some(frame)) => frame,
                            Ok(None) => break,
                            Err(err) => {
                                yield Err(err);
                                done = true;
                                break;
                            }
                        };
                        match frame {
                            ConnectFrame::Payload(payload) => {
                                capture_cursor_frame("recv", 0, &payload);
                                let decoded = decode_agent_server_message(&payload);
                                if !decoded.text.is_empty() {
                                    idle_armed = true;
                                    idle_timeout.as_mut().reset(Instant::now() + idle_duration);
                                    yield Ok(AgentServiceEvent::Text(decoded.text));
                                }
                                if !decoded.thinking.is_empty() {
                                    yield Ok(AgentServiceEvent::Thinking(decoded.thinking));
                                }
                                if !decoded.usage_fields.is_empty() {
                                    yield Ok(AgentServiceEvent::UsageFields(decoded.usage_fields));
                                }
                                if !decoded.tool_calls.is_empty() {
                                    yield Ok(AgentServiceEvent::ToolCalls(decoded.tool_calls));
                                    completed = Some("tool_calls".to_string());
                                    break;
                                }
                                if decoded.turn_ended {
                                    completed = Some("turn_ended".to_string());
                                    break;
                                }
                            }
                            ConnectFrame::EndStream(error) => {
                                if let Some(error) = error {
                                    yield Err(anyhow::anyhow!("Cursor AgentService end-stream error: {error}"));
                                    done = true;
                                    break;
                                }
                                completed = Some("end_stream".to_string());
                                break;
                            }
                        }
                    }
                    if let Some(reason) = completed {
                        yield Ok(AgentServiceEvent::Completed(reason));
                        done = true;
                    }
                }
            }
        }
    });

    Ok(AgentServiceStream {
        request_id,
        conversation_id,
        events,
    })
}

fn traceparent() -> String {
    let trace_id = Uuid::new_v4().simple().to_string();
    let span_id = &Uuid::new_v4().simple().to_string()[..16];
    format!("00-{trace_id}-{span_id}-01")
}

/// Env var pointing at a JSONL file. When set, every raw Cursor AgentService
/// wire frame (outbound request frames and inbound response payloads) is
/// appended as hex. This is a diagnostic used to reverse-engineer Cursor-native
/// tool-call frames (e.g. `edit`/`shell`) so they can be mapped into canonical
/// Roder tool execution. It has zero effect on normal runs (no var = no-op).
pub const CAPTURE_FRAMES_ENV: &str = "RODER_CURSOR_CAPTURE_FRAMES";

pub(crate) fn capture_cursor_frame(direction: &str, index: usize, bytes: &[u8]) {
    let path = match std::env::var(CAPTURE_FRAMES_ENV) {
        Ok(path) if !path.is_empty() => path,
        _ => return,
    };
    use std::io::Write as _;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = std::fmt::Write::write_fmt(&mut hex, format_args!("{byte:02x}"));
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let line = format!(
        "{{\"ts_ms\":{ts},\"dir\":\"{direction}\",\"index\":{index},\"len\":{},\"hex\":\"{hex}\"}}\n",
        bytes.len()
    );
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_agentservice_endpoint() {
        let config = AgentServiceConfig::default();
        assert_eq!(config.endpoint, DEFAULT_AGENT_SERVICE_URL);
        assert_eq!(config.path, DEFAULT_AGENT_SERVICE_PATH);
    }

    #[test]
    fn capture_frame_is_noop_without_env_and_writes_jsonl_when_set() {
        // No env var set: capture must do nothing (and must not panic).
        unsafe { std::env::remove_var(CAPTURE_FRAMES_ENV) };
        capture_cursor_frame("recv", 0, &[0xde, 0xad]);

        // With the env var set, frames are appended as hex JSONL lines.
        let mut path = std::env::temp_dir();
        path.push(format!("roder-cursor-capture-{}.jsonl", Uuid::new_v4()));
        unsafe { std::env::set_var(CAPTURE_FRAMES_ENV, &path) };
        capture_cursor_frame("send", 3, &[0x00, 0x01, 0xff]);
        capture_cursor_frame("recv", 0, &[0xab]);
        unsafe { std::env::remove_var(CAPTURE_FRAMES_ENV) };

        let contents = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"dir\":\"send\""));
        assert!(lines[0].contains("\"index\":3"));
        assert!(lines[0].contains("\"len\":3"));
        assert!(lines[0].contains("\"hex\":\"0001ff\""));
        assert!(lines[1].contains("\"hex\":\"ab\""));
    }
}
