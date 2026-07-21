use crate::error::{ClaudeSDKError, Result};
use crate::internal::control::{
    initialize_request, initialize_timeout_duration, respond_to_control_request,
    send_control_request_with_callbacks_and_timeout, ControlCallbacks,
};
use crate::internal::parser::parse_message_line;
use crate::internal::session_resume::{apply_materialized_options, materialize_resume_session};
use crate::internal::session_store_validation::validate_session_store_options;
use crate::internal::transcript_mirror::TranscriptMirrorBatcher;
use crate::internal::transport::{SubprocessCLITransport, Transport, TransportOptions};
use crate::types::{ClaudeAgentOptions, Message};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};

/// Token usage information from a query response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub total_tokens: i32,
}

/// Result from a one-shot query to Claude.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// The text content of the response.
    pub content: String,
    /// Token usage statistics, if available.
    pub usage: Option<TokenUsage>,
    /// The reason the response finished (e.g., "end_turn", "max_tokens").
    pub finish_reason: String,
}

/// Perform a one-shot query to Claude Code.
///
/// This is a convenience function for simple, stateless interactions where you do not
/// need bidirectional communication or conversation management. It creates a temporary
/// client, sends a single prompt, and returns the complete response.
///
/// # Arguments
///
/// * `prompt` - The prompt to send to Claude.
/// * `options` - Optional configuration options. If None, default options are used.
///
/// # Returns
///
/// A `QueryResult` containing the response content, usage statistics, and finish reason.
///
/// # Example
///
/// ```rust
/// use claude_code_sdk_rust::query;
///
/// async fn example() -> Result<(), Box<dyn std::error::Error>> {
///     let result = query("What is Rust?", None).await?;
///     println!("{}", result.content);
///     Ok(())
/// }
/// ```
pub async fn query(
    prompt: impl Into<String>,
    options: Option<ClaudeAgentOptions>,
) -> Result<QueryResult> {
    let messages = query_messages(prompt, options).await?;
    Ok(summarize_messages(messages))
}

/// Perform a one-shot query and return the full message sequence.
///
/// This is the Rust equivalent of the Python SDK's `query()` async iterator,
/// adapted to return the collected messages as a `Vec<Message>`.
pub async fn query_messages(
    prompt: impl Into<String>,
    options: Option<ClaudeAgentOptions>,
) -> Result<Vec<Message>> {
    let prompt = prompt.into();
    let mut options = options.unwrap_or_default();
    if options.can_use_tool.is_some() {
        return Err(ClaudeSDKError::Other(
            "can_use_tool callback requires streaming mode. \
             Please use ClaudeAgentClient instead of query_messages with a string prompt."
                .to_string(),
        ));
    }
    validate_session_store_options(&options)?;
    let materialized = materialize_resume_session(&options).await?;
    if let Some(materialized) = &materialized {
        options = apply_materialized_options(&options, materialized);
    }
    let result = run_query_messages(prompt, options).await;
    if let Some(materialized) = &materialized {
        materialized.cleanup().await;
    }
    result
}

pub async fn query_messages_with_transport(
    prompt: impl Into<String>,
    options: Option<ClaudeAgentOptions>,
    transport: Box<dyn Transport>,
) -> Result<Vec<Message>> {
    let prompt = prompt.into();
    let options = options.unwrap_or_default();
    if options.can_use_tool.is_some() {
        return Err(ClaudeSDKError::Other(
            "can_use_tool callback requires streaming mode. \
             Please use ClaudeAgentClient instead of query_messages with a string prompt."
                .to_string(),
        ));
    }
    validate_session_store_options(&options)?;
    run_query_messages_with_transport(prompt, options, transport).await
}

/// Perform a one-shot query from a stream of raw prompt messages.
///
/// Each streamed value is written to the Claude Code transport as one JSON line.
/// If a message is an object and does not include `session_id`, `"default"` is
/// inserted to match the interactive client's streaming prompt behavior.
pub async fn query_stream_messages<S>(
    stream: S,
    options: Option<ClaudeAgentOptions>,
) -> Result<Vec<Message>>
where
    S: Stream<Item = serde_json::Value> + Unpin,
{
    let mut options = options.unwrap_or_default();
    validate_session_store_options(&options)?;
    let materialized = materialize_resume_session(&options).await?;
    if let Some(materialized) = &materialized {
        options = apply_materialized_options(&options, materialized);
    }
    let result = run_query_stream_messages(stream, options).await;
    if let Some(materialized) = &materialized {
        materialized.cleanup().await;
    }
    result
}

pub async fn query_stream_messages_with_transport<S>(
    stream: S,
    options: Option<ClaudeAgentOptions>,
    transport: Box<dyn Transport>,
) -> Result<Vec<Message>>
where
    S: Stream<Item = serde_json::Value> + Unpin,
{
    let options = options.unwrap_or_default();
    validate_session_store_options(&options)?;
    run_query_stream_messages_with_transport(stream, options, transport).await
}

async fn run_query_messages(prompt: String, options: ClaudeAgentOptions) -> Result<Vec<Message>> {
    // Create transport with options
    let transport_options = TransportOptions::from(&options);
    let transport = SubprocessCLITransport::new(transport_options);
    run_query_messages_with_transport(prompt, options, Box::new(transport)).await
}

async fn run_query_stream_messages<S>(
    stream: S,
    options: ClaudeAgentOptions,
) -> Result<Vec<Message>>
where
    S: Stream<Item = serde_json::Value> + Unpin,
{
    let transport_options = TransportOptions::from(&options);
    let transport = SubprocessCLITransport::new(transport_options);
    run_query_stream_messages_with_transport(stream, options, Box::new(transport)).await
}

