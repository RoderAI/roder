use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use roder_api::tools::ToolExecutionContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolPathScope {
    /// Resolve relative paths from the workspace root, but allow absolute paths and
    /// `..` segments to address files outside the workspace.
    #[default]
    Global,
    /// Require every resolved path to stay under the workspace root.
    Workspace,
}

impl ToolPathScope {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "global" | "all" | "unrestricted" | "filesystem" | "fs" => Some(Self::Global),
            "workspace" | "workspace-only" | "cwd" | "repo" | "root" => Some(Self::Workspace),
            _ => None,
        }
    }

    pub(crate) fn allows_external_paths(self) -> bool {
        matches!(self, Self::Global)
    }
}

fn expand_home(input: &str) -> anyhow::Result<PathBuf> {
    if input == "~" {
        return home_dir();
    }

    if let Some(rest) = input.strip_prefix("~/") {
        let home = home_dir()?;
        return Ok(home.join(rest));
    }

    Ok(PathBuf::from(input))
}

fn home_dir() -> anyhow::Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow::anyhow!("home directory is not available"))
}

#[derive(Debug, Clone)]
pub(crate) struct Workspace {
    root: PathBuf,
    path_scope: ToolPathScope,
}

impl Workspace {
    #[cfg(test)]
    pub(crate) fn new(root: PathBuf) -> anyhow::Result<Self> {
        Self::new_with_scope(root, ToolPathScope::default())
    }

    pub(crate) fn new_with_scope(root: PathBuf, path_scope: ToolPathScope) -> anyhow::Result<Self> {
        let root = if root.as_os_str().is_empty() {
            std::env::current_dir()?
        } else {
            root
        };
        let root = root
            .canonicalize()
            .with_context(|| format!("workspace root does not exist: {}", root.display()))?;
        Ok(Self { root, path_scope })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn path_scope(&self) -> ToolPathScope {
        self.path_scope
    }

    pub(crate) fn from_context_or_fallback(
        ctx: &ToolExecutionContext,
        fallback: &Workspace,
    ) -> anyhow::Result<Self> {
        let Some(handle) = ctx.handles.workspace.as_ref() else {
            return Ok(fallback.clone());
        };
        let Some(root) = handle.workspace_root() else {
            return Ok(fallback.clone());
        };
        Self::new_with_scope(root, fallback.path_scope)
    }

    pub(crate) fn resolve_existing(&self, input: &str) -> anyhow::Result<PathBuf> {
        let candidate = self.candidate(input)?;
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("path does not exist: {input}"))?;
        self.ensure_allowed(&canonical)?;
        Ok(canonical)
    }

    pub(crate) fn resolve_for_write(&self, input: &str) -> anyhow::Result<PathBuf> {
        let candidate = self.normalize(self.candidate(input)?)?;
        self.ensure_allowed(&candidate)?;
        Ok(candidate)
    }

    pub(crate) fn display(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn candidate(&self, input: &str) -> anyhow::Result<PathBuf> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            bail!("path is required");
        }
        let path = expand_home(trimmed)?;
        if path.is_absolute() {
            Ok(path)
        } else {
            Ok(self.root.join(path))
        }
    }

    fn ensure_allowed(&self, path: &Path) -> anyhow::Result<()> {
        if self.path_scope.allows_external_paths() || path.starts_with(&self.root) {
            return Ok(());
        }
        bail!(
            "path {} is outside workspace {}",
            path.display(),
            self.root.display()
        );
    }

    fn normalize(&self, path: PathBuf) -> anyhow::Result<PathBuf> {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
                Component::RootDir => normalized.push(component.as_os_str()),
                Component::CurDir => {}
                Component::Normal(part) => normalized.push(part),
                Component::ParentDir => {
                    if !normalized.pop() {
                        bail!("path escapes filesystem root");
                    }
                }
            }
        }
        Ok(normalized)
    }
}
