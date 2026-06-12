use base64::Engine;
use roder_api::inference::ProviderAuthType;
use roder_api::speech::{
    SpeechAudio, SpeechProviderContext, SpeechProviderMetadata, SpeechSynthesisRequest,
    SpeechTranscriptionRequest,
};
use roder_protocol::{
    JsonRpcError, SpeechProviderDescriptor, SpeechProvidersListResult,
    SpeechSynthesisProviderDescriptor, SpeechSynthesisProvidersListResult, SpeechSynthesizeParams,
    SpeechSynthesizeResult, SpeechTranscribeParams, SpeechTranscribeResult,
};

use crate::server::{AppServer, internal_error};

impl AppServer {
    pub(crate) async fn handle_speech_providers_list(
        &self,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut providers = Vec::new();
        for transcriber in &self.runtime.registry().speech_transcribers {
            let id = transcriber.id();
            let metadata = transcriber.metadata();
            let (authenticated, auth_detail) = speech_auth_status(&metadata);
            let models = transcriber
                .list_models(SpeechProviderContext { provider_id: &id })
                .await
                .unwrap_or_default();
            providers.push(SpeechProviderDescriptor {
                id,
                name: metadata.name,
                description: metadata.description,
                auth_type: metadata.auth_type,
                auth_label: metadata.auth_label,
                authenticated,
                auth_detail,
                recommended: metadata.recommended,
                sort_order: metadata.sort_order,
                capabilities: transcriber.capabilities(),
                models,
            });
        }
        providers.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(serde_json::to_value(SpeechProvidersListResult { providers }).unwrap())
    }

    pub(crate) async fn handle_speech_synthesis_providers_list(
        &self,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut providers = Vec::new();
        for synthesizer in &self.runtime.registry().speech_synthesizers {
            let id = synthesizer.id();
            let metadata = synthesizer.metadata();
            let (authenticated, auth_detail) = speech_auth_status(&metadata);
            let models = synthesizer
                .list_models(SpeechProviderContext { provider_id: &id })
                .await
                .unwrap_or_default();
            providers.push(SpeechSynthesisProviderDescriptor {
                id,
                name: metadata.name,
                description: metadata.description,
                auth_type: metadata.auth_type,
                auth_label: metadata.auth_label,
                authenticated,
                auth_detail,
                recommended: metadata.recommended,
                sort_order: metadata.sort_order,
                capabilities: synthesizer.capabilities(),
                models,
            });
        }
        providers.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(serde_json::to_value(SpeechSynthesisProvidersListResult { providers }).unwrap())
    }

    pub(crate) async fn handle_speech_transcribe(
        &self,
        params: SpeechTranscribeParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let transcriber = match params.provider.as_deref() {
            Some(provider) => self
                .runtime
                .registry()
                .speech_transcriber(provider)
                .ok_or_else(|| speech_not_found(format!("unknown speech provider {provider:?}")))?,
            None => self
                .runtime
                .registry()
                .speech_transcribers
                .first()
                .cloned()
                .ok_or_else(|| speech_not_found("no speech providers are installed"))?,
        };
        let provider_id = transcriber.id();
        let ctx = SpeechProviderContext {
            provider_id: &provider_id,
        };
        let models = transcriber.list_models(ctx).await.map_err(internal_error)?;
        let model = params
            .model
            .or_else(|| models.first().map(|model| model.id.clone()))
            .ok_or_else(|| {
                speech_not_found(format!("speech provider {provider_id:?} has no models"))
            })?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(params.audio.bytes_base64.as_bytes())
            .map_err(|err| speech_invalid_params(format!("invalid audio.bytesBase64: {err}")))?;
        let result = transcriber
            .transcribe(
                SpeechProviderContext {
                    provider_id: &provider_id,
                },
                SpeechTranscriptionRequest {
                    model: model.clone(),
                    audio: SpeechAudio {
                        bytes,
                        mime_type: params.audio.mime_type,
                        filename: params.audio.filename,
                    },
                    language: params.language,
                    prompt: params.prompt,
                    diarization: params.diarization,
                    metadata: params.metadata,
                },
            )
            .await
            .map_err(internal_error)?;

        Ok(serde_json::to_value(SpeechTranscribeResult {
            provider: provider_id,
            model,
            text: result.text,
            language: result.language,
            duration_millis: result.duration_millis,
            segments: result.segments,
            provider_response_id: result.provider_response_id,
            metadata: result.metadata,
        })
        .unwrap())
    }

