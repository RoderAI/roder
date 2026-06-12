use std::sync::Arc;

use base64::Engine;
use roder_api::media::{
    MediaArtifact, MediaDimensions, MediaGenerationOutput, MediaGenerationResponse, MediaKind,
    data_url,
};
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;
use time::OffsetDateTime;

pub(crate) fn register(registry: &mut ToolRegistry) -> anyhow::Result<()> {
    registry.register(Arc::new(GenerateImageTool))?;
    registry.register(Arc::new(GenerateVideoTool))?;
    registry.register(Arc::new(DescribeMediaTool))?;
    registry.register(Arc::new(AttachMediaTool))
}

#[derive(Debug)]
struct GenerateImageTool;

#[derive(Debug)]
struct GenerateVideoTool;

#[derive(Debug)]
struct DescribeMediaTool;

#[derive(Debug)]
struct AttachMediaTool;

#[derive(Debug, Deserialize)]
struct GenerateArgs {
    prompt: String,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ArtifactArg {
    artifact: MediaArtifact,
}

#[async_trait::async_trait]
impl ToolExecutor for GenerateImageTool {
    fn spec(&self) -> ToolSpec {
        media_tool_spec(
            "media_generate_image",
            "Generates a deterministic offline image artifact for tests.",
        )
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<GenerateArgs>(&call)?;
        let response = fake_response(
            &args.prompt,
            args.model.as_deref(),
            MediaKind::Image,
            "image/png",
            "iVBORw0KGgo=",
            Some(MediaDimensions {
                width: 1,
                height: 1,
            }),
            None,
        );
        Ok(result(call, response, "generated image artifact"))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for GenerateVideoTool {
    fn spec(&self) -> ToolSpec {
        media_tool_spec(
            "media_generate_video",
            "Generates a deterministic offline video artifact for tests.",
        )
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<GenerateArgs>(&call)?;
        let response = fake_response(
            &args.prompt,
            args.model.as_deref(),
            MediaKind::Video,
            "video/mp4",
            "AAAAHGZ0eXBtcDQy",
            None,
            Some(1_000),
        );
        Ok(result(call, response, "generated video artifact"))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for DescribeMediaTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "media_describe".to_string(),
            description: "Returns metadata for a media artifact.".to_string(),
            parameters: artifact_schema(),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<ArtifactArg>(&call)?;
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: format!(
                "{} {} {} bytes",
                args.artifact.id, args.artifact.mime_type, args.artifact.byte_size
            ),
            data: json!({ "artifact": args.artifact }),
            is_error: false,
        })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for AttachMediaTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "media_attach".to_string(),
            description: "Converts generated media bytes into a later-turn attachment payload."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "artifact": { "type": "object" },
                    "bytesBase64": { "type": "string" }
                },
                "required": ["artifact", "bytesBase64"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Args {
            artifact: MediaArtifact,
            bytes_base64: String,
        }
        let args = parse::<Args>(&call)?;
        let url = data_url(&args.artifact.mime_type, &args.bytes_base64);
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: format!("attached media {}", args.artifact.id),
            data: json!({
                "attachment": {
                    "artifactId": args.artifact.id,
                    "mimeType": args.artifact.mime_type,
                    "dataUrl": url
                }
            }),
            is_error: false,
        })
    }
}

fn media_tool_spec(name: &str, description: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string" },
                "model": { "type": "string" },
                "outputPath": { "type": "string" }
            },
            "required": ["prompt"],
            "additionalProperties": false
        }),
    }
}

fn artifact_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "artifact": { "type": "object" }
        },
        "required": ["artifact"],
        "additionalProperties": false
    })
}

fn fake_response(
    prompt: &str,
    model: Option<&str>,
    kind: MediaKind,
    mime_type: &str,
    bytes_base64: &str,
    dimensions: Option<MediaDimensions>,
    duration_millis: Option<u64>,
) -> MediaGenerationResponse {
    let artifact = MediaArtifact {
        id: format!("media-{}", stable_id(prompt, mime_type)),
        kind,
        mime_type: mime_type.to_string(),
        dimensions,
        duration_millis,
        byte_size: base64::engine::general_purpose::STANDARD
            .decode(bytes_base64)
            .map(|bytes| bytes.len() as u64)
            .unwrap_or(0),
        provider: "fake-media".to_string(),
        prompt_hash: stable_id(prompt, mime_type),
        store_path: format!("memory://{}", stable_id(prompt, mime_type)),
        thumbnail_path: None,
        generation: None,
        created_at: OffsetDateTime::UNIX_EPOCH,
        roder_owned: true,
    };
    let preview = roder_api::media::MediaPreview {
        artifact_id: artifact.id.clone(),
        strategy: if artifact.kind == MediaKind::Image {
            roder_api::media::MediaPreviewStrategy::InlineImage
        } else {
            roder_api::media::MediaPreviewStrategy::MetadataOnly
        },
        thumbnail_path: None,
        fallback_label: format!("{} {}", artifact.provider, artifact.mime_type),
        warning: None,
    };
    MediaGenerationResponse {
        provider: "fake-media".to_string(),
        model: model.map(str::to_string),
        outputs: vec![MediaGenerationOutput {
            artifact,
            preview,
            revised_prompt: None,
        }],
        revised_prompt: None,
        provider_response_id: None,
        usage: None,
        watermark: None,
        safety: None,
        output_errors: Vec::new(),
    }
}

fn result(call: ToolCall, response: MediaGenerationResponse, text: &str) -> ToolResult {
    let artifact_ids: Vec<&str> = response
        .outputs
        .iter()
        .map(|output| output.artifact.id.as_str())
        .collect();
    let artifacts: Vec<&MediaArtifact> = response
        .outputs
        .iter()
        .map(|output| &output.artifact)
        .collect();
    let previews: Vec<&roder_api::media::MediaPreview> = response
        .outputs
        .iter()
        .map(|output| &output.preview)
        .collect();
    ToolResult {
        id: call.id,
        name: call.name,
        text: format!("{text}: {}", artifact_ids.join(", ")),
        data: json!({
            "mediaArtifacts": artifacts,
            "mediaPreviews": previews,
            "mediaGeneration": response,
        }),
        is_error: false,
    }
}

fn stable_id(prompt: &str, mime_type: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    prompt.hash(&mut hasher);
    mime_type.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn parse<T: for<'de> Deserialize<'de>>(call: &ToolCall) -> anyhow::Result<T> {
    serde_json::from_value(call.arguments.clone()).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::policy_mode::PolicyMode;

    #[tokio::test]
    async fn fake_image_tool_returns_media_artifact_payload() {
        let mut registry = ToolRegistry::default();
        register(&mut registry).unwrap();
        let tool = registry.get("media_generate_image").unwrap();
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

        assert_eq!(result.data["mediaArtifacts"][0]["kind"], "image");
        assert_eq!(result.data["mediaPreviews"][0]["strategy"], "inlineImage");
        assert_eq!(result.data["mediaGeneration"]["provider"], "fake-media");
    }
}
