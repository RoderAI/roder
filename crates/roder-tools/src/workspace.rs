use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};

#[derive(Debug, Clone)]
pub(crate) struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub(crate) fn new(root: PathBuf) -> anyhow::Result<Self> {
        let root = if root.as_os_str().is_empty() {
            std::env::current_dir()?
        } else {
            root
        };
        let root = root
            .canonicalize()
            .with_context(|| format!("workspace root does not exist: {}", root.display()))?;
        Ok(Self { root })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn resolve_existing(&self, input: &str) -> anyhow::Result<PathBuf> {
        let candidate = self.candidate(input)?;
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("path does not exist: {input}"))?;
        self.ensure_inside(&canonical)?;
        Ok(canonical)
    }

    pub(crate) fn resolve_for_write(&self, input: &str) -> anyhow::Result<PathBuf> {
        let candidate = self.normalize(self.candidate(input)?)?;
        self.ensure_inside(&candidate)?;
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
        let path = PathBuf::from(trimmed);
        if path.is_absolute() {
            Ok(path)
        } else {
            Ok(self.root.join(path))
        }
    }

    fn ensure_inside(&self, path: &Path) -> anyhow::Result<()> {
        if !path.starts_with(&self.root) {
            bail!(
                "path {} is outside workspace {}",
                path.display(),
                self.root.display()
            );
        }
        Ok(())
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
                        bail!("path escapes workspace");
                    }
                }
            }
        }
        Ok(normalized)
    }
}
