use std::path::{Component, Path};

use anyhow::Context;
use serde::Deserialize;

use crate::client::SpritesClient;

/// One entry from `GET /v1/sprites/{name}/fs/list`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SpriteFsEntry {
    pub name: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default, alias = "is_dir", alias = "isDir")]
    pub dir: bool,
    #[serde(default)]
    pub mode: Option<String>,
}

/**
 * Richer Sprites filesystem operations beyond the canonical runner
 * read/write contract: list, delete, rename, copy, and chmod under the
 * per-sprite `fs` endpoints. All paths are workspace-relative and validated
 * against traversal before they reach the wire.
 */
impl SpritesClient {
    pub async fn list_dir(
        &self,
        sprite_name: &str,
        path: &Path,
    ) -> anyhow::Result<Vec<SpriteFsEntry>> {
        let path = normalize_workspace_path(path)?;
        let response = self
            .http
            .get(self.fs_url(
                sprite_name,
                "/fs/list",
                &path,
                &[("workingDir", &self.config.working_dir)],
            ))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("list sprites runner directory")?;
        self.decode_json(response, "list directory").await
    }

    pub async fn delete_path(&self, sprite_name: &str, path: &Path) -> anyhow::Result<()> {
        let path = normalize_workspace_path(path)?;
        let response = self
            .http
            .delete(self.fs_url(
                sprite_name,
                "/fs/delete",
                &path,
                &[("workingDir", &self.config.working_dir)],
            ))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("delete sprites runner path")?;
        self.decode_empty(response, "delete path").await
    }

    pub async fn rename_path(
        &self,
        sprite_name: &str,
        from: &Path,
        to: &Path,
    ) -> anyhow::Result<()> {
        let from = normalize_workspace_path(from)?;
        let to = normalize_workspace_path(to)?;
        let response = self
            .http
            .post(self.fs_url(
                sprite_name,
                "/fs/rename",
                &from,
                &[
                    ("to", to.as_str()),
                    ("workingDir", &self.config.working_dir),
                ],
            ))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("rename sprites runner path")?;
        self.decode_empty(response, "rename path").await
    }

    pub async fn copy_path(
        &self,
        sprite_name: &str,
        from: &Path,
        to: &Path,
    ) -> anyhow::Result<()> {
        let from = normalize_workspace_path(from)?;
        let to = normalize_workspace_path(to)?;
        let response = self
            .http
            .post(self.fs_url(
                sprite_name,
                "/fs/copy",
                &from,
                &[
                    ("to", to.as_str()),
                    ("workingDir", &self.config.working_dir),
                ],
            ))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("copy sprites runner path")?;
        self.decode_empty(response, "copy path").await
    }

    pub async fn chmod_path(
        &self,
        sprite_name: &str,
        path: &Path,
        mode: &str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !mode.is_empty() && mode.len() <= 4 && mode.chars().all(|ch| ('0'..='7').contains(&ch)),
            "chmod mode must be an octal string like 755, got {mode:?}"
        );
        let path = normalize_workspace_path(path)?;
        let response = self
            .http
            .post(self.fs_url(
                sprite_name,
                "/fs/chmod",
                &path,
                &[("mode", mode), ("workingDir", &self.config.working_dir)],
            ))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("chmod sprites runner path")?;
        self.decode_empty(response, "chmod path").await
    }
}

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
