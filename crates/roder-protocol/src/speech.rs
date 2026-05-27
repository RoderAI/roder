use std::collections::BTreeMap;

use roder_api::inference::ProviderAuthType;
use roder_api::speech::{
    SpeechCapabilities, SpeechModelDescriptor, SpeechSegment, SpeechSynthesisCapabilities,
    SpeechSynthesisModelDescriptor,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechProviderDescriptor {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub auth_type: ProviderAuthType,
    pub auth_label: Option<String>,
    pub authenticated: bool,
    pub auth_detail: Option<String>,
    pub recommended: bool,
    pub sort_order: i32,
    pub capabilities: SpeechCapabilities,
    pub models: Vec<SpeechModelDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechProvidersListResult {
    pub providers: Vec<SpeechProviderDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSynthesisProviderDescriptor {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub auth_type: ProviderAuthType,
    pub auth_label: Option<String>,
    pub authenticated: bool,
    pub auth_detail: Option<String>,
    pub recommended: bool,
    pub sort_order: i32,
    pub capabilities: SpeechSynthesisCapabilities,
    pub models: Vec<SpeechSynthesisModelDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSynthesisProvidersListResult {
    pub providers: Vec<SpeechSynthesisProviderDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechAudioPayload {
    pub bytes_base64: String,
    pub mime_type: String,
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechTranscribeParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub audio: SpeechAudioPayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default)]
    pub diarization: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechTranscribeResult {
    pub provider: String,
    pub model: String,
    pub text: String,
    pub language: Option<String>,
    pub duration_millis: Option<u64>,
    pub segments: Vec<SpeechSegment>,
    pub provider_response_id: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSynthesizeParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_sample: Option<SpeechAudioPayload>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSynthesizeResult {
    pub provider: String,
    pub model: String,
    pub audio: SpeechAudioPayload,
    pub duration_millis: Option<u64>,
    pub provider_response_id: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcribe_params_use_camel_case_audio_bytes() {
        let params: SpeechTranscribeParams = serde_json::from_value(serde_json::json!({
            "provider": "openai-speech",
            "model": "gpt-4o-mini-transcribe",
            "audio": {
                "bytesBase64": "YXVkaW8=",
                "mimeType": "audio/wav",
                "filename": "clip.wav"
            },
            "language": "en",
            "diarization": false
        }))
        .unwrap();

        assert_eq!(params.audio.bytes_base64, "YXVkaW8=");
        assert_eq!(params.audio.mime_type, "audio/wav");
        assert_eq!(params.audio.filename.as_deref(), Some("clip.wav"));
    }

    #[test]
    fn synthesize_params_use_camel_case_audio_format() {
        let params: SpeechSynthesizeParams = serde_json::from_value(serde_json::json!({
            "provider": "xiaomi-mimo",
            "model": "mimo-v2.5-tts",
            "text": "hello",
            "audioFormat": "wav",
            "voiceSample": {
                "bytesBase64": "dm9pY2U=",
                "mimeType": "audio/wav",
                "filename": "voice.wav"
            }
        }))
        .unwrap();

        assert_eq!(params.audio_format.as_deref(), Some("wav"));
        assert_eq!(
            params.voice_sample.unwrap().bytes_base64.as_str(),
            "dm9pY2U="
        );
    }
}
