use base64::Engine;
use roder_api::extension::SpeechSynthesizerId;
use roder_api::inference::ProviderAuthType;
use roder_api::speech::{
    SpeechAudio, SpeechProviderContext, SpeechProviderMetadata, SpeechSynthesisCapabilities,
    SpeechSynthesisModelDescriptor, SpeechSynthesisRequest, SpeechSynthesisResult,
    SpeechSynthesizer,
};
use roder_ext_openai_chat_completions::chat::provider_status_error_with_body;
use serde_json::{Value, json};

use crate::provider::{
    XiaomiMimoConfig, XiaomiMimoInferenceEngine, XiaomiMimoProviderSpec, validate_token_plan_auth,
};

pub struct XiaomiMimoSpeechSynthesizer {
    engine: XiaomiMimoInferenceEngine,
    spec: XiaomiMimoProviderSpec,
}

impl XiaomiMimoSpeechSynthesizer {
    pub fn new(config: XiaomiMimoConfig, spec: XiaomiMimoProviderSpec) -> Self {
        Self {
            engine: XiaomiMimoInferenceEngine::new(config, spec),
            spec,
        }
    }

    fn api_key(&self) -> Option<String> {
        self.engine.api_key()
    }

    fn base_url(&self) -> Option<String> {
        self.engine.base_url()
    }
}

#[async_trait::async_trait]
impl SpeechSynthesizer for XiaomiMimoSpeechSynthesizer {
    fn id(&self) -> SpeechSynthesizerId {
        self.spec.provider_id.to_string()
    }

    fn capabilities(&self) -> SpeechSynthesisCapabilities {
        SpeechSynthesisCapabilities {
            batch: true,
            streaming: false,
            builtin_voices: true,
            voice_design: true,
            voice_clone: true,
            prompt: true,
        }
    }

    fn metadata(&self) -> SpeechProviderMetadata {
        let auth_configured = match (self.api_key(), self.base_url()) {
            (Some(api_key), Some(base_url)) if self.spec.token_plan => {
                validate_token_plan_auth(&api_key, &base_url).is_ok()
            }
            (Some(_), Some(_)) => true,
            _ => false,
        };
        SpeechProviderMetadata {
            name: format!("{} Speech Synthesis", self.spec.name),
            description: Some(
                "Xiaomi MiMo TTS over OpenAI-compatible Chat Completions".to_string(),
            ),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some(self.spec.api_key_env.to_string()),
            auth_configured: Some(auth_configured),
            recommended: true,
            sort_order: self.spec.sort_order,
        }
    }

    async fn list_models(
        &self,
        _ctx: SpeechProviderContext<'_>,
    ) -> anyhow::Result<Vec<SpeechSynthesisModelDescriptor>> {
        Ok(tts_models())
    }

    async fn synthesize(
        &self,
        _ctx: SpeechProviderContext<'_>,
        request: SpeechSynthesisRequest,
    ) -> anyhow::Result<SpeechSynthesisResult> {
        let Some(api_key) = self.api_key() else {
            anyhow::bail!(
                "{} API key is missing; set {} or configure it from the provider menu",
                self.spec.name,
                self.spec.api_key_env
            )
        };
        let Some(base_url) = self.base_url() else {
            anyhow::bail!(
                "{} base URL is missing; set {} from the Token Plan subscription page",
                self.spec.name,
                self.spec.base_url_env
            )
        };
        if self.spec.token_plan {
            validate_token_plan_auth(&api_key, &base_url)?;
        }
        let body = speech_request_body(&request)?;
        let response = reqwest::Client::new()
            .post(format!(
                "{}/chat/completions",
                base_url.trim_end_matches('/')
            ))
            .header("api-key", &api_key)
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(provider_status_error_with_body(
                self.spec.name,
                "speech synthesis",
                status,
                &body,
                Some(&api_key),
            ));
        }
        let value: Value = response.json().await?;
        let audio_base64 = value
            .pointer("/choices/0/message/audio/data")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                anyhow::anyhow!("Xiaomi MiMo speech response did not include audio data")
            })?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(audio_base64)?;
        Ok(SpeechSynthesisResult {
            audio: SpeechAudio {
                bytes,
                mime_type: audio_mime_type(request.audio_format.as_deref().unwrap_or("wav")),
                filename: None,
            },
            duration_millis: None,
            provider_response_id: value.get("id").and_then(Value::as_str).map(str::to_string),
            metadata: value,
        })
    }
}

fn tts_models() -> Vec<SpeechSynthesisModelDescriptor> {
    vec![
        speech_model(
            "mimo-v2.5-tts",
            "MiMo V2.5 TTS",
            "Built-in high-quality Xiaomi MiMo voices.",
            SpeechSynthesisCapabilities {
                builtin_voices: true,
                prompt: true,
                ..SpeechSynthesisCapabilities::default()
            },
        ),
        speech_model(
            "mimo-v2.5-tts-voiceclone",
            "MiMo V2.5 TTS VoiceClone",
            "Xiaomi MiMo TTS with voice cloning from a WAV or MP3 sample.",
            SpeechSynthesisCapabilities {
                voice_clone: true,
                prompt: true,
                ..SpeechSynthesisCapabilities::default()
            },
        ),
        speech_model(
            "mimo-v2.5-tts-voicedesign",
            "MiMo V2.5 TTS VoiceDesign",
            "Xiaomi MiMo TTS with voice design from a text description.",
            SpeechSynthesisCapabilities {
                voice_design: true,
                prompt: true,
                ..SpeechSynthesisCapabilities::default()
            },
        ),
        speech_model(
            "mimo-v2-tts",
            "MiMo V2 TTS",
            "Xiaomi MiMo V2 built-in voice speech synthesis.",
            SpeechSynthesisCapabilities {
                builtin_voices: true,
                prompt: true,
                ..SpeechSynthesisCapabilities::default()
            },
        ),
    ]
}

