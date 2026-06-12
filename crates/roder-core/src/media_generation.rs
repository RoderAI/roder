//! Provider-neutral image generation service.
//!
//! Resolves the requested or configured [`MediaGeneratorProvider`], enforces
//! request limits, resolves input artifacts into inline images, persists every
//! generated output through [`MediaArtifactStore`], and exposes the canonical
//! `media_generate_image` tool used by the runtime tool pipeline.

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use roder_api::media::{
    FAKE_MEDIA_PROVIDER_ID, GeneratedImage, ImageGenerationAction, ImageGenerationBatch,
    ImageModelDescriptor, MediaDimensions, MediaGenerationMetadata, MediaGenerationOutput,
    MediaGenerationRequest, MediaGenerationResponse, MediaGeneratorProvider, MediaImageInput,
    MediaKind, MediaProviderDescriptor,
};
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolExecutor, ToolResult, ToolSpec};
use serde_json::json;

use crate::media_artifacts::{GeneratedMediaSpec, MediaArtifactStore, default_media_artifact_dir};

/// Runtime configuration for image generation, mapped from
/// `[media.image_generation]` user config by the host.
#[derive(Debug, Clone)]
pub struct RuntimeMediaGenerationConfig {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub max_outputs: u32,
    pub max_input_images: u32,
    pub artifacts_dir: Option<PathBuf>,
    pub max_read_bytes: Option<u64>,
}

impl Default for RuntimeMediaGenerationConfig {
    fn default() -> Self {
        Self {
            default_provider: None,
            default_model: None,
            max_outputs: 4,
            max_input_images: 16,
            artifacts_dir: None,
            max_read_bytes: None,
        }
    }
}

pub struct MediaGenerationService {
    providers: Vec<Arc<dyn MediaGeneratorProvider>>,
    config: RuntimeMediaGenerationConfig,
}

impl MediaGenerationService {
    pub fn new(
        mut providers: Vec<Arc<dyn MediaGeneratorProvider>>,
        config: RuntimeMediaGenerationConfig,
    ) -> Self {
        if !providers
            .iter()
            .any(|provider| provider.provider_id() == FAKE_MEDIA_PROVIDER_ID)
        {
            providers.push(Arc::new(FakeImageProvider));
        }
        Self { providers, config }
    }

    pub fn config(&self) -> &RuntimeMediaGenerationConfig {
        &self.config
    }

    pub fn provider_descriptors(&self) -> Vec<MediaProviderDescriptor> {
        self.providers
            .iter()
            .map(|provider| provider.descriptor())
            .collect()
    }

    pub fn default_provider_id(&self) -> String {
        self.config
            .default_provider
            .clone()
            .unwrap_or_else(|| FAKE_MEDIA_PROVIDER_ID.to_string())
    }

    pub fn store(&self) -> anyhow::Result<MediaArtifactStore> {
        let root = self
            .config
            .artifacts_dir
            .clone()
            .or_else(|| std::env::var_os("RODER_MEDIA_ARTIFACT_DIR").map(PathBuf::from))
            .map(Ok)
            .unwrap_or_else(default_media_artifact_dir)?;
        let mut store = MediaArtifactStore::new(root);
        if let Some(max_read_bytes) = self.config.max_read_bytes {
            store = store.with_max_read_bytes(max_read_bytes);
        }
        Ok(store)
    }

