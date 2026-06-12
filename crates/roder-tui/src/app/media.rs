#![allow(dead_code)]

use ratatui::text::{Line, Span};
use roder_api::media::{MediaArtifact, MediaKind, MediaPreview, MediaPreviewStrategy};

use super::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MediaTimelineRow {
    artifact: MediaArtifact,
    preview: Option<MediaPreview>,
}

impl MediaTimelineRow {
    pub fn new(artifact: MediaArtifact, preview: Option<MediaPreview>) -> Self {
        Self { artifact, preview }
    }

    pub fn render(&self, theme: Theme, lines: &mut Vec<Line<'static>>) {
        lines.push(Line::from(vec![
            Span::styled("  media ", theme.tool()),
            Span::styled(
                format!(
                    "{} {} {} bytes",
                    kind_label(&self.artifact.kind),
                    self.artifact.mime_type,
                    self.artifact.byte_size
                ),
                theme.text(),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    ", theme.subtle()),
            Span::styled(
                format!("{} · {}", self.artifact.provider, self.artifact.store_path),
                theme.muted(),
            ),
        ]));
        if let Some(preview) = &self.preview {
            let strategy = match preview.strategy {
                MediaPreviewStrategy::InlineImage => "inline preview",
                MediaPreviewStrategy::Thumbnail => "thumbnail",
                MediaPreviewStrategy::MetadataOnly => "metadata",
            };
            lines.push(Line::from(vec![
                Span::styled("    ", theme.subtle()),
                Span::styled(
                    format!("{strategy}: {}", preview.fallback_label),
                    theme.subtle(),
                ),
            ]));
        }
    }
}

fn kind_label(kind: &MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
        MediaKind::Other => "media",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::media::{MediaDimensions, MediaPreviewStrategy};
    use time::OffsetDateTime;

    #[test]
    fn media_row_renders_fallback_preview_and_path() {
        let artifact = MediaArtifact {
            id: "media-1".to_string(),
            kind: MediaKind::Image,
            mime_type: "image/png".to_string(),
            dimensions: Some(MediaDimensions {
                width: 1,
                height: 1,
            }),
            duration_millis: None,
            byte_size: 3,
            provider: "fake".to_string(),
            prompt_hash: "hash".to_string(),
            store_path: "/tmp/media.png".to_string(),
            thumbnail_path: None,
            generation: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            roder_owned: true,
        };
        let row = MediaTimelineRow::new(
            artifact,
            Some(MediaPreview {
                artifact_id: "media-1".to_string(),
                strategy: MediaPreviewStrategy::MetadataOnly,
                thumbnail_path: None,
                fallback_label: "fake image/png".to_string(),
                warning: None,
            }),
        );
        let mut lines = Vec::new();
        row.render(Theme::for_terminal(), &mut lines);
        let text = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("/tmp/media.png"));
        assert!(text.contains("fake image/png"));
    }
}
