use std::path::{Path, PathBuf};

use roder_api::extension::ExtensionRegistry;
use roder_api::skills::SkillDescriptor;
use roder_core::Runtime;
use roder_extension_host::discovery_catalog::{
    DiscoveryCatalogBuildOptions, build_file_backed_catalog_with_skills,
};

pub(crate) async fn initialize(runtime: &Runtime) -> anyhow::Result<()> {
    let root = roder_config::config_dir();
    let options = DiscoveryCatalogBuildOptions::new(
        std::env::var_os("RODER_DISCOVERY_CATALOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("discovery")),
        std::env::var_os("RODER_DISCOVERY_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("threads").join("discovery-state")),
    );
    let skills = runtime
        .skills_snapshot()
        .await
        .skills()
        .iter()
        .map(|skill| skill.descriptor.clone())
        .collect::<Vec<_>>();
    initialize_catalog(runtime.registry(), &runtime.workspace(), &skills, &options)
}

fn initialize_catalog(
    registry: &ExtensionRegistry,
    workspace: &Path,
    skills: &[SkillDescriptor],
    options: &DiscoveryCatalogBuildOptions,
) -> anyhow::Result<()> {
    let workflow =
        roder_config::scan_workflow_imports(roder_config::WorkflowScanOptions::new(workspace));
    build_file_backed_catalog_with_skills(registry, &workflow.items, skills, options)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use roder_api::discovery::DiscoveryCatalog;
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_api::skills::{SkillActivationState, SkillExposure, SkillSource};

    use super::*;

    #[test]
    #[ignore = "local startup timing benchmark; no network calls"]
    fn benchmark_startup_catalog_generation() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let registry = roder_extension_host::build_default_registry(
            roder_extension_host::DefaultRegistryConfig {
                workspace: Some(workspace.to_path_buf()),
                ..Default::default()
            },
        )
        .unwrap();
        let skills = roder_config::build_skills_registry(workspace.to_path_buf(), None)
            .skills()
            .iter()
            .map(|skill| skill.descriptor.clone())
            .collect::<Vec<_>>();
        let root = std::env::temp_dir().join(format!(
            "roder-discovery-benchmark-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let options =
            DiscoveryCatalogBuildOptions::new(root.join("discovery"), root.join("discovery-state"));
        let mut samples = Vec::new();
        for _ in 0..21 {
            let start = std::time::Instant::now();
            initialize_catalog(&registry, workspace, &skills, &options).unwrap();
            samples.push(start.elapsed().as_secs_f64() * 1_000.0);
        }
        println!(
            "discovery startup: cold={:.2}ms repeated_mean={:.2}ms repeated_max={:.2}ms",
            samples[0],
            samples[1..].iter().sum::<f64>() / 20.0,
            samples[1..].iter().copied().fold(0.0, f64::max),
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_initializes_and_refreshes_catalog_with_tools_and_loaded_skills() {
        let root = std::env::temp_dir().join(format!(
            "roder-discovery-startup-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let skill_path = workspace.join("SKILL.md");
        fs::write(&skill_path, "# Release fixture\nRelease instructions.\n").unwrap();
        let skill = SkillDescriptor {
            id: "release-fixture".to_string(),
            name: "release-fixture".to_string(),
            canonical_path: skill_path.display().to_string(),
            source: SkillSource::Workspace,
            exposure: SkillExposure::Global,
            activation: SkillActivationState::Enabled,
            description: "Release fixture instructions".to_string(),
            short_description: None,
            experimental: false,
            diagnostics: Vec::new(),
            agent_metadata: None,
        };
        let mut builder = ExtensionRegistryBuilder::new();
        builder
            .tool_contributor(roder_tools::builtin_coding_tools_contributor(&workspace).unwrap());
        let registry = builder.build().unwrap();
        let options =
            DiscoveryCatalogBuildOptions::new(root.join("discovery"), root.join("discovery-state"));
        let index = options.catalog_root.join("index.json");
        assert!(!index.exists());

        for _ in 0..2 {
            initialize_catalog(
                &registry,
                &workspace,
                std::slice::from_ref(&skill),
                &options,
            )
            .unwrap();
            let catalog: DiscoveryCatalog =
                serde_json::from_str(&fs::read_to_string(&index).unwrap()).unwrap();
            let items = catalog
                .groups
                .iter()
                .flat_map(|group| &group.items)
                .collect::<Vec<_>>();
            assert!(items.iter().any(|item| item.name == "grep"));
            assert!(items.iter().any(|item| item.name == "release-fixture"));
            fs::write(&index, "stale catalog").unwrap();
        }

        fs::remove_dir_all(root).unwrap();
    }
}
