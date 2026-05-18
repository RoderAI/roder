pub use roder_api::{
    RunnerMount, RunnerMountCapabilities, RunnerMountIntent, RunnerMountKind, RunnerSecretRef,
};

pub fn validate_mount_intent(mount: &RunnerMount) -> anyhow::Result<()> {
    if mount.intent.uri.trim().is_empty() {
        if matches!(mount.intent.kind, RunnerMountKind::ProviderNative) {
            return Ok(());
        }
        anyhow::bail!("runner mount {} requires a storage uri", mount.name);
    }
    if mount.intent.credentials.as_ref().is_some_and(|secret| {
        secret.id.trim().is_empty()
            || looks_inline_secret(&secret.id)
            || secret.id.contains('=')
            || secret.id.contains('\n')
    }) {
        anyhow::bail!(
            "runner mount {} credentials must be a secret reference id",
            mount.name
        );
    }
    Ok(())
}

pub fn validate_mount_supported(
    mount: &RunnerMount,
    capabilities: &RunnerMountCapabilities,
) -> anyhow::Result<()> {
    let supported = match mount.intent.kind {
        RunnerMountKind::S3 => capabilities.s3,
        RunnerMountKind::Gcs => capabilities.gcs,
        RunnerMountKind::R2 => capabilities.r2,
        RunnerMountKind::AzureBlob => capabilities.azure_blob,
        RunnerMountKind::BoxStorage => capabilities.box_storage,
        RunnerMountKind::ProviderNative => capabilities.provider_native,
    };
    if !supported {
        anyhow::bail!(
            "runner mount kind {:?} is not supported by this provider",
            mount.intent.kind
        );
    }
    Ok(())
}

fn looks_inline_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("secret_")
        || lower.contains("token_")
        || lower.contains("apikey")
        || lower.contains("api_key")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn validates_provider_neutral_mount_intents() {
        let mount = RunnerMount {
            name: "dataset".to_string(),
            path: PathBuf::from("mnt/dataset"),
            read_only: true,
            intent: RunnerMountIntent {
                kind: RunnerMountKind::S3,
                uri: "s3://bucket/prefix".to_string(),
                credentials: Some(RunnerSecretRef {
                    id: "aws-prod-readonly".to_string(),
                }),
            },
        };

        validate_mount_intent(&mount).unwrap();
        validate_mount_supported(
            &mount,
            &RunnerMountCapabilities {
                s3: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(validate_mount_supported(&mount, &RunnerMountCapabilities::default()).is_err());
    }

    #[test]
    fn rejects_inline_secret_material_in_mounts() {
        let mount = RunnerMount {
            name: "dataset".to_string(),
            path: PathBuf::from("mnt/dataset"),
            read_only: true,
            intent: RunnerMountIntent {
                kind: RunnerMountKind::Gcs,
                uri: "gs://bucket/prefix".to_string(),
                credentials: Some(RunnerSecretRef {
                    id: "api_key=plain-text".to_string(),
                }),
            },
        };

        assert!(validate_mount_intent(&mount).is_err());
    }
}
