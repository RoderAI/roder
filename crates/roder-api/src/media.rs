use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub type MediaArtifactId = String;

/// Provider id reserved for the deterministic offline image generator.
pub const FAKE_MEDIA_PROVIDER_ID: &str = "fake";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MediaPreviewStrategy {
    InlineImage,
    Thumbnail,
    MetadataOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaDimensions {
    pub width: u32,
    pub height: u32,
}

/// Provider-reported metadata about how an artifact was generated. Persisted
/// alongside the artifact so safety/watermark provenance survives restarts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaGenerationMetadata {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
    /// Watermark scheme applied by the provider, e.g. `synthid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_response_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaArtifact {
    pub id: MediaArtifactId,
    pub kind: MediaKind,
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<MediaDimensions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_millis: Option<u64>,
    pub byte_size: u64,
    pub provider: String,
    pub prompt_hash: String,
    pub store_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<MediaGenerationMetadata>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(default)]
    pub roder_owned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaPreview {
    pub artifact_id: MediaArtifactId,
    pub strategy: MediaPreviewStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_path: Option<String>,
    pub fallback_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaAttachment {
    pub artifact_id: MediaArtifactId,
    pub mime_type: String,
    pub data_url: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImageGenerationAction {
    Auto,
    Generate,
    Edit,
}

/// Inline reference/edit image input passed to an image provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaImageInput {
    pub bytes_base64: String,
    pub mime_type: String,
}

/// Canonical provider-neutral media generation request. All option fields are
/// optional so legacy `{ "prompt": ... }` tool arguments keep decoding.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaGenerationRequest {
    #[serde(default)]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<ImageGenerationAction>,
    /// Roder artifact ids resolved into [`Self::input_images`] before the
    /// provider call.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_artifacts: Vec<MediaArtifactId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_images: Vec<MediaImageInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    /// Pixel size such as `1536x1024` (OpenAI Image API style).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    /// Resolution tier such as `1K`, `2K`, or `4K` (Gemini image config style).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// 0-100 compression for lossy output formats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_compression: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moderation: Option<String>,
    /// Requested partial-image preview count where the provider supports
    /// streaming; providers that do not support it must reject or ignore it
    /// explicitly rather than silently stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_images: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    /// Bounded, documented pass-through settings for one provider. Values are
    /// redacted from transcripts like all other request fields and must not
    /// change safety or storage semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaGenerationUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_image_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaGenerationOutput {
    pub artifact: MediaArtifact,
    pub preview: MediaPreview,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaGenerationResponse {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub outputs: Vec<MediaGenerationOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<MediaGenerationUsage>,
    /// Watermark scheme applied to every output, e.g. `synthid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety: Option<String>,
    /// Provider-reported errors for individual requested outputs that were
    /// not generated, while the rest of the batch succeeded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_errors: Vec<String>,
}

impl MediaGenerationResponse {
    pub fn primary_artifact(&self) -> Option<&MediaArtifact> {
        self.outputs.first().map(|output| &output.artifact)
    }
}

/// One raw provider output, before Roder persists it as an artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImage {
    pub bytes_base64: String,
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<MediaDimensions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety: Option<String>,
}

/// Provider result for one image generation call, prior to artifact storage.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerationBatch {
    pub provider: String,
    pub model: String,
    pub images: Vec<GeneratedImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<MediaGenerationUsage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_errors: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImageModelDescriptor {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    #[serde(default)]
    pub is_default: bool,
    /// Compatibility/legacy model kept for callers pinned to older ids.
    #[serde(default)]
    pub legacy: bool,
    #[serde(default)]
    pub supports_edit: bool,
    #[serde(default)]
    pub supports_multiple_outputs: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_aspect_ratios: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_sizes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_image_sizes: Vec<String>,
    #[serde(default)]
    pub supports_transparent_background: bool,
    #[serde(default)]
    pub supports_partial_images: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaProviderDescriptor {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub supports_images: bool,
    #[serde(default)]
    pub supports_videos: bool,
    /// Whether the provider has the credentials it needs to serve requests.
    #[serde(default)]
    pub configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_models: Vec<ImageModelDescriptor>,
}

/// Extension-host service for media (image/video) generation providers.
/// Providers return raw bytes; the core runtime persists artifacts, applies
/// policy, and emits events.
#[async_trait::async_trait]
pub trait MediaGeneratorProvider: Send + Sync + 'static {
    fn provider_id(&self) -> &str;

    fn descriptor(&self) -> MediaProviderDescriptor;

    async fn generate_image(
        &self,
        _request: MediaGenerationRequest,
    ) -> anyhow::Result<ImageGenerationBatch> {
        anyhow::bail!(
            "image generation is not supported by provider {}",
            self.provider_id()
        )
    }
}