fn speech_model(
    id: &str,
    name: &str,
    description: &str,
    capabilities: SpeechSynthesisCapabilities,
) -> SpeechSynthesisModelDescriptor {
    SpeechSynthesisModelDescriptor {
        id: id.to_string(),
        name: name.to_string(),
        description: Some(description.to_string()),
        capabilities: SpeechSynthesisCapabilities {
            batch: true,
            streaming: false,
            ..capabilities
        },
    }
}

fn speech_request_body(request: &SpeechSynthesisRequest) -> anyhow::Result<Value> {
    if request.text.trim().is_empty() {
        anyhow::bail!("speech synthesis text is required");
    }
    let audio_format = request.audio_format.as_deref().unwrap_or("wav");
    let mut messages = Vec::new();
    if let Some(prompt) = request
        .prompt
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        messages.push(json!({ "role": "user", "content": prompt }));
    } else if request.model == "mimo-v2.5-tts-voicedesign" {
        anyhow::bail!("mimo-v2.5-tts-voicedesign requires a voice design prompt");
    }
    messages.push(json!({ "role": "assistant", "content": request.text }));

    let mut audio = json!({ "format": audio_format });
    if let Some(sample) = &request.voice_sample {
        audio["voice"] = json!(voice_sample_data_url(sample)?);
    } else if let Some(voice) = request
        .voice
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        audio["voice"] = json!(voice);
    } else if request.model == "mimo-v2.5-tts-voiceclone" {
        anyhow::bail!("mimo-v2.5-tts-voiceclone requires voice or voice_sample");
    } else if request.model != "mimo-v2.5-tts-voicedesign" {
        audio["voice"] = json!("mimo_default");
    }
    if request.model == "mimo-v2.5-tts-voicedesign" {
        audio["optimize_text_preview"] = json!(true);
    }

    Ok(json!({
        "model": request.model,
        "messages": messages,
        "audio": audio,
        "stream": false,
    }))
}

fn voice_sample_data_url(audio: &SpeechAudio) -> anyhow::Result<String> {
    match audio.mime_type.as_str() {
        "audio/mpeg" | "audio/mp3" | "audio/wav" => {}
        other => anyhow::bail!("unsupported Xiaomi MiMo voice sample MIME type {other:?}"),
    }
    Ok(format!(
        "data:{};base64,{}",
        audio.mime_type,
        base64::engine::general_purpose::STANDARD.encode(&audio.bytes)
    ))
}

fn audio_mime_type(format: &str) -> String {
    match format {
        "pcm16" => "audio/pcm",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use roder_api::catalog::{PROVIDER_XIAOMI_MIMO, PROVIDER_XIAOMI_MIMO_TOKEN_PLAN};

    use super::*;

    #[test]
    fn tts_models_are_speech_synthesis_only() {
        let models = tts_models();
        assert_eq!(models.len(), 4);
        assert!(models.iter().any(|model| model.id == "mimo-v2.5-tts"));
        assert!(
            models
                .iter()
                .any(|model| model.id == "mimo-v2.5-tts-voiceclone")
        );
    }

    #[test]
    fn speech_body_places_target_text_in_assistant_message() {
        let body = speech_request_body(&SpeechSynthesisRequest {
            model: "mimo-v2.5-tts".to_string(),
            text: "hello".to_string(),
            voice: Some("Chloe".to_string()),
            audio_format: Some("wav".to_string()),
            prompt: Some("warm voice".to_string()),
            voice_sample: None,
            metadata: Default::default(),
        })
        .unwrap();

        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][1]["role"], "assistant");
        assert_eq!(body["audio"]["voice"], "Chloe");
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn token_plan_synthesizer_uses_token_plan_provider_id() {
        let synthesizer = XiaomiMimoSpeechSynthesizer::new(
            XiaomiMimoConfig::default(),
            XiaomiMimoProviderSpec::token_plan(),
        );

        assert_eq!(synthesizer.id(), PROVIDER_XIAOMI_MIMO_TOKEN_PLAN);
        assert!(synthesizer.metadata().name.contains("Token Plan"));
    }

    #[test]
    fn pay_as_you_go_synthesizer_uses_mimo_provider_id() {
        let synthesizer = XiaomiMimoSpeechSynthesizer::new(
            XiaomiMimoConfig::default(),
            XiaomiMimoProviderSpec::pay_as_you_go(),
        );

        assert_eq!(synthesizer.id(), PROVIDER_XIAOMI_MIMO);
    }
}
