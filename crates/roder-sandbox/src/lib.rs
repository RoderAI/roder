use std::path::{Path, PathBuf};

use roder_api::tools::ScopedWorkspaceHandle;

pub mod manifest;
pub mod mount;
pub mod process;
pub mod session;
pub mod snapshot;

pub use manifest::validate_manifest;
pub use snapshot::{validate_snapshot_export, validate_snapshot_ref};

#[derive(Debug, Clone)]
pub struct ScopedFilesystem {
    workspace: PathBuf,
}

impl ScopedFilesystem {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn resolve(&self, path: impl AsRef<Path>) -> anyhow::Result<PathBuf> {
        let candidate = if path.as_ref().is_absolute() {
            path.as_ref().to_path_buf()
        } else {
            self.workspace.join(path)
        };
        let normalized = normalize_path(&candidate);
        if !normalized.starts_with(&self.workspace) {
            anyhow::bail!("path escapes scoped workspace: {}", normalized.display());
        }
        Ok(normalized)
    }
}

impl ScopedWorkspaceHandle for ScopedFilesystem {
    fn workspace_root(&self) -> Option<PathBuf> {
        Some(self.workspace.clone())
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_workspace_escape() {
        let fs = ScopedFilesystem::new("/tmp/workspace");
        assert!(fs.resolve("../secret").is_err());
    }
}
