use roder_api::extension::SpeechTranscriberId;
use roder_api::inference::ProviderAuthType;
use roder_api::speech::{
    SpeechCapabilities, SpeechModelDescriptor, SpeechProviderContext, SpeechProviderMetadata,
    SpeechSegment, SpeechTranscriber, SpeechTranscriptionRequest, SpeechTranscriptionResult,
};
use serde_json::Value;

pub const OPENAI_SPEECH_PROVIDER_ID: &str = "openai-speech";
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Debug, Clone)]
pub struct OpenAiSpeechConfig {
    pub api_key: Option<String>,
    pub base_url: String,
}

impl OpenAiSpeechConfig {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key,
            base_url: std::env::var("OPENAI_BASE_URL")
                .ok()
                .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string()),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

pub struct OpenAiSpeechTranscriber {
    config: OpenAiSpeechConfig,
    client: reqwest::Client,
}

impl OpenAiSpeechTranscriber {
    pub fn new(config: OpenAiSpeechConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl SpeechTranscriber for OpenAiSpeechTranscriber {
    fn id(&self) -> SpeechTranscriberId {
        OPENAI_SPEECH_PROVIDER_ID.to_string()
    }

    fn capabilities(&self) -> SpeechCapabilities {
        SpeechCapabilities {
            batch: true,
            streaming: true,
            diarization: true,
            timestamps: true,
            language_hints: true,
            prompt: true,
        }
    }

    fn metadata(&self) -> SpeechProviderMetadata {
        SpeechProviderMetadata {
            name: "OpenAI Speech".to_string(),
            description: Some("OpenAI speech-to-text models".to_string()),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some("OPENAI_API_KEY".to_string()),
            auth_configured: Some(self.config.api_key.is_some()),
            recommended: true,
            sort_order: 10,
        }
    }

    async fn list_models(
        &self,
        _ctx: SpeechProviderContext<'_>,
    ) -> anyhow::Result<Vec<SpeechModelDescriptor>> {
        Ok(openai_speech_models())
    }

    async fn transcribe(
        &self,
        _ctx: SpeechProviderContext<'_>,
        request: SpeechTranscriptionRequest,
    ) -> anyhow::Result<SpeechTranscriptionResult> {
        validate_transcription_request(&request)?;
        let Some(api_key) = self.config.api_key.as_deref() else {
            anyhow::bail!("OpenAI speech transcription requires OPENAI_API_KEY")
        };
        let url = format!(
            "{}/audio/transcriptions",
            self.config.base_url.trim_end_matches('/')
        );
        let filename = request
            .audio
            .filename
            .clone()
            .unwrap_or_else(|| "audio".to_string());
        let part = reqwest::multipart::Part::bytes(request.audio.bytes)
            .file_name(filename)
            .mime_str(&request.audio.mime_type)?;
        let mut form = reqwest::multipart::Form::new()
            .text("model", request.model.clone())
            .text("response_format", "json")
            .part("file", part);
        if let Some(language) = request.language.as_deref() {
            form = form.text("language", language.to_string());
        }
        if let Some(prompt) = request.prompt.as_deref() {
            form = form.text("prompt", prompt.to_string());
        }
        if request.diarization || request.model.contains("diarize") {
            form = form.text("chunking_strategy", "auto");
        }

        let response = self
            .client
            .post(url)
            .bearer_auth(api_key)
            .multipart(form)
            .send()
            .await?;
        let status = response.status();
        let value = response.json::<Value>().await?;
        if !status.is_success() {
            anyhow::bail!("OpenAI speech transcription failed with {status}: {value}");
        }
        Ok(transcription_result_from_openai(value))
    }
}

pub fn openai_speech_models() -> Vec<SpeechModelDescriptor> {
    vec![
        SpeechModelDescriptor {
            id: "gpt-4o-transcribe".to_string(),
            name: "GPT-4o Transcribe".to_string(),
            description: Some("OpenAI GPT-4o speech-to-text model".to_string()),
            capabilities: SpeechCapabilities {
                batch: true,
                streaming: false,
                diarization: false,
                timestamps: false,
                language_hints: true,
                prompt: true,
            },
        },
        SpeechModelDescriptor {
            id: "gpt-4o-mini-transcribe".to_string(),
            name: "GPT-4o Mini Transcribe".to_string(),
            description: Some("Smaller OpenAI GPT-4o speech-to-text model".to_string()),
            capabilities: SpeechCapabilities {
                batch: true,
                streaming: false,
                diarization: false,
                timestamps: false,
                language_hints: true,
                prompt: true,
            },
        },
        SpeechModelDescriptor {
            id: "gpt-4o-transcribe-diarize".to_string(),
            name: "GPT-4o Transcribe Diarize".to_string(),
            description: Some("OpenAI transcription model with speaker labels".to_string()),
            capabilities: SpeechCapabilities {
                batch: true,
                streaming: false,
                diarization: true,
                timestamps: true,
                language_hints: true,
                prompt: false,
            },
        },
        SpeechModelDescriptor {
            id: "whisper-1".to_string(),
            name: "Whisper".to_string(),
            description: Some("OpenAI Whisper transcription model".to_string()),
            capabilities: SpeechCapabilities {
                batch: true,
                streaming: false,
                diarization: false,
                timestamps: true,
                language_hints: true,
                prompt: true,
            },
        },
        SpeechModelDescriptor {
            id: "gpt-realtime-whisper".to_string(),
            name: "GPT Realtime Whisper".to_string(),
            description: Some("OpenAI low-latency streaming transcription model".to_string()),
            capabilities: SpeechCapabilities {
                batch: false,
                streaming: true,
                diarization: false,
                timestamps: false,
                language_hints: true,
                prompt: true,
            },
        },
    ]
}

fn transcription_result_from_openai(value: Value) -> SpeechTranscriptionResult {
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    SpeechTranscriptionResult {
        text,
        language: value
            .get("language")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        duration_millis: value.get("duration").and_then(number_seconds_to_millis),
        segments: value
            .get("segments")
            .and_then(Value::as_array)
            .map(|segments| segments.iter().map(segment_from_openai).collect())
            .unwrap_or_default(),
        provider_response_id: value
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        metadata: value,
    }
}

fn segment_from_openai(value: &Value) -> SpeechSegment {
    SpeechSegment {
        text: value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        start_millis: value.get("start").and_then(number_seconds_to_millis),
        end_millis: value.get("end").and_then(number_seconds_to_millis),
        speaker: value
            .get("speaker")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        confidence: value
            .get("confidence")
            .and_then(Value::as_f64)
            .map(|value| value as f32),
    }
}

fn number_seconds_to_millis(value: &Value) -> Option<u64> {
    value
        .as_f64()
        .map(|seconds| (seconds * 1000.0).round() as u64)
}

fn validate_transcription_request(request: &SpeechTranscriptionRequest) -> anyhow::Result<()> {
    let Some(model) = openai_speech_models()
        .into_iter()
        .find(|model| model.id == request.model)
    else {
        return Ok(());
    };
    if !model.capabilities.batch {
        anyhow::bail!(
            "OpenAI model {} is a streaming transcription model and is not supported by speech/transcribe",
            request.model
        );
    }
    if request.diarization && !model.capabilities.diarization {
        anyhow::bail!(
            "OpenAI model {} does not support diarization on speech/transcribe",
            request.model
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use roder_api::speech::{SpeechAudio, SpeechProviderContext, SpeechTranscriptionRequest};

    use super::*;

    #[tokio::test]
    async fn lists_current_openai_transcription_models() {
        let provider = OpenAiSpeechTranscriber::new(OpenAiSpeechConfig::new(None));

        let models = provider
            .list_models(SpeechProviderContext {
                provider_id: OPENAI_SPEECH_PROVIDER_ID,
            })
            .await
            .unwrap();

        assert!(models.iter().any(|model| model.id == "gpt-4o-transcribe"));
        assert!(
            models
                .iter()
                .any(|model| model.id == "gpt-4o-mini-transcribe")
        );
        assert!(
            models
                .iter()
                .any(|model| model.id == "gpt-4o-transcribe-diarize")
        );
        assert!(models.iter().any(|model| model.id == "whisper-1"));
        assert!(
            models
                .iter()
                .any(|model| model.id == "gpt-realtime-whisper")
        );
    }

    #[tokio::test]
    async fn missing_api_key_fails_before_network_request() {
        let provider = OpenAiSpeechTranscriber::new(OpenAiSpeechConfig::new(None));

        let err = provider
            .transcribe(
                SpeechProviderContext {
                    provider_id: OPENAI_SPEECH_PROVIDER_ID,
                },
                SpeechTranscriptionRequest {
                    model: "gpt-4o-mini-transcribe".to_string(),
                    audio: SpeechAudio {
                        bytes: b"audio".to_vec(),
                        mime_type: "audio/wav".to_string(),
                        filename: Some("clip.wav".to_string()),
                    },
                    language: Some("en".to_string()),
                    prompt: None,
                    diarization: false,
                    metadata: Default::default(),
                },
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("OPENAI_API_KEY"));
    }

    #[tokio::test]
    async fn realtime_transcription_model_is_not_sent_to_batch_endpoint() {
        let provider =
            OpenAiSpeechTranscriber::new(OpenAiSpeechConfig::new(Some("secret".to_string())));

        let err = provider
            .transcribe(
                SpeechProviderContext {
                    provider_id: OPENAI_SPEECH_PROVIDER_ID,
                },
                SpeechTranscriptionRequest {
                    model: "gpt-realtime-whisper".to_string(),
                    audio: SpeechAudio {
                        bytes: b"audio".to_vec(),
                        mime_type: "audio/wav".to_string(),
                        filename: Some("clip.wav".to_string()),
                    },
                    language: Some("en".to_string()),
                    prompt: None,
                    diarization: false,
                    metadata: Default::default(),
                },
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("streaming transcription model"));
    }
}