    pub async fn generate_image(
        &self,
        mut request: MediaGenerationRequest,
    ) -> anyhow::Result<MediaGenerationResponse> {
        if request.prompt.trim().is_empty() {
            anyhow::bail!("image generation requires a non-empty prompt");
        }
        let count = request.count.unwrap_or(1);
        if count == 0 || count > self.config.max_outputs {
            anyhow::bail!(
                "requested {count} outputs; configured limit is 1..={}",
                self.config.max_outputs
            );
        }
        let input_count = request.input_artifacts.len() + request.input_images.len();
        if input_count > self.config.max_input_images as usize {
            anyhow::bail!(
                "requested {input_count} input images; configured limit is {}",
                self.config.max_input_images
            );
        }
        if request.action == Some(ImageGenerationAction::Edit) && input_count == 0 {
            anyhow::bail!(
                "the edit action requires at least one input artifact or inline input image"
            );
        }

        let provider_id = request
            .provider
            .clone()
            .unwrap_or_else(|| self.default_provider_id());
        let provider = self
            .providers
            .iter()
            .find(|provider| provider.provider_id() == provider_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "image provider {provider_id:?} is not available; installed providers: {}",
                    self.provider_ids().join(", ")
                )
            })?;

        if request.model.is_none() && provider_id == self.default_provider_id() {
            request.model = self.config.default_model.clone();
        }

        let store = self.store()?;
        self.resolve_input_artifacts(&store, &mut request)?;
        request.provider = Some(provider_id);

        let prompt = request.prompt.clone();
        let batch = provider.generate_image(request).await?;
        if batch.images.is_empty() {
            if batch.output_errors.is_empty() {
                anyhow::bail!(
                    "image provider {} returned no images",
                    provider.provider_id()
                );
            }
            anyhow::bail!(
                "image provider {} generated no images: {}",
                provider.provider_id(),
                batch.output_errors.join("; ")
            );
        }
        self.persist_batch(&store, &prompt, batch)
    }

    fn provider_ids(&self) -> Vec<String> {
        self.providers
            .iter()
            .map(|provider| provider.provider_id().to_string())
            .collect()
    }

    /// Reads referenced artifacts from the store and inlines them so
    /// providers never touch artifact storage directly.
    fn resolve_input_artifacts(
        &self,
        store: &MediaArtifactStore,
        request: &mut MediaGenerationRequest,
    ) -> anyhow::Result<()> {
        for artifact_id in std::mem::take(&mut request.input_artifacts) {
            let (artifact, bytes) = store.read(&artifact_id, None).map_err(|error| {
                anyhow::anyhow!("could not read input artifact {artifact_id}: {error}")
            })?;
            if artifact.kind != MediaKind::Image {
                anyhow::bail!("input artifact {artifact_id} is not an image");
            }
            request.input_images.push(MediaImageInput {
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                mime_type: artifact.mime_type,
            });
        }
        Ok(())
    }

    fn persist_batch(
        &self,
        store: &MediaArtifactStore,
        prompt: &str,
        batch: ImageGenerationBatch,
    ) -> anyhow::Result<MediaGenerationResponse> {
        let mut outputs = Vec::with_capacity(batch.images.len());
        let mut response_revised_prompt = None;
        let mut response_watermark = None;
        let mut response_safety = None;
        for image in &batch.images {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&image.bytes_base64)
                .map_err(|error| {
                    anyhow::anyhow!(
                        "image provider {} returned invalid base64 output: {error}",
                        batch.provider
                    )
                })?;
            let generation = MediaGenerationMetadata {
                provider: batch.provider.clone(),
                model: Some(batch.model.clone()),
                revised_prompt: image.revised_prompt.clone(),
                watermark: image.watermark.clone(),
                safety: image.safety.clone(),
                provider_response_id: batch.provider_response_id.clone(),
            };
            let (artifact, preview) = store.write_generated(&GeneratedMediaSpec {
                prompt,
                kind: MediaKind::Image,
                mime_type: &image.mime_type,
                provider: &batch.provider,
                bytes: &bytes,
                dimensions: image.dimensions.clone(),
                duration_millis: None,
                generation: Some(generation),
            })?;
            response_revised_prompt = response_revised_prompt.or(image.revised_prompt.clone());
            response_watermark = response_watermark.or(image.watermark.clone());
            response_safety = response_safety.or(image.safety.clone());
            outputs.push(MediaGenerationOutput {
                artifact,
                preview,
                revised_prompt: image.revised_prompt.clone(),
            });
        }
        Ok(MediaGenerationResponse {
            provider: batch.provider,
            model: Some(batch.model),
            outputs,
            revised_prompt: response_revised_prompt,
            provider_response_id: batch.provider_response_id,
            usage: batch.usage,
            watermark: response_watermark,
            safety: response_safety,
            output_errors: batch.output_errors,
        })
    }
}

