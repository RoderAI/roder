use base64::Engine;
use roder_api::extension::SpeechTranscriberId;
use roder_api::inference::ProviderAuthType;
use roder_api::speech::{
    SpeechCapabilities, SpeechModelDescriptor, SpeechProviderContext, SpeechProviderMetadata,
    SpeechSegment, SpeechTranscriber, SpeechTranscriptionRequest, SpeechTranscriptionResult,
};
use serde_json::{Value, json};

pub const GOOGLE_SPEECH_PROVIDER_ID: &str = "google-speech";
const DEFAULT_GOOGLE_SPEECH_ENDPOINT: &str = "https://speech.googleapis.com";
const DEFAULT_GOOGLE_SPEECH_LOCATION: &str = "global";

#[derive(Debug, Clone)]
pub struct GoogleSpeechConfig {
    pub access_token: Option<String>,
    pub project_id: Option<String>,
    pub location: String,
    pub endpoint: String,
}

impl Default for GoogleSpeechConfig {
    fn default() -> Self {
        Self {
            access_token: None,
            project_id: None,
            location: DEFAULT_GOOGLE_SPEECH_LOCATION.to_string(),
            endpoint: DEFAULT_GOOGLE_SPEECH_ENDPOINT.to_string(),
        }
    }
}

impl GoogleSpeechConfig {
    pub fn from_env() -> Self {
        Self {
            access_token: std::env::var("RODER_GOOGLE_SPEECH_ACCESS_TOKEN").ok(),
            project_id: std::env::var("RODER_GOOGLE_SPEECH_PROJECT")
                .ok()
                .or_else(|| std::env::var("GOOGLE_CLOUD_PROJECT").ok()),
            location: std::env::var("RODER_GOOGLE_SPEECH_LOCATION")
                .ok()
                .unwrap_or_else(|| DEFAULT_GOOGLE_SPEECH_LOCATION.to_string()),
            endpoint: std::env::var("RODER_GOOGLE_SPEECH_ENDPOINT")
                .ok()
                .unwrap_or_else(|| DEFAULT_GOOGLE_SPEECH_ENDPOINT.to_string()),
        }
    }
}

pub struct GoogleSpeechTranscriber {
    config: GoogleSpeechConfig,
    client: reqwest::Client,
}

