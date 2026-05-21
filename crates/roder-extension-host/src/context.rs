use std::path::Path;
use std::sync::Arc;

use roder_api::extension::ExtensionRegistryBuilder;

pub(crate) fn install_context_planner(builder: &mut ExtensionRegistryBuilder, workspace: &Path) {
    builder.context_planner(Arc::new(roder_context::EntrypointContextPlanner::new(
        workspace.to_path_buf(),
    )));
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
}