    pub(crate) async fn handle_speech_synthesize(
        &self,
        params: SpeechSynthesizeParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let synthesizer = match params.provider.as_deref() {
            Some(provider) => self
                .runtime
                .registry()
                .speech_synthesizer(provider)
                .ok_or_else(|| {
                    speech_not_found(format!("unknown speech synthesis provider {provider:?}"))
                })?,
            None => self
                .runtime
                .registry()
                .speech_synthesizers
                .first()
                .cloned()
                .ok_or_else(|| speech_not_found("no speech synthesis providers are installed"))?,
        };
        let provider_id = synthesizer.id();
        let ctx = SpeechProviderContext {
            provider_id: &provider_id,
        };
        let models = synthesizer.list_models(ctx).await.map_err(internal_error)?;
        let model = params
            .model
            .or_else(|| models.first().map(|model| model.id.clone()))
            .ok_or_else(|| {
                speech_not_found(format!(
                    "speech synthesis provider {provider_id:?} has no models"
                ))
            })?;
        let voice_sample = match params.voice_sample {
            Some(sample) => Some(SpeechAudio {
                bytes: base64::engine::general_purpose::STANDARD
                    .decode(sample.bytes_base64.as_bytes())
                    .map_err(|err| {
                        speech_invalid_params(format!("invalid voiceSample.bytesBase64: {err}"))
                    })?,
                mime_type: sample.mime_type,
                filename: sample.filename,
            }),
            None => None,
        };
        let result = synthesizer
            .synthesize(
                SpeechProviderContext {
                    provider_id: &provider_id,
                },
                SpeechSynthesisRequest {
                    model: model.clone(),
                    text: params.text,
                    voice: params.voice,
                    audio_format: params.audio_format,
                    prompt: params.prompt,
                    voice_sample,
                    metadata: params.metadata,
                },
            )
            .await
            .map_err(internal_error)?;

        Ok(serde_json::to_value(SpeechSynthesizeResult {
            provider: provider_id,
            model,
            audio: roder_protocol::SpeechAudioPayload {
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(result.audio.bytes),
                mime_type: result.audio.mime_type,
                filename: result.audio.filename,
            },
            duration_millis: result.duration_millis,
            provider_response_id: result.provider_response_id,
            metadata: result.metadata,
        })
        .unwrap())
    }
}

fn speech_auth_status(metadata: &SpeechProviderMetadata) -> (bool, Option<String>) {
    match metadata.auth_type {
        ProviderAuthType::None => (true, None),
        ProviderAuthType::ApiKey | ProviderAuthType::OAuth => (
            metadata.auth_configured.unwrap_or(false),
            metadata.auth_label.clone(),
        ),
    }
}

fn speech_invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {err}"),
        data: None,
    }
}