pub fn data_url(mime_type: &str, bytes_base64: &str) -> String {
    format!("data:{mime_type};base64,{bytes_base64}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_artifact() -> MediaArtifact {
        MediaArtifact {
            id: "media-image-1".to_string(),
            kind: MediaKind::Image,
            mime_type: "image/png".to_string(),
            dimensions: Some(MediaDimensions {
                width: 1,
                height: 1,
            }),
            duration_millis: None,
            byte_size: 67,
            provider: "fake".to_string(),
            prompt_hash: "hash".to_string(),
            store_path: "/tmp/image.png".to_string(),
            thumbnail_path: Some("/tmp/image.thumb.png".to_string()),
            generation: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            roder_owned: true,
        }
    }

    #[test]
    fn image_and_video_artifacts_serialize_as_camel_case_metadata() {
        let image = image_artifact();
        let video = MediaArtifact {
            kind: MediaKind::Video,
            mime_type: "video/mp4".to_string(),
            duration_millis: Some(1_000),
            ..image.clone()
        };

        let value = serde_json::to_value(&image).unwrap();
        assert_eq!(value["mimeType"], "image/png");
        assert_eq!(value["dimensions"]["width"], 1);
        assert_eq!(value["thumbnailPath"], "/tmp/image.thumb.png");
        assert!(value.get("generation").is_none());
        assert_eq!(serde_json::to_value(video).unwrap()["durationMillis"], 1000);
    }

    #[test]
    fn minimum_text_to_image_request_decodes_from_legacy_arguments() {
        let request: MediaGenerationRequest =
            serde_json::from_value(serde_json::json!({ "prompt": "tiny" })).unwrap();
        assert_eq!(request.prompt, "tiny");
        assert!(request.provider.is_none());
        assert!(request.model.is_none());
        assert!(request.input_artifacts.is_empty());
        assert!(request.provider_options.is_none());

        let legacy: MediaGenerationRequest = serde_json::from_value(serde_json::json!({
            "prompt": "tiny",
            "model": "gpt-image-2",
            "outputPath": "/tmp/out.png"
        }))
        .unwrap();
        assert_eq!(legacy.model.as_deref(), Some("gpt-image-2"));
        assert_eq!(legacy.output_path.as_deref(), Some("/tmp/out.png"));
    }

    #[test]
    fn image_edit_request_serializes_canonical_camel_case_fields() {
        let request = MediaGenerationRequest {
            prompt: "Make this screenshot look like a clean launch graphic".to_string(),
            provider: Some("openai".to_string()),
            model: Some("gpt-image-2".to_string()),
            action: Some(ImageGenerationAction::Edit),
            input_artifacts: vec!["media-image-123".to_string()],
            size: Some("1536x1024".to_string()),
            output_format: Some("png".to_string()),
            ..MediaGenerationRequest::default()
        };

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["provider"], "openai");
        assert_eq!(value["action"], "edit");
        assert_eq!(value["inputArtifacts"][0], "media-image-123");
        assert_eq!(value["size"], "1536x1024");
        assert_eq!(value["outputFormat"], "png");
        assert!(value.get("inputImages").is_none());
    }

    #[test]
    fn google_style_request_serializes_aspect_ratio_and_image_size() {
        let request = MediaGenerationRequest {
            prompt: "A polished product hero image".to_string(),
            provider: Some("google".to_string()),
            model: Some("gemini-3-pro-image".to_string()),
            aspect_ratio: Some("16:9".to_string()),
            image_size: Some("2K".to_string()),
            ..MediaGenerationRequest::default()
        };

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["aspectRatio"], "16:9");
        assert_eq!(value["imageSize"], "2K");
    }

    #[test]
    fn multi_output_response_round_trips_with_usage_and_metadata() {
        let artifact = image_artifact();
        let preview = MediaPreview {
            artifact_id: artifact.id.clone(),
            strategy: MediaPreviewStrategy::Thumbnail,
            thumbnail_path: None,
            fallback_label: "fake image/png".to_string(),
            warning: None,
        };
        let response = MediaGenerationResponse {
            provider: "openai".to_string(),
            model: Some("gpt-image-2".to_string()),
            outputs: vec![
                MediaGenerationOutput {
                    artifact: artifact.clone(),
                    preview: preview.clone(),
                    revised_prompt: Some("a tiny test image".to_string()),
                },
                MediaGenerationOutput {
                    artifact,
                    preview,
                    revised_prompt: None,
                },
            ],
            revised_prompt: Some("a tiny test image".to_string()),
            provider_response_id: Some("resp_123".to_string()),
            usage: Some(MediaGenerationUsage {
                input_tokens: Some(12),
                input_image_tokens: None,
                output_tokens: Some(4_160),
                total_tokens: Some(4_172),
            }),
            watermark: None,
            safety: None,
            output_errors: vec!["third output was rejected by moderation".to_string()],
        };

        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["outputs"].as_array().unwrap().len(), 2);
        assert_eq!(value["outputs"][0]["revisedPrompt"], "a tiny test image");
        assert_eq!(value["providerResponseId"], "resp_123");
        assert_eq!(value["usage"]["totalTokens"], 4_172);
        assert_eq!(
            value["outputErrors"][0],
            "third output was rejected by moderation"
        );
        let round_trip: MediaGenerationResponse = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip, response);
    }

    #[test]
    fn partial_stream_preference_and_provider_options_round_trip() {
        let request: MediaGenerationRequest = serde_json::from_value(serde_json::json!({
            "prompt": "stream me",
            "provider": "openai",
            "partialImages": 2,
            "providerOptions": { "user": "roder-tests" }
        }))
        .unwrap();
        assert_eq!(request.partial_images, Some(2));
        assert_eq!(
            request
                .provider_options
                .as_ref()
                .and_then(|options| options.get("user"))
                .and_then(|value| value.as_str()),
            Some("roder-tests")
        );
    }

    #[test]
    fn google_generation_metadata_persists_synthid_watermark() {
        let mut artifact = image_artifact();
        artifact.generation = Some(MediaGenerationMetadata {
            provider: "google".to_string(),
            model: Some("gemini-3.1-flash-image".to_string()),
            revised_prompt: None,
            watermark: Some("synthid".to_string()),
            safety: None,
            provider_response_id: None,
        });

        let value = serde_json::to_value(&artifact).unwrap();
        assert_eq!(value["generation"]["provider"], "google");
        assert_eq!(value["generation"]["watermark"], "synthid");
        let round_trip: MediaArtifact = serde_json::from_value(value).unwrap();
        assert_eq!(
            round_trip.generation.unwrap().watermark.as_deref(),
            Some("synthid")
        );
    }

    #[test]
    fn openai_batch_metadata_round_trips() {
        let batch = ImageGenerationBatch {
            provider: "openai".to_string(),
            model: "gpt-image-2".to_string(),
            images: vec![GeneratedImage {
                bytes_base64: "iVBORw0KGgo=".to_string(),
                mime_type: "image/png".to_string(),
                dimensions: Some(MediaDimensions {
                    width: 1024,
                    height: 1024,
                }),
                revised_prompt: Some("a revised prompt".to_string()),
                watermark: None,
                safety: None,
            }],
            provider_response_id: Some("img_123".to_string()),
            usage: Some(MediaGenerationUsage {
                input_tokens: Some(10),
                input_image_tokens: Some(0),
                output_tokens: Some(1_056),
                total_tokens: Some(1_066),
            }),
            output_errors: Vec::new(),
        };

        let value = serde_json::to_value(&batch).unwrap();
        assert_eq!(value["images"][0]["revisedPrompt"], "a revised prompt");
        assert_eq!(value["usage"]["inputImageTokens"], 0);
        let round_trip: ImageGenerationBatch = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip, batch);
    }
}