impl GoogleSpeechTranscriber {
    pub fn new(config: GoogleSpeechConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl SpeechTranscriber for GoogleSpeechTranscriber {
    fn id(&self) -> SpeechTranscriberId {
        GOOGLE_SPEECH_PROVIDER_ID.to_string()
    }

    fn capabilities(&self) -> SpeechCapabilities {
        SpeechCapabilities {
            batch: true,
            streaming: false,
            diarization: true,
            timestamps: true,
            language_hints: true,
            prompt: false,
        }
    }

    fn metadata(&self) -> SpeechProviderMetadata {
        SpeechProviderMetadata {
            name: "Google Speech".to_string(),
            description: Some("Google Cloud Speech-to-Text v2 models".to_string()),
            auth_type: ProviderAuthType::OAuth,
            auth_label: Some("Google Cloud OAuth access token".to_string()),
            auth_configured: Some(
                self.config.access_token.is_some() && self.config.project_id.is_some(),
            ),
            recommended: false,
            sort_order: 20,
        }
    }

    async fn list_models(
        &self,
        _ctx: SpeechProviderContext<'_>,
    ) -> anyhow::Result<Vec<SpeechModelDescriptor>> {
        Ok(google_speech_models())
    }

    async fn transcribe(
        &self,
        _ctx: SpeechProviderContext<'_>,
        request: SpeechTranscriptionRequest,
    ) -> anyhow::Result<SpeechTranscriptionResult> {
        let Some(access_token) = self.config.access_token.as_deref() else {
            anyhow::bail!("Google speech transcription requires RODER_GOOGLE_SPEECH_ACCESS_TOKEN")
        };
        let Some(project_id) = self.config.project_id.as_deref() else {
            anyhow::bail!("Google speech transcription requires RODER_GOOGLE_SPEECH_PROJECT")
        };
        let body = google_recognize_body(&request);
        let url = format!(
            "{}/v2/projects/{}/locations/{}/recognizers/_:recognize",
            self.config.endpoint.trim_end_matches('/'),
            project_id,
            self.config.location
        );
        let response = self
            .client
            .post(url)
            .bearer_auth(access_token)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let value = response.json::<Value>().await?;
        if !status.is_success() {
            anyhow::bail!("Google speech transcription failed with {status}: {value}");
        }
        Ok(transcription_result_from_google(value))
    }
}

pub fn google_speech_models() -> Vec<SpeechModelDescriptor> {
    vec![
        model("chirp_3", "Chirp 3", "Google latest multilingual ASR model"),
        model("chirp", "Chirp", "Google Universal Speech Model"),
        model(
            "latest_long",
            "Latest Long",
            "Google latest long-form model",
        ),
        model(
            "latest_short",
            "Latest Short",
            "Google latest short-command model",
        ),
        model("long", "Long", "Google long-form Speech-to-Text model"),
        model("short", "Short", "Google short-form Speech-to-Text model"),
    ]
}

fn model(id: &str, name: &str, description: &str) -> SpeechModelDescriptor {
    SpeechModelDescriptor {
        id: id.to_string(),
        name: name.to_string(),
        description: Some(description.to_string()),
        capabilities: SpeechCapabilities {
            batch: true,
            streaming: false,
            diarization: id != "chirp",
            timestamps: true,
            language_hints: true,
            prompt: false,
        },
    }
}

fn google_recognize_body(request: &SpeechTranscriptionRequest) -> Value {
    let language = request
        .language
        .clone()
        .unwrap_or_else(|| "en-US".to_string());
    let mut features = json!({
        "enableAutomaticPunctuation": true,
        "enableWordTimeOffsets": request.diarization,
    });
    if request.diarization {
        features["diarizationConfig"] = json!({
            "minSpeakerCount": 2,
            "maxSpeakerCount": 6,
        });
    }
    json!({
        "config": {
            "autoDecodingConfig": {},
            "languageCodes": [language],
            "model": request.model,
            "features": features,
        },
        "content": base64::engine::general_purpose::STANDARD.encode(&request.audio.bytes),
    })
}

fn transcription_result_from_google(value: Value) -> SpeechTranscriptionResult {
    let alternatives = value
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|result| result.get("alternatives")?.as_array()?.first());
    let mut text_parts = Vec::new();
    let mut segments = Vec::new();
    for alternative in alternatives {
        if let Some(transcript) = alternative.get("transcript").and_then(Value::as_str) {
            text_parts.push(transcript.to_string());
            segments.push(SpeechSegment {
                text: transcript.to_string(),
                start_millis: None,
                end_millis: None,
                speaker: None,
                confidence: alternative
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .map(|value| value as f32),
            });
        }
        if let Some(words) = alternative.get("words").and_then(Value::as_array) {
            for word in words {
                if let Some(segment) = word_segment_from_google(word) {
                    segments.push(segment);
                }
            }
        }
    }
    SpeechTranscriptionResult {
        text: text_parts.join(" "),
        language: value
            .pointer("/results/0/languageCode")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        duration_millis: None,
        segments,
        provider_response_id: None,
        metadata: value,
    }
}

fn word_segment_from_google(value: &Value) -> Option<SpeechSegment> {
    let text = value
        .get("word")
        .or_else(|| value.get("transcript"))
        .and_then(Value::as_str)?
        .to_string();
    Some(SpeechSegment {
        text,
        start_millis: value
            .get("startOffset")
            .and_then(Value::as_str)
            .and_then(google_duration_to_millis),
        end_millis: value
            .get("endOffset")
            .and_then(Value::as_str)
            .and_then(google_duration_to_millis),
        speaker: value
            .get("speakerLabel")
            .or_else(|| value.get("speakerTag"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        confidence: value
            .get("confidence")
            .and_then(Value::as_f64)
            .map(|value| value as f32),
    })
}

fn google_duration_to_millis(value: &str) -> Option<u64> {
    let seconds = value.strip_suffix('s')?.parse::<f64>().ok()?;
    Some((seconds * 1000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use roder_api::speech::{SpeechAudio, SpeechProviderContext, SpeechTranscriptionRequest};

    use super::*;

    #[tokio::test]
    async fn lists_google_speech_models() {
        let provider = GoogleSpeechTranscriber::new(GoogleSpeechConfig::default());

        let models = provider
            .list_models(SpeechProviderContext {
                provider_id: GOOGLE_SPEECH_PROVIDER_ID,
            })
            .await
            .unwrap();

        assert!(models.iter().any(|model| model.id == "chirp_3"));
        assert!(models.iter().any(|model| model.id == "latest_long"));
        assert!(models.iter().any(|model| model.id == "latest_short"));
    }

    #[tokio::test]
    async fn missing_credentials_fail_before_network_request() {
        let provider = GoogleSpeechTranscriber::new(GoogleSpeechConfig::default());

        let err = provider
            .transcribe(
                SpeechProviderContext {
                    provider_id: GOOGLE_SPEECH_PROVIDER_ID,
                },
                SpeechTranscriptionRequest {
                    model: "chirp_3".to_string(),
                    audio: SpeechAudio {
                        bytes: b"audio".to_vec(),
                        mime_type: "audio/wav".to_string(),
                        filename: Some("clip.wav".to_string()),
                    },
                    language: Some("en-US".to_string()),
                    prompt: None,
                    diarization: false,
                    metadata: Default::default(),
                },
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("RODER_GOOGLE_SPEECH_ACCESS_TOKEN"));
    }

    #[test]
    fn maps_google_recognize_request() {
        let body = google_recognize_body(&SpeechTranscriptionRequest {
            model: "chirp_3".to_string(),
            audio: SpeechAudio {
                bytes: b"audio".to_vec(),
                mime_type: "audio/wav".to_string(),
                filename: None,
            },
            language: Some("en-US".to_string()),
            prompt: None,
            diarization: true,
            metadata: Default::default(),
        });

        assert_eq!(body["config"]["model"], "chirp_3");
        assert_eq!(body["config"]["languageCodes"][0], "en-US");
        assert_eq!(body["content"], "YXVkaW8=");
        assert_eq!(body["config"]["features"]["enableWordTimeOffsets"], true);
    }
}
