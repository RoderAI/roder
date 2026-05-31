use std::collections::BTreeSet;
use std::path::Path;

use roder_api::{RunnerManifest, RunnerManifestEntry, RunnerMount};

use crate::mount::validate_mount_intent;

pub fn validate_manifest(manifest: &RunnerManifest) -> anyhow::Result<()> {
    let mut targets = BTreeSet::new();
    for entry in &manifest.entries {
        validate_entry(entry)?;
        if !targets.insert(entry.target.clone()) {
            anyhow::bail!(
                "duplicate runner manifest target: {}",
                entry.target.display()
            );
        }
    }
    let mut mounts = BTreeSet::new();
    for mount in &manifest.mounts {
        validate_mount(mount)?;
        if !mounts.insert(mount.name.clone()) {
            anyhow::bail!("duplicate runner mount: {}", mount.name);
        }
    }
    Ok(())
}

fn validate_entry(entry: &RunnerManifestEntry) -> anyhow::Result<()> {
    validate_relative_path(&entry.source, "manifest source")?;
    validate_relative_path(&entry.target, "manifest target")
}

fn validate_mount(mount: &RunnerMount) -> anyhow::Result<()> {
    if mount.name.trim().is_empty() {
        anyhow::bail!("runner mount name cannot be empty");
    }
    validate_relative_path(&mount.path, "mount path")?;
    validate_mount_intent(mount)
}

pub(crate) fn validate_relative_path(path: &Path, label: &str) -> anyhow::Result<()> {
    if path.is_absolute() {
        anyhow::bail!("{label} must be relative: {}", path.display());
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        anyhow::bail!("{label} cannot contain '..': {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use roder_api::{RunnerManifestEntry, RunnerMount};

    use super::*;

    #[test]
    fn validates_relative_manifest_entries_and_mounts() {
        let manifest = RunnerManifest {
            entries: vec![RunnerManifestEntry {
                source: "src".into(),
                target: "workspace/src".into(),
                writable: true,
            }],
            mounts: vec![RunnerMount {
                name: "cache".to_string(),
                path: ".cache".into(),
                read_only: false,
                intent: Default::default(),
            }],
        };

        validate_manifest(&manifest).unwrap();
    }

    #[test]
    fn rejects_absolute_manifest_paths_and_escapes() {
        let absolute = RunnerManifest {
            entries: vec![RunnerManifestEntry {
                source: std::env::temp_dir().join("src"),
                target: "workspace/src".into(),
                writable: true,
            }],
            mounts: vec![],
        };
        assert!(validate_manifest(&absolute).is_err());

        let escape = RunnerManifest {
            entries: vec![RunnerManifestEntry {
                source: "src".into(),
                target: "../secret".into(),
                writable: true,
            }],
            mounts: vec![],
        };
        assert!(validate_manifest(&escape).is_err());
    }

    #[test]
    fn rejects_duplicate_manifest_targets_and_mounts() {
        let duplicate_targets = RunnerManifest {
            entries: vec![
                RunnerManifestEntry {
                    source: "src-a".into(),
                    target: "workspace/src".into(),
                    writable: true,
                },
                RunnerManifestEntry {
                    source: "src-b".into(),
                    target: "workspace/src".into(),
                    writable: true,
                },
            ],
            mounts: vec![],
        };
        assert!(validate_manifest(&duplicate_targets).is_err());

        let duplicate_mounts = RunnerManifest {
            entries: vec![],
            mounts: vec![
                RunnerMount {
                    name: "cache".to_string(),
                    path: ".cache".into(),
                    read_only: false,
                    intent: Default::default(),
                },
                RunnerMount {
                    name: "cache".to_string(),
                    path: ".cache2".into(),
                    read_only: false,
                    intent: Default::default(),
                },
            ],
        };
        assert!(validate_manifest(&duplicate_mounts).is_err());
    }
}