fn speech_not_found(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: message.into(),
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use roder_api::extension::{ExtensionRegistryBuilder, SpeechTranscriberId};
    use roder_api::speech::{
        SpeechCapabilities, SpeechModelDescriptor, SpeechSegment, SpeechTranscriber,
        SpeechTranscriptionResult,
    };
    use roder_core::{Runtime, RuntimeConfig};
    use roder_extension_host::{DefaultRegistryConfig, build_default_registry};
    use roder_protocol::{
        JsonRpcRequest, SpeechProvidersListResult, SpeechSynthesisProvidersListResult,
    };

    use super::*;

    /// Offline fake transcriber that "recognizes" the submitted bytes so the
    /// JSON-RPC success path can be tested without provider credentials.
    struct FakeSpeechTranscriber;

    #[async_trait::async_trait]
    impl SpeechTranscriber for FakeSpeechTranscriber {
        fn id(&self) -> SpeechTranscriberId {
            "fake-speech".to_string()
        }

        fn capabilities(&self) -> SpeechCapabilities {
            SpeechCapabilities {
                batch: true,
                ..SpeechCapabilities::default()
            }
        }

        fn metadata(&self) -> SpeechProviderMetadata {
            SpeechProviderMetadata::local("Fake Speech")
        }

        async fn list_models(
            &self,
            _ctx: SpeechProviderContext<'_>,
        ) -> anyhow::Result<Vec<SpeechModelDescriptor>> {
            Ok(vec![SpeechModelDescriptor {
                id: "fake-stt".to_string(),
                name: "Fake STT".to_string(),
                description: None,
                capabilities: SpeechCapabilities {
                    batch: true,
                    ..SpeechCapabilities::default()
                },
            }])
        }

        async fn transcribe(
            &self,
            _ctx: SpeechProviderContext<'_>,
            request: SpeechTranscriptionRequest,
        ) -> anyhow::Result<SpeechTranscriptionResult> {
            anyhow::ensure!(request.model == "fake-stt", "unexpected model");
            let decoded = String::from_utf8_lossy(&request.audio.bytes).into_owned();
            Ok(SpeechTranscriptionResult {
                text: format!("transcribed: {decoded}"),
                language: request.language,
                duration_millis: Some(400),
                segments: vec![SpeechSegment {
                    text: decoded,
                    start_millis: Some(0),
                    end_millis: Some(400),
                    speaker: None,
                    confidence: Some(0.99),
                }],
                provider_response_id: Some("fake-response-1".to_string()),
                metadata: serde_json::json!({}),
            })
        }
    }

    #[tokio::test]
    async fn speech_transcribe_succeeds_through_json_rpc_with_fake_provider() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(roder_core::fake_provider::FakeInferenceEngine));
        builder.speech_transcriber(Arc::new(FakeSpeechTranscriber));
        let registry = builder.build().unwrap();
        let runtime = Arc::new(Runtime::new(registry, RuntimeConfig::default()).unwrap());
        let server = AppServer::new(runtime);

        let audio_base64 = base64::engine::general_purpose::STANDARD.encode(b"hello roder speech");
        let response = server
            .handle_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(1)),
                method: "speech/transcribe".to_string(),
                params: Some(serde_json::json!({
                    "provider": "fake-speech",
                    "audio": {
                        "bytesBase64": audio_base64,
                        "mimeType": "audio/wav",
                        "filename": "clip.wav"
                    },
                    "language": "en"
                })),
            })
            .await;

        assert!(response.error.is_none(), "{:?}", response.error);
        let result: SpeechTranscribeResult =
            serde_json::from_value(response.result.unwrap()).unwrap();
        assert_eq!(result.provider, "fake-speech");
        assert_eq!(result.model, "fake-stt");
        assert_eq!(result.text, "transcribed: hello roder speech");
        assert_eq!(result.language.as_deref(), Some("en"));
        assert_eq!(result.duration_millis, Some(400));
        assert_eq!(result.segments.len(), 1);
        assert_eq!(result.segments[0].confidence, Some(0.99));
        assert_eq!(
            result.provider_response_id.as_deref(),
            Some("fake-response-1")
        );
    }

    #[tokio::test]
    async fn speech_transcribe_rejects_invalid_base64_audio() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(roder_core::fake_provider::FakeInferenceEngine));
        builder.speech_transcriber(Arc::new(FakeSpeechTranscriber));
        let registry = builder.build().unwrap();
        let runtime = Arc::new(Runtime::new(registry, RuntimeConfig::default()).unwrap());
        let server = AppServer::new(runtime);

        let response = server
            .handle_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(1)),
                method: "speech/transcribe".to_string(),
                params: Some(serde_json::json!({
                    "provider": "fake-speech",
                    "audio": {
                        "bytesBase64": "not base64 at all!!!",
                        "mimeType": "audio/wav"
                    }
                })),
            })
            .await;

        let error = response.error.expect("invalid base64 must fail");
        assert_eq!(error.code, -32602);
        assert!(error.message.contains("bytesBase64"), "{}", error.message);
    }

    #[tokio::test]
    async fn speech_providers_list_uses_default_registry_extensions() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
        let runtime = Arc::new(Runtime::new(registry, RuntimeConfig::default()).unwrap());
        let server = AppServer::new(runtime);

        let response = server
            .handle_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(1)),
                method: "speech/providers/list".to_string(),
                params: None,
            })
            .await;

        assert!(response.error.is_none(), "{:?}", response.error);
        let result: SpeechProvidersListResult =
            serde_json::from_value(response.result.unwrap()).unwrap();
        let provider_ids = result
            .providers
            .into_iter()
            .map(|provider| provider.id)
            .collect::<Vec<_>>();
        assert!(provider_ids.contains(&"openai-speech".to_string()));
        assert!(provider_ids.contains(&"google-speech".to_string()));
    }

    #[tokio::test]
    async fn speech_synthesis_providers_list_uses_default_registry_extensions() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
        let runtime = Arc::new(Runtime::new(registry, RuntimeConfig::default()).unwrap());
        let server = AppServer::new(runtime);

        let response = server
            .handle_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(1)),
                method: "speech/synthesis/providers/list".to_string(),
                params: None,
            })
            .await;

        assert!(response.error.is_none(), "{:?}", response.error);
        let result: SpeechSynthesisProvidersListResult =
            serde_json::from_value(response.result.unwrap()).unwrap();
        let provider_ids = result
            .providers
            .into_iter()
            .map(|provider| provider.id)
            .collect::<Vec<_>>();
        assert!(provider_ids.contains(&"xiaomi-mimo".to_string()));
        assert!(provider_ids.contains(&"xiaomi-mimo-token-plan".to_string()));
    }
}
