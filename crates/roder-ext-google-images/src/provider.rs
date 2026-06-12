//! Google Gemini (Nano Banana) image generation adapter.
//!
//! Maps canonical text/reference/edit requests to Gemini `generateContent`
//! with text and inline image parts, parses inline image data parts, and
//! records the documented SynthID watermark in generation metadata.

use std::time::Duration;

use roder_api::catalog::{IMAGE_PROVIDER_GOOGLE, image_models_for_provider, lookup_image_model};
use roder_api::media::{
    GeneratedImage, ImageGenerationBatch, MediaGenerationRequest, MediaGenerationUsage,
    MediaGeneratorProvider, MediaProviderDescriptor,
};
use roder_api::reliability::{ReliabilityRequestPolicy, provider_retry_delay_ms};
use serde_json::{Value, json};

pub const GOOGLE_IMAGES_PROVIDER_ID: &str = IMAGE_PROVIDER_GOOGLE;

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const DEFAULT_MODEL: &str = "gemini-3.1-flash-image";
const ERROR_EXCERPT_LIMIT: usize = 600;

/// All Gemini image outputs carry a SynthID watermark per Google's docs.
const SYNTHID_WATERMARK: &str = "synthid";

#[derive(Debug, Clone)]
pub struct GoogleImagesConfig {
    pub api_key: Option<String>,
    pub base_url: String,
    pub retry_policy: ReliabilityRequestPolicy,
}

impl GoogleImagesConfig {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key,
            base_url: DEFAULT_BASE_URL.to_string(),
            retry_policy: ReliabilityRequestPolicy::default(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    pub fn with_retry_policy(mut self, retry_policy: ReliabilityRequestPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }
}

pub struct GoogleImagesProvider {
    config: GoogleImagesConfig,
}

impl GoogleImagesProvider {
    pub fn new(config: GoogleImagesConfig) -> Self {
        Self { config }
    }

    fn api_key(&self) -> anyhow::Result<&str> {
        self.config.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Gemini image generation API key is missing; set GEMINI_API_KEY or GEMINI_API_TOKEN, or configure media.image_generation.providers.google"
            )
        })
    }

    fn validate(&self, request: &MediaGenerationRequest) -> anyhow::Result<&'static str> {
        let model = request.model.as_deref().unwrap_or(DEFAULT_MODEL);
        let entry = lookup_image_model(GOOGLE_IMAGES_PROVIDER_ID, model).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown Gemini image model {model:?}; known models: {}",
                image_models_for_provider(GOOGLE_IMAGES_PROVIDER_ID)
                    .iter()
                    .map(|entry| entry.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        if let Some(count) = request.count
            && count > 1
        {
            anyhow::bail!(
                "Gemini image models generate one image per request; remove `count` or set it to 1"
            );
        }
        if let Some(aspect_ratio) = request.aspect_ratio.as_deref()
            && !entry.supported_aspect_ratios.contains(&aspect_ratio)
        {
            anyhow::bail!(
                "aspect ratio {aspect_ratio:?} is not supported by {model}; supported ratios: {}",
                entry.supported_aspect_ratios.join(", ")
            );
        }
        if let Some(image_size) = request.image_size.as_deref() {
            if entry.supported_image_sizes.is_empty() {
                anyhow::bail!(
                    "imageSize is not supported by {model}; use gemini-3.1-flash-image or gemini-3-pro-image for 1K/2K/4K output"
                );
            }
            if !entry.supported_image_sizes.contains(&image_size) {
                anyhow::bail!(
                    "image size {image_size:?} is not supported by {model}; supported sizes: {}",
                    entry.supported_image_sizes.join(", ")
                );
            }
        }
        if request.size.is_some() {
            anyhow::bail!(
                "Gemini image models use `aspectRatio` and `imageSize`, not pixel `size` values"
            );
        }
        for (field, present) in [
            ("quality", request.quality.is_some()),
            ("background", request.background.is_some()),
            ("outputFormat", request.output_format.is_some()),
            ("outputCompression", request.output_compression.is_some()),
            ("moderation", request.moderation.is_some()),
        ] {
            if present {
                anyhow::bail!("{field} is not supported by the Roder Gemini image provider");
            }
        }
        if request.partial_images.is_some() {
            anyhow::bail!(
                "partial image streaming is not supported by the Roder Gemini image provider"
            );
        }
        if let Some(options) = request.provider_options.as_ref()
            && !options.is_empty()
        {
            anyhow::bail!(
                "the Roder Gemini image provider does not accept providerOptions; use typed fields instead"
            );
        }
        Ok(entry.id)
    }

    fn request_body(model: &str, request: &MediaGenerationRequest) -> Value {
        let mut parts = vec![json!({ "text": request.prompt })];
        for image in &request.input_images {
            parts.push(json!({
                "inline_data": {
                    "mime_type": image.mime_type,
                    "data": image.bytes_base64,
                }
            }));
        }
        let mut generation_config = json!({ "responseModalities": ["TEXT", "IMAGE"] });
        let mut image_config = serde_json::Map::new();
        if let Some(aspect_ratio) = request.aspect_ratio.as_deref() {
            image_config.insert("aspectRatio".to_string(), json!(aspect_ratio));
        }
        if let Some(image_size) = request.image_size.as_deref() {
            image_config.insert("imageSize".to_string(), json!(image_size));
        }
        if !image_config.is_empty() {
            generation_config
                .as_object_mut()
                .expect("generation config is an object")
                .insert("imageConfig".to_string(), Value::Object(image_config));
        }
        let _ = model;
        json!({
            "contents": [{ "parts": parts }],
            "generationConfig": generation_config,
        })
    }

    async fn send(&self, model: &str, body: &Value) -> anyhow::Result<reqwest::Response> {
        let url = format!("{}/models/{model}:generateContent", self.config.base_url);
        let api_key = self.api_key()?;
        let policy = &self.config.retry_policy;
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let response = client()?
                .post(&url)
                // Header auth keeps the key out of URLs, logs, and error text.
                .header("x-goog-api-key", api_key)
                .json(body)
                .send()
                .await
                .map_err(|error| anyhow::anyhow!("Gemini image request failed: {error}"))?;
            let status = response.status().as_u16();
            if policy.provider_retry_status_codes.contains(&status)
                && attempt < policy.provider_retry_max_attempts
            {
                tokio::time::sleep(Duration::from_millis(provider_retry_delay_ms(
                    policy, attempt,
                )))
                .await;
                continue;
            }
            return Ok(response);
        }
    }

    async fn parse_response(
        &self,
        model: &str,
        response: reqwest::Response,
    ) -> anyhow::Result<ImageGenerationBatch> {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(map_provider_error(status.as_u16(), &body));
        }
        let value: Value = serde_json::from_str(&body)
            .map_err(|error| anyhow::anyhow!("Gemini image response was not JSON: {error}"))?;

        if let Some(block_reason) = value
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
        {
            anyhow::bail!("Gemini blocked the image generation prompt: {block_reason}");
        }

        let mut images = Vec::new();
        let mut output_errors = Vec::new();
        let candidates = value
            .get("candidates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for candidate in &candidates {
            if let Some(finish_reason) = candidate.get("finishReason").and_then(Value::as_str)
                && !matches!(finish_reason, "STOP" | "MAX_TOKENS")
            {
                output_errors.push(format!(
                    "Gemini candidate finished with reason {finish_reason}"
                ));
            }
            let parts = candidate
                .pointer("/content/parts")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for part in &parts {
                let inline = part.get("inlineData").or_else(|| part.get("inline_data"));
                let Some(inline) = inline else { continue };
                let Some(data) = inline.get("data").and_then(Value::as_str) else {
                    output_errors
                        .push("Gemini returned an inline image part without data".to_string());
                    continue;
                };
                let mime_type = inline
                    .get("mimeType")
                    .or_else(|| inline.get("mime_type"))
                    .and_then(Value::as_str)
                    .unwrap_or("image/png");
                images.push(GeneratedImage {
                    bytes_base64: data.to_string(),
                    mime_type: mime_type.to_string(),
                    dimensions: None,
                    revised_prompt: None,
                    watermark: Some(SYNTHID_WATERMARK.to_string()),
                    safety: None,
                });
            }
        }

        Ok(ImageGenerationBatch {
            provider: GOOGLE_IMAGES_PROVIDER_ID.to_string(),
            model: model.to_string(),
            images,
            provider_response_id: value
                .get("responseId")
                .and_then(Value::as_str)
                .map(str::to_string),
            usage: value.get("usageMetadata").map(parse_usage),
            output_errors,
        })
    }
}

