//! End-to-end regression test for the bidi same-stream exec loop against a
//! local mock AgentService (HTTP/2 + Connect framing).
//!
//! Reproduces the composer-2.5 "stuck on tool calls" failure offline: the
//! server issues an exec request variant Roder does not implement and refuses
//! to end the turn until *some* result with the mirrored seq + field number
//! arrives. Before the Unknown-exec handling, the client silently dropped the
//! frame and the turn hung until the no-progress cap killed it; now the client
//! replies and the turn completes promptly.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use roder_api::inference::{InferenceEvent, ToolCallCompleted, TurnToolExecutor, TurnToolOutcome};

use crate::agentservice::AgentServiceConfig;
use crate::bidi::{BidiRequest, BidiUsageMetadata, run_bidi_turn};
use crate::proto::{
    encode_connect_frame, proto_field_bytes, proto_field_string, proto_field_varint, proto_message,
};

const UNKNOWN_EXEC_FIELD: u32 = 21;
const EXEC_SEQ: u64 = 7;

#[derive(Default)]
struct RecordingExecutor {
    calls: Mutex<Vec<ToolCallCompleted>>,
}

#[async_trait::async_trait]
impl TurnToolExecutor for RecordingExecutor {
    async fn execute(&self, call: ToolCallCompleted) -> anyhow::Result<TurnToolOutcome> {
        self.calls.lock().unwrap().push(call);
        Ok(TurnToolOutcome {
            result: "unknown tool".to_string(),
            is_error: true,
        })
    }
}

/// `AgentServerMessage{ 2: ExecServerMessage{ 1: seq, 21: { 1: "arg" } } }`
/// — an exec request slot Roder has no handler for.
fn unknown_exec_request_frame() -> Vec<u8> {
    encode_connect_frame(&proto_message(vec![proto_field_bytes(
        2,
        proto_message(vec![
            proto_field_varint(1, EXEC_SEQ),
            proto_field_bytes(
                UNKNOWN_EXEC_FIELD,
                proto_message(vec![proto_field_string(1, "mystery/arg")]),
            ),
        ]),
    )]))
}

/// `AgentServerMessage{ 1: InteractionUpdate{ 1: { 1: text } } }`
fn text_frame(text: &str) -> Vec<u8> {
    encode_connect_frame(&proto_message(vec![proto_field_bytes(
        1,
        proto_message(vec![proto_field_bytes(
            1,
            proto_message(vec![proto_field_string(1, text)]),
        )]),
    )]))
}

/// `AgentServerMessage{ 1: InteractionUpdate{ 14: usage } }` — turn end.
fn turn_end_frame() -> Vec<u8> {
    encode_connect_frame(&proto_message(vec![proto_field_bytes(
        1,
        proto_message(vec![proto_field_bytes(
            14,
            proto_message(vec![proto_field_varint(1, 10)]),
        )]),
    )]))
}

