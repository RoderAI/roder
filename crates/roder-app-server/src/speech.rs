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

    use roder_core::{Runtime, RuntimeConfig};
    use roder_extension_host::{DefaultRegistryConfig, build_default_registry};
    use roder_protocol::{
        JsonRpcRequest, SpeechProvidersListResult, SpeechSynthesisProvidersListResult,
    };

    use super::*;

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
