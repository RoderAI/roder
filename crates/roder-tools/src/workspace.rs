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

fn strip_matching_quotes(input: &str) -> &str {
    let mut chars = input.chars();
    let Some(first) = chars.next() else {
        return input;
    };
    let Some(last) = input.chars().last() else {
        return input;
    };
    if input.len() >= 2 && matches!(first, '\'' | '"' | '`') && first == last {
        &input[first.len_utf8()..input.len() - last.len_utf8()]
    } else {
        input
    }
}

fn is_workspace_root_alias(input: &str) -> bool {
    let compact = input
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .map(|ch| if ch == '\\' { '/' } else { ch })
        .collect::<String>();
    compact.starts_with('.') && compact[1..].chars().all(|ch| ch == '/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workdir_resolution_accepts_common_workspace_root_spellings() {
        let root = temp_workspace("roder-workdir-root");
        std::fs::create_dir_all(&root).unwrap();
        let workspace = Workspace::new(root.clone()).unwrap();
        let canonical = root.canonicalize().unwrap();

        for value in [
            "", " ", ".", "./", " . ", " . /", "'.'", "\"./\"", "` . / `",
        ] {
            assert_eq!(
                workspace.resolve_existing_workdir(value).unwrap(),
                canonical
            );
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn remote_workspace_resolves_paths_lexically_without_local_disk() {
        let workspace = Workspace::remote(
            PathBuf::from("/sandbox/workspace"),
            ToolPathScope::Workspace,
        )
        .unwrap();

        // None of these paths exist locally; resolution must not touch local disk.
        assert_eq!(
            workspace.resolve_existing("src/main.rs").unwrap(),
            PathBuf::from("/sandbox/workspace/src/main.rs")
        );
        assert_eq!(
            workspace.resolve_for_write("src/../notes.txt").unwrap(),
            PathBuf::from("/sandbox/workspace/notes.txt")
        );
        assert_eq!(
            workspace.resolve_existing_workdir("./").unwrap(),
            PathBuf::from("/sandbox/workspace")
        );

        let escape = workspace.resolve_for_write("../outside.txt").unwrap_err();
        assert!(escape.to_string().contains("outside workspace"));
        let home = workspace.resolve_existing("~/secrets").unwrap_err();
        assert!(home.to_string().contains("not supported"));
    }

    #[test]
    fn remote_workspace_requires_an_absolute_root() {
        let error =
            Workspace::remote(PathBuf::from("workspace"), ToolPathScope::Workspace).unwrap_err();
        assert!(error.to_string().contains("absolute"));
    }

    #[test]
    fn remote_read_roots_widen_reads_but_not_writes_or_workdir() {
        let workspace = Workspace::remote_with_read_roots(
            PathBuf::from("/var/workspace/session"),
            ToolPathScope::Workspace,
            vec![PathBuf::from("/var/workspace/skills")],
        )
        .unwrap();

        // A read under a declared read root resolves.
        assert_eq!(
            workspace
                .resolve_existing("/var/workspace/skills/global/x/SKILL.md")
                .unwrap(),
            PathBuf::from("/var/workspace/skills/global/x/SKILL.md")
        );
        // Reads under the primary root still resolve.
        assert_eq!(
            workspace.resolve_existing("notes.md").unwrap(),
            PathBuf::from("/var/workspace/session/notes.md")
        );

        // A read outside every declared root is rejected.
        let undeclared = workspace
            .resolve_existing("/var/workspace/documents/a.md")
            .unwrap_err();
        assert!(undeclared.to_string().contains("outside workspace"));

        // Writes stay confined to the primary root even under a read root.
        let write_escape = workspace
            .resolve_for_write("/var/workspace/skills/global/x/out.md")
            .unwrap_err();
        assert!(write_escape.to_string().contains("outside workspace"));

        // The working directory stays confined to the primary root.
        let workdir_escape = workspace
            .resolve_existing_workdir("/var/workspace/skills/global")
            .unwrap_err();
        assert!(workdir_escape.to_string().contains("outside workspace"));
    }

    #[test]
    fn remote_read_roots_must_be_absolute() {
        let error = Workspace::remote_with_read_roots(
            PathBuf::from("/var/workspace/session"),
            ToolPathScope::Workspace,
            vec![PathBuf::from("skills")],
        )
        .unwrap_err();
        assert!(error.to_string().contains("absolute"));
    }

    #[test]
    fn workdir_resolution_still_accepts_normal_relative_directories() {
        let root = temp_workspace("roder-workdir-subdir");
        let subdir = root.join("crates").join("roder-tools");
        std::fs::create_dir_all(&subdir).unwrap();
        let workspace = Workspace::new(root.clone()).unwrap();

        assert_eq!(
            workspace
                .resolve_existing_workdir("'crates/roder-tools'")
                .unwrap(),
            subdir.canonicalize().unwrap()
        );

        let _ = std::fs::remove_dir_all(root);
    }

    fn temp_workspace(prefix: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }
}

pub(crate) fn expand_home(input: &str) -> anyhow::Result<PathBuf> {
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
    /**
     * Extra absolute roots that file reads may resolve under, beyond `root`.
     * Writes and the working directory stay confined to `root`; only the
     * read path (`resolve_existing`) consults these. Populated for remote
     * workspaces that expose read-only mounts outside the writable root.
     */
    read_roots: Vec<PathBuf>,
    /**
     * Remote workspaces scope paths on a runner filesystem: resolution is
     * purely lexical (no canonicalize/existence checks against local disk)
     * and existence errors surface from the runner backend instead.
     */
    remote: bool,
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
        Ok(Self {
            root,
            path_scope,
            read_roots: Vec::new(),
            remote: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn remote(root: PathBuf, path_scope: ToolPathScope) -> anyhow::Result<Self> {
        Self::remote_with_read_roots(root, path_scope, Vec::new())
    }

    pub(crate) fn remote_with_read_roots(
        root: PathBuf,
        path_scope: ToolPathScope,
        read_roots: Vec<PathBuf>,
    ) -> anyhow::Result<Self> {
        if !root.is_absolute() {
            bail!(
                "remote workspace root must be an absolute runner path: {}",
                root.display()
            );
        }
        for read_root in &read_roots {
            if !read_root.is_absolute() {
                bail!(
                    "remote workspace read root must be an absolute runner path: {}",
                    read_root.display()
                );
            }
        }
        Ok(Self {
            root,
            path_scope,
            read_roots,
            remote: true,
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn path_scope(&self) -> ToolPathScope {
        self.path_scope
    }

    pub(crate) fn is_remote(&self) -> bool {
        self.remote
    }

    pub(crate) fn ensure_readable_path(&self, path: &Path) -> anyhow::Result<()> {
        self.ensure_readable(path)
    }

    pub(crate) fn from_context_or_fallback(
        ctx: &ToolExecutionContext,
        fallback: &Workspace,
    ) -> anyhow::Result<Self> {
        if let Some(remote) = ctx.handles.remote_workspace.as_ref() {
            return Self::remote_with_read_roots(
                remote.root.clone(),
                fallback.path_scope,
                remote.read_roots.clone(),
            );
        }
        let Some(handle) = ctx.handles.workspace.as_ref() else {
            return Ok(fallback.clone());
        };
        let Some(root) = handle.workspace_root() else {
            return Ok(fallback.clone());
        };
        Self::new_with_scope(root, fallback.path_scope)
    }

    /// Like `from_context_or_fallback` but rejects remote workspaces for tools that only run locally.
    pub(crate) fn local_from_context_or_fallback(
        ctx: &ToolExecutionContext,
        fallback: &Workspace,
        tool: &str,
    ) -> anyhow::Result<Self> {
        if ctx.handles.remote_workspace.is_some() {
            bail!("{tool} is not supported on a remote runner workspace");
        }
        Self::from_context_or_fallback(ctx, fallback)
    }

    pub(crate) fn resolve_existing(&self, input: &str) -> anyhow::Result<PathBuf> {
        let candidate = self.candidate(input)?;
        if self.remote {
            let normalized = self.normalize(candidate)?;
            self.ensure_readable(&normalized)?;
            return Ok(normalized);
        }
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("path does not exist: {input}"))?;
        self.ensure_readable(&canonical)?;
        Ok(canonical)
    }

    pub(crate) fn resolve_existing_workdir(&self, input: &str) -> anyhow::Result<PathBuf> {
        let trimmed = strip_matching_quotes(input.trim()).trim();
        if trimmed.is_empty() || is_workspace_root_alias(trimmed) {
            return Ok(self.root.clone());
        }
        let resolved = self.resolve_existing(trimmed)?;
        // The working directory is where writes land; keep it under the
        // primary root even though read roots widen `resolve_existing`.
        self.ensure_allowed(&resolved)?;
        Ok(resolved)
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
        // `~` would expand to the local home, not the runner's; reject it.
        if self.remote && (trimmed == "~" || trimmed.starts_with("~/")) {
            bail!("home-relative paths are not supported on a remote runner workspace: {trimmed}");
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

    /**
     * Read access check: the primary root, any declared read root, or an
     * unrestricted scope. Reads may reach declared read roots even though
     * writes (`ensure_allowed`) stay confined to the primary root.
     */
    fn ensure_readable(&self, path: &Path) -> anyhow::Result<()> {
        if self.path_scope.allows_external_paths()
            || path.starts_with(&self.root)
            || self.read_roots.iter().any(|root| path.starts_with(root))
        {
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