/// Connect end-stream frame (flag 2) with an empty JSON object (no error).
fn connect_end_stream_frame() -> Vec<u8> {
    let payload = b"{}";
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(2);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// Whether the accumulated client body contains the mirrored Unknown-exec
/// result: `AgentClientMessage{ 2: ExecClientMessage{ 1: seq, 21: {} } }`.
fn contains_mirrored_unknown_result(buffer: &mut Vec<u8>) -> bool {
    loop {
        match crate::proto::take_connect_frame(buffer) {
            Ok(Some(crate::proto::ConnectFrame::Payload(payload))) => {
                let Some(exec) = crate::proto::submessage(&payload, 2) else {
                    continue;
                };
                if crate::proto::scalar_u64(&exec, 1) == Some(EXEC_SEQ)
                    && crate::proto::submessage(&exec, UNKNOWN_EXEC_FIELD).is_some()
                {
                    return true;
                }
            }
            Ok(Some(crate::proto::ConnectFrame::EndStream(_))) => continue,
            Ok(None) | Err(_) => return false,
        }
    }
}

/// Mock AgentService: one h2 stream that sends an unknown exec request and
/// ends the turn only after the client's mirrored result arrives.
async fn run_mock_agent_service(listener: tokio::net::TcpListener) {
    let (socket, _) = listener.accept().await.expect("accept TCP");
    let mut connection = h2::server::handshake(socket).await.expect("h2 handshake");
    let (request, mut respond) = connection
        .accept()
        .await
        .expect("one request")
        .expect("h2 request");
    // Keep polling the connection so stream I/O makes progress.
    let driver = tokio::spawn(async move { while connection.accept().await.is_some() {} });

    let mut body = request.into_body();
    let response = http::Response::builder()
        .status(200)
        .header("content-type", "application/connect+proto")
        .body(())
        .expect("response");
    let mut send = respond
        .send_response(response, false)
        .expect("send headers");

    send.send_data(unknown_exec_request_frame().into(), false)
        .expect("send exec request");

    let mut buffer: Vec<u8> = Vec::new();
    let mut got_reply = false;
    while !got_reply {
        let Some(chunk) = body.data().await else {
            break;
        };
        let chunk = chunk.expect("request body chunk");
        let _ = body.flow_control().release_capacity(chunk.len());
        buffer.extend_from_slice(&chunk);
        got_reply = contains_mirrored_unknown_result(&mut buffer);
    }
    assert!(got_reply, "client must reply to the unknown exec request");

    send.send_data(text_frame("done").into(), false)
        .expect("send text");
    send.send_data(turn_end_frame().into(), false)
        .expect("send turn end");
    send.send_data(connect_end_stream_frame().into(), true)
        .expect("send end-stream");
    // Let the connection driver flush the final frames and close naturally
    // when the client drops the stream; aborting it here breaks the pipe
    // before the turn-end frames are delivered.
    let _ = tokio::time::timeout(Duration::from_secs(10), driver).await;
}

#[tokio::test]
async fn bidi_turn_replies_to_unknown_exec_and_completes_instead_of_hanging() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(run_mock_agent_service(listener));

    let config = AgentServiceConfig {
        endpoint: format!("http://{addr}"),
        ..AgentServiceConfig::default()
    };
    let executor = Arc::new(RecordingExecutor::default());
    let request = BidiRequest {
        access_token: "test-token".to_string(),
        run_request: proto_message(vec![proto_field_bytes(1, Vec::new())]),
        context_frames: Vec::new(),
        workspace: std::env::temp_dir(),
        tool_executor: Some(executor.clone()),
        usage_metadata: BidiUsageMetadata {
            prompt_tokens: 7,
            provider: "cursor".to_string(),
            transport: "cursor-agentservice-http2-connect-proto-bidi".to_string(),
            auth_source: "test".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            model: "cursor/opus-4.8".to_string(),
            conversation_id: "conversation-1".to_string(),
        },
    };

    // Well under the 90s no-progress cap: with the fix the turn completes as
    // soon as the server sees the mirrored reply; without it this times out.
    let events = tokio::time::timeout(Duration::from_secs(20), async {
        let mut stream = run_bidi_turn(config, request).await.expect("bidi turn");
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.expect("inference event"));
        }
        events
    })
    .await
    .expect("bidi turn must complete promptly instead of hanging on the tool call");

    server.await.expect("mock server");

    let calls = executor.calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "unknown exec surfaces one timeline entry");
    assert_eq!(calls[0].name, "cursor_unsupported_tool");

    assert!(
        events.iter().any(
            |event| matches!(event, InferenceEvent::MessageDelta(delta) if delta.text == "done")
        )
    );
    let usage_index = events
        .iter()
        .position(|event| matches!(event, InferenceEvent::Usage(_)))
        .expect("bidi turn should emit token usage before completion");
    assert!(matches!(
        &events[usage_index],
        InferenceEvent::Usage(usage)
            if usage.prompt_tokens == 7
                && usage.completion_tokens == 1
                && usage.total_tokens == 8
    ));
    assert!(matches!(
        events.get(usage_index + 1),
        Some(InferenceEvent::ProviderMetadata(metadata))
            if metadata.get("usageEstimated").and_then(|value| value.as_bool()) == Some(true)
                && metadata
                    .get("usageFields")
                    .and_then(|fields| fields.get("field_1"))
                    .and_then(|value| value.as_u64())
                    == Some(10)
    ));
    assert!(matches!(
        events.last(),
        Some(InferenceEvent::Completed(metadata))
            if metadata.stop_reason.as_deref() == Some("turn_ended")
    ));
}