#[async_trait::async_trait]
impl MediaGeneratorProvider for GoogleImagesProvider {
    fn provider_id(&self) -> &str {
        GOOGLE_IMAGES_PROVIDER_ID
    }

    fn descriptor(&self) -> MediaProviderDescriptor {
        MediaProviderDescriptor {
            id: GOOGLE_IMAGES_PROVIDER_ID.to_string(),
            display_name: "Google Gemini Images".to_string(),
            supports_images: true,
            supports_videos: false,
            configured: self.config.api_key.is_some(),
            default_model: Some(DEFAULT_MODEL.to_string()),
            image_models: roder_api::catalog::image_model_descriptors(GOOGLE_IMAGES_PROVIDER_ID),
        }
    }

    async fn generate_image(
        &self,
        request: MediaGenerationRequest,
    ) -> anyhow::Result<ImageGenerationBatch> {
        let model = self.validate(&request)?;
        let body = Self::request_body(model, &request);
        let response = self.send(model, &body).await?;
        self.parse_response(model, response).await
    }
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .read_timeout(Duration::from_secs(300))
        .build()?)
}

fn parse_usage(usage: &Value) -> MediaGenerationUsage {
    MediaGenerationUsage {
        input_tokens: usage.get("promptTokenCount").and_then(Value::as_u64),
        input_image_tokens: None,
        output_tokens: usage.get("candidatesTokenCount").and_then(Value::as_u64),
        total_tokens: usage.get("totalTokenCount").and_then(Value::as_u64),
    }
}

/// Auth failures never echo the response body; other provider errors surface
/// a bounded message excerpt.
fn map_provider_error(status: u16, body: &str) -> anyhow::Error {
    if status == 401 || status == 403 {
        return anyhow::anyhow!(
            "Gemini image generation authentication failed (status {status}); check GEMINI_API_KEY or GEMINI_API_TOKEN"
        );
    }
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(ERROR_EXCERPT_LIMIT).collect());
    anyhow::anyhow!(
        "Gemini image generation failed (status {status}): {}",
        message
            .chars()
            .take(ERROR_EXCERPT_LIMIT)
            .collect::<String>()
    )
}
