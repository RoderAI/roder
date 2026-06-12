//! OpenAI Image API adapter for the Roder media generation contract.
//!
//! Maps canonical text-to-image requests to `POST /v1/images/generations`
//! and edit/reference-image requests to multipart `POST /v1/images/edits`.
//! GPT Image model ids (`gpt-image-*`) are direct Image API model ids; the
//! Responses API hosted `image_generation` tool is intentionally out of
//! scope for this provider.

use std::time::Duration;

use base64::Engine;
use roder_api::catalog::{IMAGE_PROVIDER_OPENAI, image_models_for_provider, lookup_image_model};
use roder_api::media::{
    GeneratedImage, ImageGenerationAction, ImageGenerationBatch, MediaDimensions,
    MediaGenerationRequest, MediaGenerationUsage, MediaGeneratorProvider, MediaProviderDescriptor,
};
use roder_api::reliability::{ReliabilityRequestPolicy, provider_retry_delay_ms};
use serde_json::{Value, json};

pub const OPENAI_IMAGES_PROVIDER_ID: &str = IMAGE_PROVIDER_OPENAI;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-image-2";
const ERROR_EXCERPT_LIMIT: usize = 600;

/// Documented pass-through `providerOptions` keys for this provider.
const ALLOWED_PROVIDER_OPTIONS: &[&str] = &["user"];

#[derive(Debug, Clone)]
pub struct OpenAiImagesConfig {
    pub api_key: Option<String>,
    pub base_url: String,
    pub retry_policy: ReliabilityRequestPolicy,
}

impl OpenAiImagesConfig {
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

pub struct OpenAiImagesProvider {
    config: OpenAiImagesConfig,
}

impl OpenAiImagesProvider {
    pub fn new(config: OpenAiImagesConfig) -> Self {
        Self { config }
    }

