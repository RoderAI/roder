use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub type MediaArtifactId = String;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaGenerationRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaGenerationResponse {
    pub artifact: MediaArtifact,
    pub preview: MediaPreview,
}

pub fn data_url(mime_type: &str, bytes_base64: &str) -> String {
    format!("data:{mime_type};base64,{bytes_base64}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_and_video_artifacts_serialize_as_camel_case_metadata() {
        let image = MediaArtifact {
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
            created_at: OffsetDateTime::UNIX_EPOCH,
            roder_owned: true,
        };
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
        assert_eq!(serde_json::to_value(video).unwrap()["durationMillis"], 1000);
    }
}
