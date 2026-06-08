use std::path::{Component, Path};

pub fn normalize_workspace_path(path: &Path) -> anyhow::Result<String> {
    if path.is_absolute() {
        anyhow::bail!("runner path must be workspace-relative");
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir => anyhow::bail!("runner path cannot escape workspace"),
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("runner path must be workspace-relative")
            }
        }
    }
    if parts.is_empty() {
        anyhow::bail!("runner path cannot be empty");
    }
    Ok(parts.join("/"))
}

pub fn target_manifest_path(path: &Path) -> anyhow::Result<String> {
    normalize_workspace_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_escapes() {
        assert!(normalize_workspace_path(Path::new("../secret")).is_err());
        assert!(normalize_workspace_path(Path::new("/tmp/secret")).is_err());
        assert_eq!(
            normalize_workspace_path(Path::new("./src/lib.rs")).unwrap(),
            "src/lib.rs"
        );
    }
}