/// Deterministic offline image generator used when no live provider is
/// configured and as the reference implementation in tests.
pub struct FakeImageProvider;

/// 1x1 transparent PNG used by deterministic fake image generation.
pub const FAKE_IMAGE_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

#[async_trait::async_trait]
impl MediaGeneratorProvider for FakeImageProvider {
    fn provider_id(&self) -> &str {
        FAKE_MEDIA_PROVIDER_ID
    }

    fn descriptor(&self) -> MediaProviderDescriptor {
        MediaProviderDescriptor {
            id: FAKE_MEDIA_PROVIDER_ID.to_string(),
            display_name: "Fake Media (offline)".to_string(),
            supports_images: true,
            supports_videos: false,
            configured: true,
            default_model: Some("fake-image".to_string()),
            image_models: vec![ImageModelDescriptor {
                id: "fake-image".to_string(),
                display_name: "Fake Image".to_string(),
                provider: FAKE_MEDIA_PROVIDER_ID.to_string(),
                is_default: true,
                legacy: false,
                supports_edit: true,
                supports_multiple_outputs: true,
                supported_aspect_ratios: Vec::new(),
                supported_sizes: Vec::new(),
                supported_image_sizes: Vec::new(),
                supports_transparent_background: false,
                supports_partial_images: false,
            }],
        }
    }

    async fn generate_image(
        &self,
        request: MediaGenerationRequest,
    ) -> anyhow::Result<ImageGenerationBatch> {
        let count = request.count.unwrap_or(1).max(1);
        let model = request.model.unwrap_or_else(|| "fake-image".to_string());
        let images = (0..count)
            .map(|_| GeneratedImage {
                bytes_base64: FAKE_IMAGE_PNG_BASE64.to_string(),
                mime_type: "image/png".to_string(),
                dimensions: Some(MediaDimensions {
                    width: 1,
                    height: 1,
                }),
                revised_prompt: None,
                watermark: None,
                safety: None,
            })
            .collect();
        Ok(ImageGenerationBatch {
            provider: FAKE_MEDIA_PROVIDER_ID.to_string(),
            model,
            images,
            provider_response_id: None,
            usage: None,
            output_errors: Vec::new(),
        })
    }
}

/// Canonical `media_generate_image` tool. Registered by the runtime, replacing
/// the offline-only fake tool contributed by `roder-tools`.
pub struct MediaGenerateImageTool {
    service: Arc<MediaGenerationService>,
}

impl MediaGenerateImageTool {
    pub fn new(service: Arc<MediaGenerationService>) -> Self {
        Self { service }
    }
}

