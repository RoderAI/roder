use roder_api::{RunnerManifest, RunnerSnapshotRef};

pub fn validate_snapshot_ref(snapshot: &RunnerSnapshotRef) -> anyhow::Result<()> {
    if looks_secret_like(&snapshot.snapshot_id) {
        anyhow::bail!("runner snapshot id looks like secret material");
    }
    reject_secret_json(&snapshot.metadata)
}

pub fn validate_snapshot_export(
    snapshot: &RunnerSnapshotRef,
    manifest: &RunnerManifest,
) -> anyhow::Result<()> {
    validate_snapshot_ref(snapshot)?;
    let encoded = snapshot.metadata.to_string();
    for mount in &manifest.mounts {
        let path = mount.path.to_string_lossy();
        if !path.is_empty() && encoded.contains(path.as_ref()) {
            anyhow::bail!(
                "runner snapshot metadata must not include mounted storage path: {}",
                mount.path.display()
            );
        }
        if !mount.intent.uri.is_empty() && encoded.contains(&mount.intent.uri) {
            anyhow::bail!("runner snapshot metadata must not include mounted storage uri");
        }
    }
    Ok(())
}

fn reject_secret_json(value: &serde_json::Value) -> anyhow::Result<()> {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                if looks_secret_like(key) {
                    anyhow::bail!("runner snapshot metadata key looks secret-like: {key}");
                }
                reject_secret_json(value)?;
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                reject_secret_json(value)?;
            }
        }
        serde_json::Value::String(value) if looks_secret_like(value) => {
            anyhow::bail!("runner snapshot metadata value looks secret-like");
        }
        _ => {}
    }
    Ok(())
}

fn looks_secret_like(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("secret") || lower.contains("token") || lower.contains("api_key")
}

#[cfg(test)]
mod tests {
    use roder_api::{RunnerManifest, RunnerMount, RunnerMountIntent, RunnerMountKind};

    use super::*;

    #[test]
    fn rejects_secret_like_snapshot_values() {
        let snapshot = RunnerSnapshotRef {
            provider_id: "docker".to_string(),
            snapshot_id: "snapshot-1".to_string(),
            metadata: serde_json::json!({ "api_key": "redacted" }),
        };

        assert!(validate_snapshot_ref(&snapshot).is_err());
    }

    #[test]
    fn rejects_snapshot_metadata_that_mentions_remote_mounts() {
        let manifest = RunnerManifest {
            entries: Vec::new(),
            mounts: vec![RunnerMount {
                name: "dataset".to_string(),
                path: "mnt/dataset".into(),
                read_only: true,
                intent: RunnerMountIntent {
                    kind: RunnerMountKind::S3,
                    uri: "s3://bucket/dataset".to_string(),
                    credentials: None,
                },
            }],
        };
        let snapshot = RunnerSnapshotRef {
            provider_id: "hosted".to_string(),
            snapshot_id: "snapshot-1".to_string(),
            metadata: serde_json::json!({ "excluded": ["mnt/dataset"] }),
        };

        assert!(validate_snapshot_export(&snapshot, &manifest).is_err());
    }
}