async fn run_query_messages_with_transport(
    prompt: String,
    options: ClaudeAgentOptions,
    mut transport: Box<dyn Transport>,
) -> Result<Vec<Message>> {
    let control_callbacks = ControlCallbacks::from_options(&options);
    let mut transcript_mirror = TranscriptMirrorBatcher::from_options(&options);

    // Connect to the CLI
    transport.connect().await?;
    send_control_request_with_callbacks_and_timeout(
        transport.as_mut(),
        initialize_request(&control_callbacks),
        &control_callbacks,
        initialize_timeout_duration(),
    )
    .await?;

    // Build and send the user message
    let user_message = serde_json::json!({
        "type": "user",
        "session_id": "",
        "message": {
            "role": "user",
            "content": prompt
        },
        "parent_tool_use_id": null
    });

    transport
        .write(format!("{}\n", user_message).as_bytes())
        .await?;

    let mut messages = Vec::new();

    // Read messages until we get the result
    while let Some(data) = transport.read().await? {
        let line = String::from_utf8_lossy(&data);
        let value = serde_json::from_slice::<serde_json::Value>(&data)?;
        if value.get("type").and_then(|v| v.as_str()) == Some("control_request") {
            respond_to_control_request(transport.as_mut(), &value, &control_callbacks).await?;
            continue;
        }
        if value.get("type").and_then(|v| v.as_str()) == Some("transcript_mirror") {
            if let Some(batcher) = &mut transcript_mirror {
                messages.extend(batcher.enqueue_value(&value).await?);
            }
            continue;
        }
        match parse_message_line(&line)? {
            Some(message @ Message::ResultMsg { .. }) => {
                flush_transcript_mirror(&mut transcript_mirror).await?;
                messages.push(message);
                break;
            }
            Some(message) => {
                messages.push(message);
            }
            None => {}
        }
    }

    // Close the transport
    flush_transcript_mirror(&mut transcript_mirror).await?;
    transport.close().await?;

    Ok(messages)
}

async fn run_query_stream_messages_with_transport<S>(
    mut stream: S,
    options: ClaudeAgentOptions,
    mut transport: Box<dyn Transport>,
) -> Result<Vec<Message>>
where
    S: Stream<Item = serde_json::Value> + Unpin,
{
    let control_callbacks = ControlCallbacks::from_options(&options);
    let mut transcript_mirror = TranscriptMirrorBatcher::from_options(&options);

    transport.connect().await?;
    send_control_request_with_callbacks_and_timeout(
        transport.as_mut(),
        initialize_request(&control_callbacks),
        &control_callbacks,
        initialize_timeout_duration(),
    )
    .await?;

    while let Some(mut message) = stream.next().await {
        if let Some(object) = message.as_object_mut() {
            object
                .entry("session_id")
                .or_insert_with(|| serde_json::Value::String("default".to_string()));
        }
        let mut json_payload = serde_json::to_vec(&message)?;
        json_payload.push(b'\n');
        transport.write(&json_payload).await?;
    }
    transport.close_input().await?;

    let mut messages = Vec::new();
    while let Some(data) = transport.read().await? {
        let line = String::from_utf8_lossy(&data);
        let value = serde_json::from_slice::<serde_json::Value>(&data)?;
        if value.get("type").and_then(|v| v.as_str()) == Some("control_request") {
            respond_to_control_request(transport.as_mut(), &value, &control_callbacks).await?;
            continue;
        }
        if value.get("type").and_then(|v| v.as_str()) == Some("transcript_mirror") {
            if let Some(batcher) = &mut transcript_mirror {
                messages.extend(batcher.enqueue_value(&value).await?);
            }
            continue;
        }
        match parse_message_line(&line)? {
            Some(message @ Message::ResultMsg { .. }) => {
                flush_transcript_mirror(&mut transcript_mirror).await?;
                messages.push(message);
                break;
            }
            Some(message) => {
                messages.push(message);
            }
            None => {}
        }
    }

    flush_transcript_mirror(&mut transcript_mirror).await?;
    transport.close().await?;

    Ok(messages)
}

fn summarize_messages(messages: Vec<Message>) -> QueryResult {
    let mut content_parts: Vec<String> = Vec::new();
    let mut usage: Option<TokenUsage> = None;
    let mut finish_reason = String::from("unknown");

    for message in messages {
        match message {
            Message::AssistantMsg { content, .. } => {
                for block in &content.content {
                    if let crate::types::ContentBlock::Text { text } = block {
                        content_parts.push(text.clone());
                    }
                }
            }
            Message::ResultMsg {
                usage: msg_usage,
                stop_reason,
                result,
                ..
            } => {
                if let Some(result_text) = result {
                    if content_parts.is_empty() {
                        content_parts.push(result_text);
                    }
                }
                if let Some(u) = msg_usage {
                    usage = extract_token_usage(&u);
                }
                if let Some(reason) = stop_reason {
                    finish_reason = reason;
                }
            }
            _ => {}
        }
    }

    QueryResult {
        content: content_parts.join(""),
        usage,
        finish_reason,
    }
}

async fn flush_transcript_mirror(
    transcript_mirror: &mut Option<TranscriptMirrorBatcher>,
) -> Result<()> {
    if let Some(batcher) = transcript_mirror {
        let _ = batcher.flush().await?;
    }
    Ok(())
}

/// Extract TokenUsage from a JSON map.
fn extract_token_usage(
    usage_map: &serde_json::Map<String, serde_json::Value>,
) -> Option<TokenUsage> {
    let input_tokens = usage_map
        .get("input_tokens")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)?;
    let output_tokens = usage_map
        .get("output_tokens")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)?;
    let total_tokens = usage_map
        .get("total_tokens")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)?;

    Some(TokenUsage {
        input_tokens,
        output_tokens,
        total_tokens,
    })
}