#[async_trait::async_trait]
impl ToolExecutor for MediaGenerateImageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "media_generate_image".to_string(),
            description:
                "Generates one or more images with the configured image provider and stores them as Roder media artifacts."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Text description of the image(s) to generate." },
                    "provider": { "type": "string", "description": "Image provider id (e.g. openai, google, fake). Defaults to the configured provider." },
                    "model": { "type": "string", "description": "Image model id, e.g. gpt-image-2 or gemini-3.1-flash-image." },
                    "action": { "type": "string", "enum": ["auto", "generate", "edit"] },
                    "inputArtifacts": { "type": "array", "items": { "type": "string" }, "description": "Roder media artifact ids used as reference/edit inputs." },
                    "count": { "type": "integer", "minimum": 1 },
                    "aspectRatio": { "type": "string", "description": "Aspect ratio such as 16:9 (Gemini models)." },
                    "size": { "type": "string", "description": "Pixel size such as 1536x1024 (OpenAI models)." },
                    "imageSize": { "type": "string", "description": "Resolution tier such as 1K, 2K, or 4K (Gemini models)." },
                    "quality": { "type": "string" },
                    "outputFormat": { "type": "string", "enum": ["png", "jpeg", "webp"] },
                    "background": { "type": "string", "enum": ["auto", "transparent", "opaque"] },
                    "outputCompression": { "type": "integer", "minimum": 0, "maximum": 100 },
                    "moderation": { "type": "string" }
                },
                "required": ["prompt"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let request: MediaGenerationRequest = serde_json::from_value(call.arguments.clone())
            .map_err(|error| anyhow::anyhow!("invalid media_generate_image arguments: {error}"))?;
        let response = self.service.generate_image(request).await?;
        let artifact_ids: Vec<&str> = response
            .outputs
            .iter()
            .map(|output| output.artifact.id.as_str())
            .collect();
        let artifacts: Vec<_> = response
            .outputs
            .iter()
            .map(|output| output.artifact.clone())
            .collect();
        let previews: Vec<_> = response
            .outputs
            .iter()
            .map(|output| output.preview.clone())
            .collect();
        let text = format!(
            "generated {} image artifact(s) with {}{}: {}",
            response.outputs.len(),
            response.provider,
            response
                .model
                .as_deref()
                .map(|model| format!("/{model}"))
                .unwrap_or_default(),
            artifact_ids.join(", ")
        );
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text,
            data: json!({
                "mediaArtifacts": artifacts,
                "mediaPreviews": previews,
                "mediaGeneration": response,
            }),
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::policy_mode::PolicyMode;

    fn temp_config() -> (RuntimeMediaGenerationConfig, PathBuf) {
        let dir = std::env::temp_dir().join(format!("roder-media-gen-{}", uuid::Uuid::new_v4()));
        let config = RuntimeMediaGenerationConfig {
            artifacts_dir: Some(dir.clone()),
            ..RuntimeMediaGenerationConfig::default()
        };
        (config, dir)
    }

    fn request(prompt: &str) -> MediaGenerationRequest {
        MediaGenerationRequest {
            prompt: prompt.to_string(),
            ..MediaGenerationRequest::default()
        }
    }

    #[tokio::test]
    async fn fake_image_generation_persists_artifacts_in_store() {
        let (config, dir) = temp_config();
        let service = MediaGenerationService::new(Vec::new(), config);

        let response = service
            .generate_image(MediaGenerationRequest {
                count: Some(2),
                ..request("two tiny images")
            })
            .await
            .unwrap();

        assert_eq!(response.provider, FAKE_MEDIA_PROVIDER_ID);
        assert_eq!(response.outputs.len(), 2);
        for output in &response.outputs {
            assert!(
                output
                    .artifact
                    .store_path
                    .starts_with(&*dir.display().to_string())
            );
            assert!(output.artifact.roder_owned);
            assert_eq!(
                output
                    .artifact
                    .generation
                    .as_ref()
                    .map(|generation| generation.provider.as_str()),
                Some(FAKE_MEDIA_PROVIDER_ID)
            );
        }
        assert_eq!(service.store().unwrap().list().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn generation_limits_and_missing_provider_fail_with_clear_errors() {
        let (config, _dir) = temp_config();
        let service = MediaGenerationService::new(Vec::new(), config);

        let empty_prompt = service.generate_image(request(" ")).await.unwrap_err();
        assert!(empty_prompt.to_string().contains("non-empty prompt"));

        let too_many = service
            .generate_image(MediaGenerationRequest {
                count: Some(5),
                ..request("too many")
            })
            .await
            .unwrap_err();
        assert!(too_many.to_string().contains("configured limit is 1..=4"));

        let missing_provider = service
            .generate_image(MediaGenerationRequest {
                provider: Some("missing".to_string()),
                ..request("nope")
            })
            .await
            .unwrap_err();
        assert!(
            missing_provider
                .to_string()
                .contains("image provider \"missing\" is not available")
        );

        let edit_without_inputs = service
            .generate_image(MediaGenerationRequest {
                action: Some(ImageGenerationAction::Edit),
                ..request("edit nothing")
            })
            .await
            .unwrap_err();
        assert!(
            edit_without_inputs
                .to_string()
                .contains("edit action requires at least one input")
        );
    }

    #[tokio::test]
    async fn input_artifacts_are_resolved_into_inline_images_for_the_provider() {
        struct CapturingProvider {
            captured: std::sync::Mutex<Option<MediaGenerationRequest>>,
        }

        #[async_trait::async_trait]
        impl MediaGeneratorProvider for CapturingProvider {
            fn provider_id(&self) -> &str {
                "capture"
            }

            fn descriptor(&self) -> MediaProviderDescriptor {
                MediaProviderDescriptor {
                    id: "capture".to_string(),
                    display_name: "Capture".to_string(),
                    supports_images: true,
                    configured: true,
                    ..MediaProviderDescriptor::default()
                }
            }

            async fn generate_image(
                &self,
                request: MediaGenerationRequest,
            ) -> anyhow::Result<ImageGenerationBatch> {
                *self.captured.lock().unwrap() = Some(request);
                Ok(ImageGenerationBatch {
                    provider: "capture".to_string(),
                    model: "capture-image".to_string(),
                    images: vec![GeneratedImage {
                        bytes_base64: FAKE_IMAGE_PNG_BASE64.to_string(),
                        mime_type: "image/png".to_string(),
                        dimensions: None,
                        revised_prompt: Some("revised".to_string()),
                        watermark: Some("synthid".to_string()),
                        safety: None,
                    }],
                    provider_response_id: Some("resp-1".to_string()),
                    usage: None,
                    output_errors: Vec::new(),
                })
            }
        }

        let provider = Arc::new(CapturingProvider {
            captured: std::sync::Mutex::new(None),
        });
        let (config, _dir) = temp_config();
        let service = MediaGenerationService::new(vec![provider.clone()], config);

        let (seed_artifact, _) = service
            .store()
            .unwrap()
            .write_generated(&GeneratedMediaSpec {
                prompt: "seed",
                kind: MediaKind::Image,
                mime_type: "image/png",
                provider: "fake",
                bytes: b"abc",
                dimensions: None,
                duration_millis: None,
                generation: None,
            })
            .unwrap();

        let response = service
            .generate_image(MediaGenerationRequest {
                provider: Some("capture".to_string()),
                action: Some(ImageGenerationAction::Edit),
                input_artifacts: vec![seed_artifact.id.clone()],
                ..request("edit the seed")
            })
            .await
            .unwrap();

        let captured = provider.captured.lock().unwrap().clone().unwrap();
        assert!(captured.input_artifacts.is_empty());
        assert_eq!(captured.input_images.len(), 1);
        assert_eq!(captured.input_images[0].mime_type, "image/png");
        assert_eq!(
            captured.input_images[0].bytes_base64,
            base64::engine::general_purpose::STANDARD.encode(b"abc")
        );

        assert_eq!(response.revised_prompt.as_deref(), Some("revised"));
        assert_eq!(response.watermark.as_deref(), Some("synthid"));
        assert_eq!(response.provider_response_id.as_deref(), Some("resp-1"));
        let generation = response.outputs[0].artifact.generation.clone().unwrap();
        assert_eq!(generation.watermark.as_deref(), Some("synthid"));
        assert_eq!(generation.provider_response_id.as_deref(), Some("resp-1"));
    }

    #[tokio::test]
    async fn media_generate_image_tool_returns_canonical_payload() {
        let (config, _dir) = temp_config();
        let service = Arc::new(MediaGenerationService::new(Vec::new(), config));
        let tool = MediaGenerateImageTool::new(service);

        let result = tool
            .execute(
                ToolExecutionContext::new("thread", "turn", PolicyMode::Default),
                ToolCall {
                    id: "call".to_string(),
                    name: "media_generate_image".to_string(),
                    arguments: json!({ "prompt": "tiny" }),
                    raw_arguments: "{}".to_string(),
                    thread_id: "thread".to_string(),
                    turn_id: "turn".to_string(),
                },
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["mediaArtifacts"][0]["kind"], "image");
        assert_eq!(result.data["mediaPreviews"][0]["strategy"], "thumbnail");
        assert_eq!(result.data["mediaGeneration"]["provider"], "fake");
        assert!(result.text.contains("generated 1 image artifact(s)"));
    }
}
