use roder_api::media::{
    MediaArtifact, MediaArtifactId, MediaDimensions, MediaGenerationRequest, MediaKind,
    MediaPreview, MediaPreviewStrategy,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;

#[derive(Debug, Clone)]
pub struct MediaArtifactStore {
    root: PathBuf,
    max_read_bytes: u64,
}

impl MediaArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_read_bytes: 10 * 1024 * 1024,
        }
    }

    pub fn with_max_read_bytes(mut self, max_read_bytes: u64) -> Self {
        self.max_read_bytes = max_read_bytes;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write_generated(
        &self,
        request: &MediaGenerationRequest,
        kind: MediaKind,
        mime_type: &str,
        provider: &str,
        bytes: &[u8],
        dimensions: Option<MediaDimensions>,
        duration_millis: Option<u64>,
    ) -> anyhow::Result<(MediaArtifact, MediaPreview)> {
        std::fs::create_dir_all(&self.root)?;
        let ext = extension_for_mime(mime_type);
        let id = format!("media-{}", uuid::Uuid::new_v4());
        let store_path = self.root.join(format!("{id}.{ext}"));
        std::fs::write(&store_path, bytes)?;
        let artifact = MediaArtifact {
            id: id.clone(),
            kind: kind.clone(),
            mime_type: mime_type.to_string(),
            dimensions,
            duration_millis,
            byte_size: bytes.len() as u64,
            provider: provider.to_string(),
            prompt_hash: prompt_hash(&request.prompt),
            store_path: store_path.display().to_string(),
            thumbnail_path: Some(store_path.display().to_string()),
            created_at: OffsetDateTime::now_utc(),
            roder_owned: true,
        };
        self.write_metadata(&artifact)?;
        let preview = preview_for(&artifact);
        Ok((artifact, preview))
    }

    pub fn list(&self) -> anyhow::Result<Vec<MediaArtifact>> {
        let mut artifacts = Vec::new();
        if !self.root.exists() {
            return Ok(artifacts);
        }
        for entry in std::fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(path)?;
            artifacts.push(serde_json::from_str(&text)?);
        }
        artifacts.sort_by(|left: &MediaArtifact, right| left.id.cmp(&right.id));
        Ok(artifacts)
    }

    pub fn read(
        &self,
        artifact_id: &MediaArtifactId,
        max_bytes: Option<u64>,
    ) -> anyhow::Result<(MediaArtifact, Vec<u8>)> {
        let artifact = self.get(artifact_id)?;
        let limit = max_bytes.unwrap_or(self.max_read_bytes);
        if artifact.byte_size > limit {
            anyhow::bail!(
                "media artifact {} is {} bytes, over read limit {}",
                artifact.id,
                artifact.byte_size,
                limit
            );
        }
        let bytes = std::fs::read(&artifact.store_path)?;
        Ok((artifact, bytes))
    }

    pub fn get(&self, artifact_id: &MediaArtifactId) -> anyhow::Result<MediaArtifact> {
        let path = self.metadata_path(artifact_id);
        let text = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn delete(&self, artifact_id: &MediaArtifactId) -> anyhow::Result<bool> {
        let artifact = self.get(artifact_id)?;
        if !artifact.roder_owned {
            anyhow::bail!(
                "refusing to delete non-Roder-owned artifact {}",
                artifact.id
            );
        }
        let mut deleted = false;
        let data_path = PathBuf::from(&artifact.store_path);
        if data_path.starts_with(&self.root) && data_path.exists() {
            std::fs::remove_file(data_path)?;
            deleted = true;
        }
        let metadata = self.metadata_path(artifact_id);
        if metadata.exists() {
            std::fs::remove_file(metadata)?;
            deleted = true;
        }
        Ok(deleted)
    }

    pub fn preview(&self, artifact_id: &MediaArtifactId) -> anyhow::Result<MediaPreview> {
        Ok(preview_for(&self.get(artifact_id)?))
    }

    fn write_metadata(&self, artifact: &MediaArtifact) -> anyhow::Result<()> {
        let path = self.metadata_path(&artifact.id);
        std::fs::write(path, serde_json::to_string_pretty(artifact)?)?;
        Ok(())
    }

    fn metadata_path(&self, artifact_id: &MediaArtifactId) -> PathBuf {
        self.root.join(format!("{artifact_id}.json"))
    }
}

pub fn default_media_artifact_dir() -> anyhow::Result<PathBuf> {
    let data_dir = std::env::var_os("RODER_DATA_DIR")
        .or_else(|| std::env::var_os("RODER_CONFIG_DIR"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".roder")))
        .ok_or_else(|| {
            anyhow::anyhow!("could not resolve Roder data directory for media artifacts")
        })?;
    Ok(data_dir.join("artifacts"))
}

fn preview_for(artifact: &MediaArtifact) -> MediaPreview {
    let strategy = match artifact.kind {
        MediaKind::Image => MediaPreviewStrategy::Thumbnail,
        _ => MediaPreviewStrategy::MetadataOnly,
    };
    MediaPreview {
        artifact_id: artifact.id.clone(),
        strategy,
        thumbnail_path: artifact.thumbnail_path.clone(),
        fallback_label: format!(
            "{} {} ({} bytes)",
            artifact.provider, artifact.mime_type, artifact.byte_size
        ),
        warning: None,
    }
}

fn prompt_hash(prompt: &str) -> String {
    let mut hasher = DefaultHasher::new();
    prompt.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn extension_for_mime(mime_type: &str) -> &'static str {
    match mime_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "video/mp4" => "mp4",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_artifact_store_writes_reads_previews_and_deletes_owned_artifacts() {
        let root =
            std::env::temp_dir().join(format!("roder-media-artifacts-{}", uuid::Uuid::new_v4()));
        let store = MediaArtifactStore::new(&root).with_max_read_bytes(1024);
        let request = MediaGenerationRequest {
            prompt: "tiny image".to_string(),
            model: None,
            output_path: None,
        };

        let (artifact, preview) = store
            .write_generated(
                &request,
                MediaKind::Image,
                "image/png",
                "fake",
                b"abc",
                Some(MediaDimensions {
                    width: 1,
                    height: 1,
                }),
                None,
            )
            .unwrap();

        assert!(
            artifact
                .store_path
                .starts_with(root.to_string_lossy().as_ref())
        );
        assert_eq!(preview.artifact_id, artifact.id);
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(store.read(&artifact.id, None).unwrap().1, b"abc");
        assert!(store.delete(&artifact.id).unwrap());
        assert!(store.list().unwrap().is_empty());
    }
}
