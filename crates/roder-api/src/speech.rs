use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::extension::{SpeechSynthesizerId, SpeechTranscriberId};
use crate::inference::ProviderAuthType;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeechCapabilities {
    pub batch: bool,
    pub streaming: bool,
    pub diarization: bool,
    pub timestamps: bool,
    pub language_hints: bool,
    pub prompt: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSynthesisCapabilities {
    pub batch: bool,
    pub streaming: bool,
    pub builtin_voices: bool,
    pub voice_design: bool,
    pub voice_clone: bool,
    pub prompt: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpeechProviderMetadata {
    pub name: String,
    pub description: Option<String>,
    pub auth_type: ProviderAuthType,
    pub auth_label: Option<String>,
    pub auth_configured: Option<bool>,
    pub recommended: bool,
    pub sort_order: i32,
}

impl SpeechProviderMetadata {
    pub fn local(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            auth_type: ProviderAuthType::None,
            auth_label: None,
            auth_configured: Some(true),
            recommended: false,
            sort_order: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpeechModelDescriptor {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub capabilities: SpeechCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpeechSynthesisModelDescriptor {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub capabilities: SpeechSynthesisCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpeechAudio {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeechTranscriptionRequest {
    pub model: String,
    pub audio: SpeechAudio,
    pub language: Option<String>,
    pub prompt: Option<String>,
    pub diarization: bool,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeechSegment {
    pub text: String,
    pub start_millis: Option<u64>,
    pub end_millis: Option<u64>,
    pub speaker: Option<String>,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeechTranscriptionResult {
    pub text: String,
    pub language: Option<String>,
    pub duration_millis: Option<u64>,
    pub segments: Vec<SpeechSegment>,
    pub provider_response_id: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeechSynthesisRequest {
    pub model: String,
    pub text: String,
    pub voice: Option<String>,
    pub audio_format: Option<String>,
    pub prompt: Option<String>,
    pub voice_sample: Option<SpeechAudio>,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeechSynthesisResult {
    pub audio: SpeechAudio,
    pub duration_millis: Option<u64>,
    pub provider_response_id: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Copy)]
pub struct SpeechProviderContext<'a> {
    pub provider_id: &'a str,
}

#[async_trait::async_trait]
pub trait SpeechTranscriber: Send + Sync + 'static {
    fn id(&self) -> SpeechTranscriberId;

    fn capabilities(&self) -> SpeechCapabilities;

    fn metadata(&self) -> SpeechProviderMetadata;

    async fn list_models(
        &self,
        ctx: SpeechProviderContext<'_>,
    ) -> anyhow::Result<Vec<SpeechModelDescriptor>>;

    async fn transcribe(
        &self,
        ctx: SpeechProviderContext<'_>,
        request: SpeechTranscriptionRequest,
    ) -> anyhow::Result<SpeechTranscriptionResult>;
}

#[async_trait::async_trait]
pub trait SpeechSynthesizer: Send + Sync + 'static {
    fn id(&self) -> SpeechSynthesizerId;

    fn capabilities(&self) -> SpeechSynthesisCapabilities;

    fn metadata(&self) -> SpeechProviderMetadata;

    async fn list_models(
        &self,
        ctx: SpeechProviderContext<'_>,
    ) -> anyhow::Result<Vec<SpeechSynthesisModelDescriptor>>;

    async fn synthesize(
        &self,
        ctx: SpeechProviderContext<'_>,
        request: SpeechSynthesisRequest,
    ) -> anyhow::Result<SpeechSynthesisResult>;
}
