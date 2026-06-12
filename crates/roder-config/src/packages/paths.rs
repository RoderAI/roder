//! Filesystem layout for package stores and settings files.
//!
//! All public package operations take a [`PackagePaths`] so tests can point
//! everything at temp directories without mutating process environment.

use std::path::{Path, PathBuf};

use roder_api::packages::{PACKAGES_SETTINGS_FILE, PackageScope, PackageSource};

/// `std::env::join_paths`-style list of package roots loaded for the current
/// run only (ephemeral `roder -e <path>` semantics).
pub const RODER_EPHEMERAL_PACKAGES_ENV: &str = "RODER_EPHEMERAL_PACKAGES";
/// When `"1"`, ephemeral package process extensions count as approved.
pub const RODER_EPHEMERAL_APPROVE_ENV: &str = "RODER_EPHEMERAL_APPROVE";

#[derive(Debug, Clone, Default)]
pub struct PackagePaths {
    /// User scope directory (normally `~/.roder`); settings live at
    /// `<user_dir>/packages.json`, stores under `<user_dir>/packages/`.
    pub user_dir: PathBuf,
    /// Workspace root for project scope; settings live at
    /// `<workspace>/.roder/packages.json`, stores under
    /// `<workspace>/.roder/packages/`.
    pub workspace: Option<PathBuf>,
    /// Extra package roots activated for this run only. Never persisted.
    pub ephemeral_roots: Vec<PathBuf>,
    /// Whether ephemeral packages may launch process extensions.
    pub ephemeral_extensions_approved: bool,
}

impl PackagePaths {
    /// Standard paths: user scope under [`crate::config_dir`], project scope
    /// under `<workspace>/.roder`, ephemeral roots from
    /// [`RODER_EPHEMERAL_PACKAGES_ENV`].
    pub fn standard(workspace: Option<&Path>) -> Self {
        let ephemeral_roots = std::env::var_os(RODER_EPHEMERAL_PACKAGES_ENV)
            .map(|joined| std::env::split_paths(&joined).collect())
            .unwrap_or_default();
        let ephemeral_extensions_approved =
            std::env::var(RODER_EPHEMERAL_APPROVE_ENV).is_ok_and(|value| value.trim() == "1");
        Self {
            user_dir: crate::config_dir(),
            workspace: workspace.map(Path::to_path_buf),
            ephemeral_roots,
            ephemeral_extensions_approved,
        }
    }

    /// Directory holding a scope's settings file and `packages/` store.
    pub fn scope_dir(&self, scope: PackageScope) -> anyhow::Result<PathBuf> {
        match scope {
            PackageScope::User => Ok(self.user_dir.clone()),
            PackageScope::Project => self
                .workspace
                .as_ref()
                .map(|workspace| workspace.join(".roder"))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "project-scope packages need a workspace; open a workspace or install \
                         with user scope"
                    )
                }),
        }
    }

    /// `packages.json` path for a scope.
    pub fn settings_path(&self, scope: PackageScope) -> anyhow::Result<PathBuf> {
        Ok(self.scope_dir(scope)?.join(PACKAGES_SETTINGS_FILE))
    }

    /// Root directory holding materialized package trees for a scope.
    pub fn store_root(&self, scope: PackageScope) -> anyhow::Result<PathBuf> {
        Ok(self.scope_dir(scope)?.join("packages"))
    }

    /// Materialized install directory for a fetched source. `None` for local
    /// paths, which load in place and are never copied into a store.
    pub fn store_path(
        &self,
        scope: PackageScope,
        source: &PackageSource,
    ) -> anyhow::Result<Option<PathBuf>> {
        let root = self.store_root(scope)?;
        Ok(match source {
            PackageSource::Npm { name, .. } => {
                Some(root.join("npm").join(npm_store_dir_name(name)))
            }
            PackageSource::Git { url, .. } => {
                let (host, path) = git_store_components(url);
                Some(root.join("git").join(host).join(path))
            }
            PackageSource::LocalPath { .. } => None,
        })
    }
}

/// npm store directory name: the package name with `/` replaced by `__`
/// (`@scope/pkg` -> `@scope__pkg`).
pub fn npm_store_dir_name(name: &str) -> String {
    name.replace('/', "__")
}

/// Git store components: `git/<sanitized host>/<sanitized path>`.
pub fn git_store_components(url: &str) -> (String, String) {
    let trimmed = url.trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let (host, path) = if let Some((_scheme, rest)) = trimmed.split_once("://") {
        let (authority, path) = match rest.split_once('/') {
            Some((authority, path)) => (authority, path),
            None => (rest, ""),
        };
        let host = authority
            .rsplit_once('@')
            .map(|(_, host)| host)
            .unwrap_or(authority);
        let host = host.split_once(':').map(|(host, _)| host).unwrap_or(host);
        (host, path)
    } else if let Some((user_host, path)) = trimmed.split_once(':') {
        // scp form: git@host:user/repo
        let host = user_host
            .rsplit_once('@')
            .map(|(_, host)| host)
            .unwrap_or(user_host);
        (host, path)
    } else {
        ("", trimmed)
    };
    let host = sanitize_store_component(host, "local");
    let path = sanitize_store_component(&path.replace('/', "__"), "repo");
    (host, path)
}

fn sanitize_store_component(value: &str, fallback: &str) -> String {
    let mut sanitized: String = value
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    while sanitized.starts_with(['-', '.']) {
        sanitized.remove(0);
    }
    while sanitized.ends_with(['-', '.']) {
        sanitized.pop();
    }
    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_store_dir_name_replaces_slashes() {
        assert_eq!(npm_store_dir_name("@foo/pkg"), "@foo__pkg");
        assert_eq!(npm_store_dir_name("plain"), "plain");
    }

    #[test]
    fn git_store_components_cover_url_forms() {
        assert_eq!(
            git_store_components("https://github.com/user/repo.git"),
            ("github.com".to_string(), "user__repo".to_string())
        );
        assert_eq!(
            git_store_components("ssh://git@github.com/user/repo"),
            ("github.com".to_string(), "user__repo".to_string())
        );
        assert_eq!(
            git_store_components("git@github.com:user/repo.git"),
            ("github.com".to_string(), "user__repo".to_string())
        );
        let (host, path) = git_store_components("file:///tmp/pkg repo");
        assert_eq!(host, "local");
        assert_eq!(path, "tmp__pkg-repo");
    }
}
