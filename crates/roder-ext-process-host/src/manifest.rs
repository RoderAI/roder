//! Loading and validating process-extension manifests from config.

use std::path::Path;

use roder_api::process_extension::{
    ProcessExtensionConfig, ProcessExtensionManifest, validate_manifest,
};

#[derive(Debug, Clone)]
pub struct LoadedProcessExtension {
    pub config: ProcessExtensionConfig,
    pub manifest: ProcessExtensionManifest,
    /// Raw manifest TOML; the child must echo its checksum on initialize.
    pub manifest_toml: String,
}

/// Loads the manifest referenced by `config.manifest` (relative paths
/// resolve against `base_dir`) and validates it against the supported
/// extension API.
pub fn load_process_extension(
    config: ProcessExtensionConfig,
    base_dir: &Path,
) -> anyhow::Result<LoadedProcessExtension> {
    let manifest_path = {
        let path = Path::new(&config.manifest);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base_dir.join(path)
        }
    };
    let manifest_toml = std::fs::read_to_string(&manifest_path).map_err(|err| {
        anyhow::anyhow!(
            "process extension {} manifest {} is unreadable: {err}",
            config.id,
            manifest_path.display()
        )
    })?;
    let manifest: ProcessExtensionManifest = toml::from_str(&manifest_toml).map_err(|err| {
        anyhow::anyhow!(
            "process extension {} manifest {} is invalid: {err}",
            config.id,
            manifest_path.display()
        )
    })?;
    validate_manifest(&manifest)?;
    Ok(LoadedProcessExtension {
        config,
        manifest,
        manifest_toml,
    })
}