    fn api_key(&self) -> anyhow::Result<&str> {
        self.config.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "OpenAI image generation API key is missing; set OPENAI_API_KEY or configure media.image_generation.providers.openai"
            )
        })
    }

    fn validate(&self, request: &MediaGenerationRequest) -> anyhow::Result<&'static str> {
        let model = request.model.as_deref().unwrap_or(DEFAULT_MODEL);
        let entry = lookup_image_model(OPENAI_IMAGES_PROVIDER_ID, model).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown OpenAI image model {model:?}; known models: {}",
                image_models_for_provider(OPENAI_IMAGES_PROVIDER_ID)
                    .iter()
                    .map(|entry| entry.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        if let Some(size) = request.size.as_deref()
            && !entry.supported_sizes.contains(&size)
        {
            anyhow::bail!(
                "size {size:?} is not supported by {model}; supported sizes: {}",
                entry.supported_sizes.join(", ")
            );
        }
        if request.aspect_ratio.is_some() {
            anyhow::bail!(
                "OpenAI image models use `size` (e.g. 1536x1024), not `aspectRatio`; supported sizes: {}",
                entry.supported_sizes.join(", ")
            );
        }
        if request.image_size.is_some() {
            anyhow::bail!(
                "OpenAI image models use `size` (e.g. 1536x1024), not the `imageSize` resolution tier"
            );
        }
        if let Some(format) = request.output_format.as_deref()
            && !matches!(format, "png" | "jpeg" | "webp")
        {
            anyhow::bail!(
                "output format {format:?} is not supported by OpenAI image models; use png, jpeg, or webp"
            );
        }
        if request.partial_images.is_some() {
            anyhow::bail!(
                "partial image streaming is not supported by the Roder OpenAI image provider yet"
            );
        }
        if let Some(options) = request.provider_options.as_ref() {
            for key in options.keys() {
                if !ALLOWED_PROVIDER_OPTIONS.contains(&key.as_str()) {
                    anyhow::bail!(
                        "unsupported OpenAI providerOptions key {key:?}; supported keys: {}",
                        ALLOWED_PROVIDER_OPTIONS.join(", ")
                    );
                }
            }
        }
        Ok(entry.id)
    }

    fn wants_edit(request: &MediaGenerationRequest) -> bool {
        match request.action {
            Some(ImageGenerationAction::Edit) => true,
            Some(ImageGenerationAction::Generate) => false,
            Some(ImageGenerationAction::Auto) | None => !request.input_images.is_empty(),
        }
    }

    async fn send_generation(
        &self,
        model: &str,
        request: &MediaGenerationRequest,
    ) -> anyhow::Result<reqwest::Response> {
        let mut body = json!({
            "model": model,
            "prompt": request.prompt,
        });
        let object = body.as_object_mut().expect("body is an object");
        if let Some(count) = request.count {
            object.insert("n".to_string(), json!(count));
        }
        if let Some(size) = request.size.as_deref() {
            object.insert("size".to_string(), json!(size));
        }
        if let Some(quality) = request.quality.as_deref() {
            object.insert("quality".to_string(), json!(quality));
        }
        if let Some(format) = request.output_format.as_deref() {
            object.insert("output_format".to_string(), json!(format));
        }
        if let Some(background) = request.background.as_deref() {
            object.insert("background".to_string(), json!(background));
        }
        if let Some(compression) = request.output_compression {
            object.insert("output_compression".to_string(), json!(compression));
        }
        if let Some(moderation) = request.moderation.as_deref() {
            object.insert("moderation".to_string(), json!(moderation));
        }
        if let Some(options) = request.provider_options.as_ref() {
            for (key, value) in options {
                object.insert(key.clone(), value.clone());
            }
        }

        let url = format!("{}/images/generations", self.config.base_url);
        let api_key = self.api_key()?;
        let policy = &self.config.retry_policy;
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let response = client()?
                .post(&url)
                .bearer_auth(api_key)
                .json(&body)
                .send()
                .await
                .map_err(|error| anyhow::anyhow!("OpenAI image request failed: {error}"))?;
            let status = response.status().as_u16();
            // JSON generation requests are idempotent enough to retry on
            // transient provider failures; multipart edits are never retried.
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

    async fn send_edit(
        &self,
        model: &str,
        request: &MediaGenerationRequest,
    ) -> anyhow::Result<reqwest::Response> {
        if request.input_images.is_empty() {
            anyhow::bail!("OpenAI image edits require at least one input image");
        }
        let mut form = reqwest::multipart::Form::new()
            .text("model", model.to_string())
            .text("prompt", request.prompt.clone());
        for (index, image) in request.input_images.iter().enumerate() {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&image.bytes_base64)
                .map_err(|error| anyhow::anyhow!("input image {index} is not valid base64: {error}"))?;
            let extension = match image.mime_type.as_str() {
                "image/jpeg" => "jpg",
                "image/webp" => "webp",
                _ => "png",
            };
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(format!("input-{index}.{extension}"))
                .mime_str(&image.mime_type)
                .map_err(|error| {
                    anyhow::anyhow!("input image {index} has invalid mime type: {error}")
                })?;
            form = form.part("image[]", part);
        }
        if let Some(count) = request.count {
            form = form.text("n", count.to_string());
        }
        if let Some(size) = request.size.as_deref() {
            form = form.text("size", size.to_string());
        }
        if let Some(quality) = request.quality.as_deref() {
            form = form.text("quality", quality.to_string());
        }
        if let Some(format) = request.output_format.as_deref() {
            form = form.text("output_format", format.to_string());
        }
        if let Some(background) = request.background.as_deref() {
            form = form.text("background", background.to_string());
        }
        if let Some(options) = request.provider_options.as_ref() {
            for (key, value) in options {
                if let Some(text) = value.as_str() {
                    form = form.text(key.clone(), text.to_string());
                }
            }
        }

        let url = format!("{}/images/edits", self.config.base_url);
        // Multipart uploads are not retried: the request is not marked
        // retry-safe and re-uploading reference images on transient failures
        // risks duplicate billing.
        client()?
            .post(&url)
            .bearer_auth(self.api_key()?)
            .multipart(form)
            .send()
            .await
            .map_err(|error| anyhow::anyhow!("OpenAI image edit request failed: {error}"))
    }

    async fn parse_response(
        &self,
        model: &str,
        request: &MediaGenerationRequest,
        response: reqwest::Response,
    ) -> anyhow::Result<ImageGenerationBatch> {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(map_provider_error(status.as_u16(), &body));
        }
        let value: Value = serde_json::from_str(&body)
            .map_err(|error| anyhow::anyhow!("OpenAI image response was not JSON: {error}"))?;

        let mime_type = match value
            .get("output_format")
            .and_then(Value::as_str)
            .or(request.output_format.as_deref())
            .unwrap_or("png")
        {
            "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "image/png",
        };
        let dimensions = value
            .get("size")
            .and_then(Value::as_str)
            .or(request.size.as_deref())
            .and_then(parse_dimensions);

        let data = value
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut images = Vec::new();
        let mut output_errors = Vec::new();
        for entry in &data {
            let Some(bytes_base64) = entry.get("b64_json").and_then(Value::as_str) else {
                output_errors
                    .push("OpenAI returned an output without base64 image data".to_string());
                continue;
            };
            images.push(GeneratedImage {
                bytes_base64: bytes_base64.to_string(),
                mime_type: mime_type.to_string(),
                dimensions: dimensions.clone(),
                revised_prompt: entry
                    .get("revised_prompt")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                watermark: None,
                safety: None,
            });
        }

        Ok(ImageGenerationBatch {
            provider: OPENAI_IMAGES_PROVIDER_ID.to_string(),
            model: model.to_string(),
            images,
            provider_response_id: value.get("id").and_then(Value::as_str).map(str::to_string),
            usage: value.get("usage").map(parse_usage),
            output_errors,
        })
    }
}

