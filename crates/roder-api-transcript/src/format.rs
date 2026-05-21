use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTranscriptHeader {
    pub schema_version: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub roder_version: String,
    pub cwd: String,
    pub terminal: RecordedTerminalSize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

impl ApiTranscriptHeader {
    pub fn validate(&self) -> Result<(), TranscriptFormatError> {
        if self.schema_version == SUPPORTED_SCHEMA_VERSION {
            Ok(())
        } else {
            Err(TranscriptFormatError::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: SUPPORTED_SCHEMA_VERSION,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedTerminalSize {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ApiTranscriptRecord {
    #[serde(rename = "header")]
    Header(ApiTranscriptHeader),
    #[serde(rename = "api.request", rename_all = "camelCase")]
    ApiRequest {
        seq: u64,
        #[serde(rename = "atMs")]
        at_ms: u64,
        client: String,
        request: Value,
    },
    #[serde(rename = "api.response", rename_all = "camelCase")]
    ApiResponse {
        seq: u64,
        #[serde(rename = "atMs")]
        at_ms: u64,
        request_seq: u64,
        response: Value,
    },
    #[serde(rename = "api.notification", rename_all = "camelCase")]
    ApiNotification {
        seq: u64,
        #[serde(rename = "atMs")]
        at_ms: u64,
        notification: Value,
    },
    #[serde(rename = "runtime.event", rename_all = "camelCase")]
    RuntimeEvent {
        seq: u64,
        #[serde(rename = "atMs")]
        at_ms: u64,
        envelope: Value,
    },
    #[serde(rename = "extension.event", rename_all = "camelCase")]
    ExtensionEvent {
        seq: u64,
        #[serde(rename = "atMs")]
        at_ms: u64,
        event: RecordedExtensionEvent,
    },
    #[serde(rename = "ui.input", rename_all = "camelCase")]
    UiInput {
        seq: u64,
        #[serde(rename = "atMs")]
        at_ms: u64,
        event: RecordedUiInput,
    },
    #[serde(rename = "ui.frame", rename_all = "camelCase")]
    UiFrame {
        seq: u64,
        #[serde(rename = "atMs")]
        at_ms: u64,
        frame: RecordedFrame,
    },
    #[serde(rename = "artifact", rename_all = "camelCase")]
    Artifact {
        seq: u64,
        #[serde(rename = "atMs")]
        at_ms: u64,
        artifact: RecordedArtifactRef,
    },
}

impl ApiTranscriptRecord {
    pub fn validate(&self) -> Result<(), TranscriptFormatError> {
        match self {
            Self::Header(header) => header.validate(),
            _ => Ok(()),
        }
    }

    pub fn seq(&self) -> Option<u64> {
        match self {
            Self::Header(_) => None,
            Self::ApiRequest { seq, .. }
            | Self::ApiResponse { seq, .. }
            | Self::ApiNotification { seq, .. }
            | Self::RuntimeEvent { seq, .. }
            | Self::ExtensionEvent { seq, .. }
            | Self::UiInput { seq, .. }
            | Self::UiFrame { seq, .. }
            | Self::Artifact { seq, .. } => Some(*seq),
        }
    }

    pub fn transcript_kind(&self) -> ApiTranscriptKind {
        match self {
            Self::Header(_) => ApiTranscriptKind::Header,
            Self::ApiRequest { .. } => ApiTranscriptKind::ApiRequest,
            Self::ApiResponse { .. } => ApiTranscriptKind::ApiResponse,
            Self::ApiNotification { .. } => ApiTranscriptKind::ApiNotification,
            Self::RuntimeEvent { .. } => ApiTranscriptKind::RuntimeEvent,
            Self::ExtensionEvent { .. } => ApiTranscriptKind::ExtensionEvent,
            Self::UiInput { .. } => ApiTranscriptKind::UiInput,
            Self::UiFrame { .. } => ApiTranscriptKind::UiFrame,
            Self::Artifact { .. } => ApiTranscriptKind::Artifact,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApiTranscriptKind {
    Header,
    ApiRequest,
    ApiResponse,
    ApiNotification,
    RuntimeEvent,
    ExtensionEvent,
    UiInput,
    UiFrame,
    Artifact,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedExtensionEvent {
    pub extension_id: String,
    pub topic: String,
    pub direction: RecordedExtensionDirection,
    pub payload: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordedExtensionDirection {
    Subscribe,
    Emit,
    Receive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum RecordedUiInput {
    Key {
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        char: Option<char>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        modifiers: Vec<String>,
    },
    Paste {
        text: String,
    },
    Mouse {
        kind: RecordedMouseEventKind,
        column: u16,
        row: u16,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        modifiers: Vec<String>,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    ReplayControl {
        command: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RecordedMouseEventKind {
    Down { button: RecordedMouseButton },
    Up { button: RecordedMouseButton },
    Drag { button: RecordedMouseButton },
    Moved,
    ScrollDown,
    ScrollUp,
    ScrollLeft,
    ScrollRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordedMouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedFrame {
    pub cols: u16,
    pub rows: u16,
    pub text_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<RecordedArtifactRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedArtifactRef {
    pub path: String,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptFormatError {
    UnsupportedSchemaVersion { found: u32, supported: u32 },
}

impl fmt::Display for TranscriptFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion { found, supported } => write!(
                f,
                "unsupported transcript schema version {found}; supported version is {supported}"
            ),
        }
    }
}

impl std::error::Error for TranscriptFormatError {}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use time::OffsetDateTime;

    use super::*;

    #[test]
    fn header_round_trips_and_validates_schema_version() {
        let header = header();
        let value = serde_json::to_string(&ApiTranscriptRecord::Header(header.clone())).unwrap();
        let parsed: ApiTranscriptRecord = serde_json::from_str(&value).unwrap();

        assert_eq!(parsed, ApiTranscriptRecord::Header(header));
        assert!(parsed.validate().is_ok());
        assert_eq!(parsed.transcript_kind(), ApiTranscriptKind::Header);
    }

    #[test]
    fn unsupported_transcript_version_fails_before_replay() {
        let mut header = header();
        header.schema_version = SUPPORTED_SCHEMA_VERSION + 1;

        let err = header.validate().unwrap_err();

        assert_eq!(
            err.to_string(),
            "unsupported transcript schema version 2; supported version is 1"
        );
    }

    #[test]
    fn request_response_notification_runtime_extension_input_resize_and_frame_round_trip() {
        let records = vec![
            ApiTranscriptRecord::ApiRequest {
                seq: 1,
                at_ms: 0,
                client: "tui".to_string(),
                request: json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "thread/start",
                    "params": {"workspace": "/tmp/redacted"}
                }),
            },
            ApiTranscriptRecord::ApiResponse {
                seq: 2,
                at_ms: 10,
                request_seq: 1,
                response: json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {"ok": true}
                }),
            },
            ApiTranscriptRecord::ApiNotification {
                seq: 3,
                at_ms: 20,
                notification: json!({
                    "jsonrpc": "2.0",
                    "method": "item/agentMessage/delta",
                    "params": {"delta": "hello"}
                }),
            },
            ApiTranscriptRecord::RuntimeEvent {
                seq: 4,
                at_ms: 30,
                envelope: json!({
                    "event_id": "event-42",
                    "seq": 42,
                    "timestamp": "1970-01-01T00:00:00Z",
                    "source": "core",
                    "kind": "runtime.started",
                    "thread_id": null,
                    "turn_id": null,
                    "event": {"RuntimeStarted": {"timestamp": "1970-01-01T00:00:00Z"}}
                }),
            },
            ApiTranscriptRecord::ExtensionEvent {
                seq: 5,
                at_ms: 40,
                event: RecordedExtensionEvent {
                    extension_id: "ext-a".to_string(),
                    topic: "tools".to_string(),
                    direction: RecordedExtensionDirection::Emit,
                    payload: json!({"tool": "read_file"}),
                },
            },
            ApiTranscriptRecord::UiInput {
                seq: 6,
                at_ms: 50,
                event: RecordedUiInput::Key {
                    code: "char".to_string(),
                    char: Some('/'),
                    modifiers: Vec::new(),
                },
            },
            ApiTranscriptRecord::UiInput {
                seq: 7,
                at_ms: 60,
                event: RecordedUiInput::Resize { cols: 120, rows: 36 },
            },
            ApiTranscriptRecord::UiFrame {
                seq: 8,
                at_ms: 70,
                frame: RecordedFrame {
                    cols: 120,
                    rows: 36,
                    text_hash: "sha256:abc".to_string(),
                    text: Some("Slash commands".to_string()),
                    artifacts: vec![RecordedArtifactRef {
                        path: "frames/0001.txt".to_string(),
                        media_type: "text/plain".to_string(),
                        sha256: Some("abc".to_string()),
                        bytes: Some(14),
                    }],
                },
            },
        ];

        for record in records {
            let line = serde_json::to_string(&record).unwrap();
            let parsed: ApiTranscriptRecord = serde_json::from_str(&line).unwrap();
            assert_eq!(parsed, record);
            assert!(parsed.validate().is_ok());
        }
    }

    fn header() -> ApiTranscriptHeader {
        ApiTranscriptHeader {
            schema_version: SUPPORTED_SCHEMA_VERSION,
            created_at: OffsetDateTime::UNIX_EPOCH,
            roder_version: "dev".to_string(),
            cwd: "<redacted>".to_string(),
            terminal: RecordedTerminalSize {
                cols: 120,
                rows: 36,
            },
            features: vec!["tui".to_string()],
            metadata: Value::Null,
        }
    }
}

