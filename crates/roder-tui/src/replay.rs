use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use roder_api::events::EventEnvelope;
use roder_api_transcript::{
    ApiTranscriptRecord, RecordedFrame, RecordedMouseButton, RecordedMouseEventKind,
    RecordedTerminalSize, RecordedUiInput,
};
use roder_app_server_core::{AppClient, AppEventReceiver, AppNotificationReceiver};
use roder_protocol::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use tokio::sync::broadcast;

use crate::{
    frame_snapshot::{frame_text_hash, normalized_frame_text},
    runtime_io::{TuiClock, TuiInputSource},
};

#[derive(Debug, Clone)]
pub struct ReplayTranscript {
    requests: VecDeque<ReplayRequestPair>,
    events: VecDeque<EventEnvelope>,
    notifications: VecDeque<JsonRpcNotification>,
    inputs: VecDeque<RecordedUiInput>,
    frames: VecDeque<RecordedFrame>,
}

impl ReplayTranscript {
    pub fn from_records(records: Vec<ApiTranscriptRecord>) -> anyhow::Result<Self> {
        let mut requests = VecDeque::new();
        let mut responses = BTreeMap::new();
        let mut events = VecDeque::new();
        let mut notifications = VecDeque::new();
        let mut inputs = VecDeque::new();
        let mut frames = VecDeque::new();

        for record in records {
            record.validate()?;
            match record {
                ApiTranscriptRecord::Header(_) => {}
                ApiTranscriptRecord::ApiRequest { seq, request, .. } => {
                    let request = serde_json::from_value::<JsonRpcRequest>(request)?;
                    requests.push_back(ReplayRequestPair {
                        request_seq: seq,
                        request,
                        response: None,
                    });
                }
                ApiTranscriptRecord::ApiResponse {
                    request_seq,
                    response,
                    ..
                } => {
                    responses.insert(request_seq, serde_json::from_value(response)?);
                }
                ApiTranscriptRecord::ApiNotification { notification, .. } => {
                    notifications.push_back(serde_json::from_value(notification)?);
                }
                ApiTranscriptRecord::RuntimeEvent { envelope, .. } => {
                    events.push_back(serde_json::from_value(envelope)?);
                }
                ApiTranscriptRecord::UiInput { event, .. } => inputs.push_back(event),
                ApiTranscriptRecord::UiFrame { frame, .. } => frames.push_back(frame),
                ApiTranscriptRecord::ExtensionEvent { .. }
                | ApiTranscriptRecord::Artifact { .. }
                | ApiTranscriptRecord::BroadcastLag { .. } => {}
            }
        }

        for pair in &mut requests {
            let Some(response) = responses.remove(&pair.request_seq) else {
                anyhow::bail!("missing api.response for request seq {}", pair.request_seq);
            };
            pair.response = Some(response);
        }
        if let Some((request_seq, _)) = responses.into_iter().next() {
            anyhow::bail!("api.response references unknown request seq {request_seq}");
        }

        Ok(Self {
            requests,
            events,
            notifications,
            inputs,
            frames,
        })
    }

    pub fn into_client(self) -> ReplayAppClient {
        ReplayAppClient::new(self)
    }

    pub fn inputs(&self) -> &VecDeque<RecordedUiInput> {
        &self.inputs
    }

    pub fn frames(&self) -> &VecDeque<RecordedFrame> {
        &self.frames
    }

    pub fn into_inputs(self) -> VecDeque<RecordedUiInput> {
        self.inputs
    }
}

#[derive(Debug, Clone)]
struct ReplayRequestPair {
    request_seq: u64,
    request: JsonRpcRequest,
    response: Option<JsonRpcResponse>,
}

#[derive(Debug, Clone)]
pub struct ReplayAppClient {
    state: Arc<Mutex<ReplayState>>,
}

#[derive(Debug)]
struct ReplayState {
    requests: VecDeque<ReplayRequestPair>,
    events: VecDeque<EventEnvelope>,
    notifications: VecDeque<JsonRpcNotification>,
}

impl ReplayAppClient {
    pub fn new(transcript: ReplayTranscript) -> Self {
        Self {
            state: Arc::new(Mutex::new(ReplayState {
                requests: transcript.requests,
                events: transcript.events,
                notifications: transcript.notifications,
            })),
        }
    }

    fn mismatch_response(request: JsonRpcRequest, message: String) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: None,
            error: Some(JsonRpcError {
                code: -32098,
                message,
                data: None,
            }),
        }
    }
}

#[async_trait]
impl AppClient for ReplayAppClient {
    type EventReceiver = ReplayEventReceiver;
    type NotificationReceiver = ReplayNotificationReceiver;

