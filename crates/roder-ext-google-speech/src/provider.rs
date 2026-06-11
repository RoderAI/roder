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
    pub api_key: Option<String>,
    pub project_id: Option<String>,
    pub location: String,
    pub endpoint: String,
}

impl Default for GoogleSpeechConfig {
    fn default() -> Self {
        Self {
            access_token: None,
            api_key: None,
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
            api_key: google_speech_api_key_from_env(),
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
    adc: crate::adc::AdcTokenSource,
}

impl GoogleSpeechTranscriber {
    pub fn new(config: GoogleSpeechConfig) -> Self {
        Self::with_adc(config, crate::adc::AdcTokenSource::from_env())
    }

    /// Constructor with an explicit ADC source (tests, custom hosts).
    pub fn with_adc(config: GoogleSpeechConfig, adc: crate::adc::AdcTokenSource) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            adc,
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
            auth_type: if self.config.access_token.is_some() {
                ProviderAuthType::OAuth
            } else {
                ProviderAuthType::ApiKey
            },
            auth_label: Some("Google Cloud OAuth token or API key".to_string()),
            auth_configured: Some(
                self.config.api_key.is_some()
                    || (self.config.access_token.is_some() && self.config.project_id.is_some()),
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
        let auth = self.google_speech_auth().await?;
        let request = match auth {
            GoogleSpeechAuth::AccessToken(token) => {
                let token = token.as_str();
                let Some(project_id) = self.config.project_id.as_deref() else {
                    anyhow::bail!(
                        "Google speech transcription with OAuth requires RODER_GOOGLE_SPEECH_PROJECT or GOOGLE_CLOUD_PROJECT; set RODER_GOOGLE_SPEECH_API_KEY, GEMINI_API_KEY, GEMINI_API_TOKEN, or GOOGLE_API_KEY to use the projectless API-key fallback"
                    );
                };
                self.client
                    .post(google_recognize_v2_url(&self.config, project_id))
                    .bearer_auth(token)
                    .json(&google_recognize_v2_body(&request))
            }
            GoogleSpeechAuth::ApiKey(ref key) => {
                let (url, body) = if let Some(project_id) = self.config.project_id.as_deref() {
                    (
                        google_recognize_v2_url(&self.config, project_id),
                        google_recognize_v2_body(&request),
                    )
                } else {
                    (
                        google_recognize_v1_url(&self.config),
                        google_recognize_v1_body(&request),
                    )
                };
                let mut url = reqwest::Url::parse(&url)?;
                url.query_pairs_mut().append_pair("key", key);
                self.client.post(url).json(&body)
            }
        };
        let response = request.send().await?;
        let status = response.status();
        let value = response.json::<Value>().await?;
        if !status.is_success() {
            anyhow::bail!("Google speech transcription failed with {status}: {value}");
        }
        Ok(transcription_result_from_google(value))
    }
}

impl GoogleSpeechTranscriber {
    /**
     * Auth resolution order: explicit access token, then API key, then
     * Application Default Credentials (authorized-user ADC JSON refresh or
     * the gcloud CLI; see `crate::adc`).
     */
    async fn google_speech_auth(&self) -> anyhow::Result<GoogleSpeechAuth> {
        if let Some(access_token) = self.config.access_token.as_deref() {
            return Ok(GoogleSpeechAuth::AccessToken(access_token.to_string()));
        }
        if let Some(api_key) = self.config.api_key.as_deref() {
            return Ok(GoogleSpeechAuth::ApiKey(api_key.to_string()));
        }
        match self.adc.access_token().await {
            Ok(token) => Ok(GoogleSpeechAuth::AccessToken(token)),
            Err(error) => anyhow::bail!(
                "Google speech transcription requires RODER_GOOGLE_SPEECH_ACCESS_TOKEN,                  RODER_GOOGLE_SPEECH_API_KEY, GEMINI_API_KEY, GEMINI_API_TOKEN, GOOGLE_API_KEY,                  or Application Default Credentials ({error})"
            ),
        }
    }
}

enum GoogleSpeechAuth {
    AccessToken(String),
    ApiKey(String),
}

fn google_speech_api_key_from_env() -> Option<String> {
    [
        "RODER_GOOGLE_SPEECH_API_KEY",
        "GEMINI_API_TOKEN",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
    ]
    .into_iter()
    .find_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
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

fn google_recognize_v2_url(config: &GoogleSpeechConfig, project_id: &str) -> String {
    format!(
        "{}/v2/projects/{}/locations/{}/recognizers/_:recognize",
        config.endpoint.trim_end_matches('/'),
        project_id,
        config.location
    )
}

fn google_recognize_v1_url(config: &GoogleSpeechConfig) -> String {
    format!(
        "{}/v1/speech:recognize",
        config.endpoint.trim_end_matches('/')
    )
}

fn google_recognize_v2_body(request: &SpeechTranscriptionRequest) -> Value {
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

fn google_recognize_v1_body(request: &SpeechTranscriptionRequest) -> Value {
    let language = request
        .language
        .clone()
        .unwrap_or_else(|| "en-US".to_string());
    let mut config = json!({
        "languageCode": language,
        "model": google_recognize_v1_model(&request.model),
        "enableAutomaticPunctuation": true,
        "enableWordTimeOffsets": request.diarization,
    });
    if request.diarization {
        config["diarizationConfig"] = json!({
            "enableSpeakerDiarization": true,
            "minSpeakerCount": 2,
            "maxSpeakerCount": 6,
        });
    }
    json!({
        "config": config,
        "audio": {
            "content": base64::engine::general_purpose::STANDARD.encode(&request.audio.bytes),
        },
    })
}

fn google_recognize_v1_model(model: &str) -> &str {
    match model {
        "latest_short" | "short" => "latest_short",
        "latest_long" | "long" | "chirp" | "chirp_3" => "latest_long",
        other => other,
    }
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
        // Explicitly unavailable ADC source: the developer machine may have
        // real gcloud/ADC credentials and tests must never touch them.
        let provider = GoogleSpeechTranscriber::with_adc(
            GoogleSpeechConfig::default(),
            crate::adc::AdcTokenSource::new(
                None,
                "http://127.0.0.1:1/token",
                "/nonexistent/roder-test-gcloud",
            ),
        );

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

        assert!(err.to_string().contains("RODER_GOOGLE_SPEECH_API_KEY"));
    }

    #[tokio::test]
    async fn api_key_counts_as_configured_auth() {
        let provider = GoogleSpeechTranscriber::new(GoogleSpeechConfig {
            api_key: Some("key".to_string()),
            ..GoogleSpeechConfig::default()
        });
        let metadata = provider.metadata();

        assert_eq!(metadata.auth_type, ProviderAuthType::ApiKey);
        assert_eq!(metadata.auth_configured, Some(true));
        assert!(matches!(
            provider.google_speech_auth().await.unwrap(),
            GoogleSpeechAuth::ApiKey(ref key) if key == "key"
        ));
    }

    #[test]
    fn oauth_requires_project_to_count_as_configured_auth() {
        let provider = GoogleSpeechTranscriber::new(GoogleSpeechConfig {
            access_token: Some("token".to_string()),
            ..GoogleSpeechConfig::default()
        });
        let metadata = provider.metadata();

        assert_eq!(metadata.auth_type, ProviderAuthType::OAuth);
        assert_eq!(metadata.auth_configured, Some(false));
    }

    #[test]
    fn maps_google_recognize_v2_request() {
        let body = google_recognize_v2_body(&SpeechTranscriptionRequest {
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

    #[test]
    fn maps_google_recognize_v1_projectless_request() {
        let body = google_recognize_v1_body(&SpeechTranscriptionRequest {
            model: "chirp".to_string(),
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

        assert_eq!(body["config"]["model"], "latest_long");
        assert_eq!(body["config"]["languageCode"], "en-US");
        assert_eq!(body["audio"]["content"], "YXVkaW8=");
        assert_eq!(body["config"]["enableWordTimeOffsets"], true);
        assert_eq!(
            body["config"]["diarizationConfig"]["enableSpeakerDiarization"],
            true
        );
    }
}
