use std::path::Path;
use std::sync::Arc;

use roder_api::extension::ExtensionRegistryBuilder;

pub(crate) fn install_context_planner(builder: &mut ExtensionRegistryBuilder, workspace: &Path) {
    builder.context_planner(Arc::new(roder_context::EntrypointContextPlanner::new(
        workspace.to_path_buf(),
    )));
}

pub(crate) fn install_code_index_context_provider(
    builder: &mut ExtensionRegistryBuilder,
    workspace: &Path,
    roder_home: &Path,
) -> anyhow::Result<()> {
    let store_path =
        roder_code_index::sqlite::default_store_path(roder_home.join("code-index"), workspace);
    let store = roder_code_index::sqlite::SqliteCodeIndexStore::open(store_path)?;
    builder.context_provider(Arc::new(roder_context::CodeIndexContextProvider::new(
        workspace.to_path_buf(),
        Arc::new(store),
    )));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_planner_registration_installs_entrypoint_planner() {
        let mut builder = ExtensionRegistryBuilder::new();
        install_context_planner(&mut builder, Path::new("."));
        let registry = builder.build().unwrap();

        assert_eq!(registry.context_planners.len(), 1);
        assert_eq!(
            registry.context_planners[0].id(),
            "entrypoint-context-planner"
        );
    }

    #[test]
    fn code_index_context_provider_registration_installs_provider() {
        let root = tempdir("code-index-provider-registration");
        let mut builder = ExtensionRegistryBuilder::new();
        install_code_index_context_provider(&mut builder, &root, &root.join("home")).unwrap();
        let registry = builder.build().unwrap();

        assert_eq!(registry.context_providers.len(), 1);
        assert_eq!(
            registry.context_providers[0].id(),
            "code-index-context-provider"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    fn tempdir(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "roder-extension-host-{name}-{}",
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