    async fn send_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let mut state = self.state.lock().expect("replay state mutex poisoned");
        let Some(pair) = state.requests.pop_front() else {
            return Self::mismatch_response(
                request,
                "replay transcript has no remaining api.request records".to_string(),
            );
        };
        if pair.request.method != request.method || pair.request.params != request.params {
            let message = format!(
                "replay request mismatch: expected {} {:?}, got {} {:?}",
                pair.request.method, pair.request.params, request.method, request.params
            );
            return Self::mismatch_response(request, message);
        }
        pair.response
            .expect("replay request pair is populated during transcript load")
    }

    fn subscribe_events(&self) -> Self::EventReceiver {
        let state = self.state.lock().expect("replay state mutex poisoned");
        ReplayEventReceiver {
            events: Arc::new(Mutex::new(state.events.clone())),
        }
    }

    fn subscribe_notifications(&self) -> Self::NotificationReceiver {
        let state = self.state.lock().expect("replay state mutex poisoned");
        ReplayNotificationReceiver {
            notifications: Arc::new(Mutex::new(state.notifications.clone())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReplayEventReceiver {
    events: Arc<Mutex<VecDeque<EventEnvelope>>>,
}

#[async_trait]
impl AppEventReceiver for ReplayEventReceiver {
    async fn recv(&mut self) -> Result<EventEnvelope, broadcast::error::RecvError> {
        self.events
            .lock()
            .expect("replay event receiver mutex poisoned")
            .pop_front()
            .ok_or(broadcast::error::RecvError::Closed)
    }

    fn try_recv(&mut self) -> Result<EventEnvelope, broadcast::error::TryRecvError> {
        self.events
            .lock()
            .expect("replay event receiver mutex poisoned")
            .pop_front()
            .ok_or(broadcast::error::TryRecvError::Empty)
    }
}

#[derive(Debug, Clone)]
pub struct ReplayNotificationReceiver {
    notifications: Arc<Mutex<VecDeque<JsonRpcNotification>>>,
}

#[async_trait]
impl AppNotificationReceiver for ReplayNotificationReceiver {
    async fn recv(&mut self) -> Result<JsonRpcNotification, broadcast::error::RecvError> {
        self.notifications
            .lock()
            .expect("replay notification receiver mutex poisoned")
            .pop_front()
            .ok_or(broadcast::error::RecvError::Closed)
    }

    fn try_recv(&mut self) -> Result<JsonRpcNotification, broadcast::error::TryRecvError> {
        self.notifications
            .lock()
            .expect("replay notification receiver mutex poisoned")
            .pop_front()
            .ok_or(broadcast::error::TryRecvError::Empty)
    }
}

pub struct HeadlessReplayDriver {
    terminal: Terminal<TestBackend>,
    expected_frames: VecDeque<RecordedFrame>,
}

impl HeadlessReplayDriver {
    pub fn new(terminal: RecordedTerminalSize, expected_frames: VecDeque<RecordedFrame>) -> Self {
        Self {
            terminal: Terminal::new(TestBackend::new(terminal.cols, terminal.rows))
                .expect("construct test terminal"),
            expected_frames,
        }
    }

    pub fn draw_and_assert_next_frame<F>(&mut self, render: F) -> anyhow::Result<()>
    where
        F: FnOnce(&mut ratatui::Frame<'_>),
    {
        self.terminal.draw(render)?;
        let Some(expected) = self.expected_frames.pop_front() else {
            anyhow::bail!("no expected ui.frame remains for rendered frame");
        };
        let buffer = self.terminal.backend().buffer();
        let text = normalized_frame_text(buffer);
        let text_hash = frame_text_hash(&text);
        if expected.cols != buffer.area.width || expected.rows != buffer.area.height {
            anyhow::bail!(
                "frame size mismatch: expected {}x{}, got {}x{}",
                expected.cols,
                expected.rows,
                buffer.area.width,
                buffer.area.height
            );
        }
        if expected.text_hash != text_hash {
            anyhow::bail!(
                "frame hash mismatch: expected {}, got {}",
                expected.text_hash,
                text_hash
            );
        }
        if let Some(expected_text) = expected.text
            && expected_text != text
        {
            anyhow::bail!("frame text mismatch:\nexpected:\n{expected_text}\nactual:\n{text}");
        }
        Ok(())
    }

    pub fn remaining_frame_count(&self) -> usize {
        self.expected_frames.len()
    }

    pub fn area(&self) -> Rect {
        self.terminal.backend().buffer().area
    }
}

pub struct ReplayInputSource {
    events: VecDeque<Event>,
}

impl ReplayInputSource {
    pub fn new(inputs: impl IntoIterator<Item = RecordedUiInput>) -> anyhow::Result<Self> {
        let events = inputs
            .into_iter()
            .map(event_from_recorded_input)
            .collect::<anyhow::Result<VecDeque<_>>>()?;
        Ok(Self { events })
    }

    pub fn remaining_event_count(&self) -> usize {
        self.events.len()
    }
}

impl TuiInputSource for ReplayInputSource {
    fn poll(&self, _timeout: Duration) -> io::Result<bool> {
        Ok(!self.events.is_empty())
    }

    fn read(&mut self) -> io::Result<Event> {
        self.events
            .pop_front()
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "replay input exhausted"))
    }
}

#[derive(Debug, Clone)]
pub struct FixedReplayClock {
    now: Instant,
}

impl FixedReplayClock {
    pub fn new(start: Instant) -> Self {
        Self { now: start }
    }

    pub fn advance(&mut self, duration: Duration) {
        self.now += duration;
    }
}

impl TuiClock for FixedReplayClock {
    fn now(&self) -> Instant {
        self.now
    }
}

fn event_from_recorded_input(input: RecordedUiInput) -> anyhow::Result<Event> {
    match input {
        RecordedUiInput::Key {
            code,
            char,
            modifiers,
        } => Ok(Event::Key(KeyEvent::new(
            key_code_from_recorded(&code, char)?,
            key_modifiers_from_recorded(&modifiers),
        ))),
        RecordedUiInput::Paste { text } => Ok(Event::Paste(text)),
        RecordedUiInput::Mouse {
            kind,
            column,
            row,
            modifiers,
        } => Ok(Event::Mouse(MouseEvent {
            kind: mouse_kind_from_recorded(kind)?,
            column,
            row,
            modifiers: key_modifiers_from_recorded(&modifiers),
        })),
        RecordedUiInput::Resize { cols, rows } => Ok(Event::Resize(cols, rows)),
        RecordedUiInput::ReplayControl { command } => {
            anyhow::bail!("replay control input cannot be delivered as crossterm event: {command}")
        }
    }
}

fn key_code_from_recorded(code: &str, char: Option<char>) -> anyhow::Result<KeyCode> {
    Ok(match code {
        "backspace" => KeyCode::Backspace,
        "enter" => KeyCode::Enter,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "page-up" => KeyCode::PageUp,
        "page-down" => KeyCode::PageDown,
        "tab" => KeyCode::Tab,
        "back-tab" => KeyCode::BackTab,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "char" => KeyCode::Char(char.ok_or_else(|| anyhow::anyhow!("missing char payload"))?),
        "null" => KeyCode::Null,
        "escape" => KeyCode::Esc,
        "caps-lock" => KeyCode::CapsLock,
        "scroll-lock" => KeyCode::ScrollLock,
        "num-lock" => KeyCode::NumLock,
        "print-screen" => KeyCode::PrintScreen,
        "pause" => KeyCode::Pause,
        "menu" => KeyCode::Menu,
        "keypad-begin" => KeyCode::KeypadBegin,
        function if function.starts_with('f') => {
            let number = function[1..].parse::<u8>()?;
            KeyCode::F(number)
        }
        other => anyhow::bail!("unsupported recorded key code: {other}"),
    })
}

fn key_modifiers_from_recorded(modifiers: &[String]) -> KeyModifiers {
    let mut out = KeyModifiers::empty();
    for modifier in modifiers {
        match modifier.as_str() {
            "control" => out |= KeyModifiers::CONTROL,
            "alt" => out |= KeyModifiers::ALT,
            "shift" => out |= KeyModifiers::SHIFT,
            "super" => out |= KeyModifiers::SUPER,
            "hyper" => out |= KeyModifiers::HYPER,
            "meta" => out |= KeyModifiers::META,
            _ => {}
        }
    }
    out
}

fn mouse_kind_from_recorded(kind: RecordedMouseEventKind) -> anyhow::Result<MouseEventKind> {
    Ok(match kind {
        RecordedMouseEventKind::Down { button } => {
            MouseEventKind::Down(mouse_button_from_recorded(button))
        }
        RecordedMouseEventKind::Up { button } => {
            MouseEventKind::Up(mouse_button_from_recorded(button))
        }
        RecordedMouseEventKind::Drag { button } => {
            MouseEventKind::Drag(mouse_button_from_recorded(button))
        }
        RecordedMouseEventKind::Moved => MouseEventKind::Moved,
        RecordedMouseEventKind::ScrollDown => MouseEventKind::ScrollDown,
        RecordedMouseEventKind::ScrollUp => MouseEventKind::ScrollUp,
        RecordedMouseEventKind::ScrollLeft => MouseEventKind::ScrollLeft,
        RecordedMouseEventKind::ScrollRight => MouseEventKind::ScrollRight,
    })
}

fn mouse_button_from_recorded(button: RecordedMouseButton) -> MouseButton {
    match button {
        RecordedMouseButton::Left => MouseButton::Left,
        RecordedMouseButton::Right => MouseButton::Right,
        RecordedMouseButton::Middle => MouseButton::Middle,
    }
}