#[async_trait::async_trait]
impl MediaGeneratorProvider for OpenAiImagesProvider {
    fn provider_id(&self) -> &str {
        OPENAI_IMAGES_PROVIDER_ID
    }

    fn descriptor(&self) -> MediaProviderDescriptor {
        MediaProviderDescriptor {
            id: OPENAI_IMAGES_PROVIDER_ID.to_string(),
            display_name: "OpenAI GPT Image".to_string(),
            supports_images: true,
            supports_videos: false,
            configured: self.config.api_key.is_some(),
            default_model: Some(DEFAULT_MODEL.to_string()),
            image_models: roder_api::catalog::image_model_descriptors(OPENAI_IMAGES_PROVIDER_ID),
        }
    }

    async fn generate_image(
        &self,
        request: MediaGenerationRequest,
    ) -> anyhow::Result<ImageGenerationBatch> {
        let model = self.validate(&request)?;
        let response = if Self::wants_edit(&request) {
            self.send_edit(model, &request).await?
        } else {
            self.send_generation(model, &request).await?
        };
        self.parse_response(model, &request, response).await
    }
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .read_timeout(Duration::from_secs(300))
        .build()?)
}

fn parse_dimensions(size: &str) -> Option<MediaDimensions> {
    let (width, height) = size.split_once('x')?;
    Some(MediaDimensions {
        width: width.parse().ok()?,
        height: height.parse().ok()?,
    })
}

fn parse_usage(usage: &Value) -> MediaGenerationUsage {
    MediaGenerationUsage {
        input_tokens: usage.get("input_tokens").and_then(Value::as_u64),
        input_image_tokens: usage
            .get("input_tokens_details")
            .and_then(|details| details.get("image_tokens"))
            .and_then(Value::as_u64),
        output_tokens: usage.get("output_tokens").and_then(Value::as_u64),
        total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
    }
}

/// Auth failures never echo the response body (it may quote header values);
/// other provider errors surface a bounded message excerpt.
fn map_provider_error(status: u16, body: &str) -> anyhow::Error {
    if status == 401 || status == 403 {
        return anyhow::anyhow!(
            "OpenAI image generation authentication failed (status {status}); check OPENAI_API_KEY and organization verification"
        );
    }
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(ERROR_EXCERPT_LIMIT).collect());
    anyhow::anyhow!(
        "OpenAI image generation failed (status {status}): {}",
        message.chars().take(ERROR_EXCERPT_LIMIT).collect::<String>()
    )
}
