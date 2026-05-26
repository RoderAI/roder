use ratatui::{layout::Rect, text::Line, widgets::Paragraph};
use roder_api::events::{EventEnvelope, EventSource, RoderEvent, RuntimeStarted};
use roder_api_transcript::{
    ApiTranscriptRecord, RecordedFrame, RecordedTerminalSize, RecordedUiInput,
};
use roder_app_server::{AppClient, AppEventReceiver, AppNotificationReceiver};
use roder_protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use roder_tui::{
    frame_snapshot::frame_text_hash,
    replay::{FixedReplayClock, HeadlessReplayDriver, ReplayInputSource, ReplayTranscript},
    runtime_io::{TuiClock, TuiInputSource},
};
use serde_json::json;
use std::time::{Duration, Instant};
use time::OffsetDateTime;

#[tokio::test]
async fn replay_client_matches_requests_and_replays_events_and_notifications() {
    let envelope = EventEnvelope {
        event_id: "event-1".to_string(),
        seq: 1,
        timestamp: OffsetDateTime::UNIX_EPOCH,
        source: EventSource::Runtime,
        kind: "runtime.started".to_string(),
        thread_id: None,
        turn_id: None,
        event: RoderEvent::RuntimeStarted(RuntimeStarted {
            timestamp: OffsetDateTime::UNIX_EPOCH,
        }),
    };
    let notification = JsonRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: "item/agentMessage/delta".to_string(),
        params: json!({"delta": "hi"}),
    };
    let transcript = ReplayTranscript::from_records(vec![
        ApiTranscriptRecord::ApiRequest {
            seq: 1,
            at_ms: 0,
            client: "tui".to_string(),
            request: json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "thread/state"
            }),
        },
        ApiTranscriptRecord::ApiResponse {
            seq: 2,
            at_ms: 1,
            request_seq: 1,
            response: json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {"policyMode": "default"}
            }),
        },
        ApiTranscriptRecord::RuntimeEvent {
            seq: 3,
            at_ms: 2,
            envelope: serde_json::to_value(&envelope).unwrap(),
        },
        ApiTranscriptRecord::ApiNotification {
            seq: 4,
            at_ms: 3,
            notification: serde_json::to_value(&notification).unwrap(),
        },
        ApiTranscriptRecord::UiInput {
            seq: 5,
            at_ms: 4,
            event: RecordedUiInput::Resize { cols: 10, rows: 2 },
        },
    ])
    .unwrap();
    assert_eq!(transcript.inputs().len(), 1);

    let client = transcript.into_client();
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "thread/state".to_string(),
            params: None,
        })
        .await;

    assert_eq!(
        response.result,
        Some(json!({
            "policyMode": "default"
        }))
    );
    let mut events = client.subscribe_events();
    assert_eq!(events.recv().await.unwrap().event_id, "event-1");
    let mut notifications = client.subscribe_notifications();
    assert_eq!(
        notifications.recv().await.unwrap().method,
        "item/agentMessage/delta"
    );
}

#[tokio::test]
async fn replay_client_reports_first_request_mismatch() {
    let transcript = ReplayTranscript::from_records(vec![
        ApiTranscriptRecord::ApiRequest {
            seq: 1,
            at_ms: 0,
            client: "tui".to_string(),
            request: serde_json::to_value(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(1)),
                method: "thread/state".to_string(),
                params: None,
            })
            .unwrap(),
        },
        ApiTranscriptRecord::ApiResponse {
            seq: 2,
            at_ms: 1,
            request_seq: 1,
            response: serde_json::to_value(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(1)),
                result: Some(json!({})),
                error: None,
            })
            .unwrap(),
        },
    ])
    .unwrap();

    let response = transcript
        .into_client()
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(2)),
            method: "settings/get".to_string(),
            params: None,
        })
        .await;

    assert!(response.error.unwrap().message.contains("request mismatch"));
}

#[test]
fn headless_replay_driver_compares_normalized_frame_text() {
    let expected_text = "ready";
    let mut driver = HeadlessReplayDriver::new(
        RecordedTerminalSize { cols: 10, rows: 2 },
        [RecordedFrame {
            cols: 10,
            rows: 2,
            text_hash: frame_text_hash(expected_text),
            text: Some(expected_text.to_string()),
            artifacts: Vec::new(),
        }]
        .into(),
    );

    driver
        .draw_and_assert_next_frame(|frame| {
            frame.render_widget(Paragraph::new(Line::raw("ready")), Rect::new(0, 0, 10, 2));
        })
        .unwrap();

    assert_eq!(driver.remaining_frame_count(), 0);
}

#[test]
fn api_replay_fixture_startup_frame() {
    assert_fixture_frame("ready\nmodel: gpt-5.5");
}

#[test]
fn api_replay_fixture_slash_command_menu_frame() {
    assert_fixture_frame("Slash commands\n/help\n/ps\n/plugin");
}

#[test]
fn api_replay_fixture_ctrl_p_palette_frame() {
    assert_fixture_frame("Command palette\nModels\nProviders\nSettings");
}

#[test]
fn api_replay_fixture_session_resume_frame() {
    assert_fixture_frame("Resumed session\nPrevious prompt\nAssistant response");
}

#[test]
fn api_replay_fixture_streamed_assistant_output_frame() {
    assert_fixture_frame("Working\nassistant: streaming delta");
}

#[test]
fn api_replay_fixture_policy_approval_frame() {
    assert_fixture_frame("Approval requested\nAccept\nReject");
}

#[test]
fn replay_input_source_delivers_recorded_inputs_in_order() {
    let mut input = ReplayInputSource::new([
        RecordedUiInput::Key {
            code: "char".to_string(),
            char: Some('/'),
            modifiers: Vec::new(),
        },
        RecordedUiInput::Paste {
            text: "hello".to_string(),
        },
        RecordedUiInput::Resize { cols: 80, rows: 24 },
    ])
    .unwrap();

    assert!(input.poll(Duration::ZERO).unwrap());
    assert!(matches!(
        input.read().unwrap(),
        crossterm::event::Event::Key(_)
    ));
    assert_eq!(
        input.read().unwrap(),
        crossterm::event::Event::Paste("hello".to_string())
    );
    assert_eq!(
        input.read().unwrap(),
        crossterm::event::Event::Resize(80, 24)
    );
    assert!(!input.poll(Duration::ZERO).unwrap());
}

#[test]
fn fixed_replay_clock_advances_deterministically() {
    let start = Instant::now();
    let mut clock = FixedReplayClock::new(start);

    clock.advance(Duration::from_millis(250));

    assert_eq!(clock.now(), start + Duration::from_millis(250));
}

fn assert_fixture_frame(text: &str) {
    let rows = text.lines().count().max(1) as u16;
    let cols = text.lines().map(str::len).max().unwrap_or(1).max(1) as u16;
    let mut driver = HeadlessReplayDriver::new(
        RecordedTerminalSize { cols, rows },
        [RecordedFrame {
            cols,
            rows,
            text_hash: frame_text_hash(text),
            text: Some(text.to_string()),
            artifacts: Vec::new(),
        }]
        .into(),
    );

    driver
        .draw_and_assert_next_frame(|frame| {
            frame.render_widget(
                Paragraph::new(text.to_string()),
                Rect::new(0, 0, cols, rows),
            );
        })
        .unwrap();
}
