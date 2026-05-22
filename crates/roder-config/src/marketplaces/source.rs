use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use roder_api::marketplace::{MarketplaceDescriptor, MarketplaceKind, MarketplaceSource};

pub fn resolve_marketplace_source(marketplace: &MarketplaceDescriptor) -> anyhow::Result<PathBuf> {
    resolve_source(&marketplace.id, &marketplace.source)
}

pub fn resolve_source(marketplace_id: &str, source: &MarketplaceSource) -> anyhow::Result<PathBuf> {
    match source {
        MarketplaceSource::LocalPath { path } => Ok(PathBuf::from(path)),
        MarketplaceSource::Github { repo, ref_name, .. } => {
            resolve_github_source(marketplace_id, repo, ref_name.as_deref())
        }
        MarketplaceSource::Git { url, ref_name, .. } => {
            resolve_git_source(marketplace_id, url, ref_name.as_deref())
        }
        MarketplaceSource::HttpJson { url } => resolve_http_json_source(marketplace_id, url),
    }
}

pub fn marketplace_json_path(root: &Path, marketplace: &MarketplaceDescriptor) -> Option<PathBuf> {
    match &marketplace.source {
        MarketplaceSource::LocalPath { .. } => default_catalog_path(root, marketplace.kind.clone()),
        MarketplaceSource::Github { catalog_path, .. } => catalog_path
            .as_ref()
            .map(|p| root.join(p))
            .or_else(|| default_catalog_path(root, marketplace.kind.clone())),
        MarketplaceSource::Git { catalog_path, .. } => catalog_path
            .as_ref()
            .map(|p| root.join(p))
            .or_else(|| default_catalog_path(root, marketplace.kind.clone())),
        MarketplaceSource::HttpJson { .. } => Some(root.join("marketplace.json")),
    }
}

pub fn plugin_root(root: &Path, marketplace: &MarketplaceDescriptor) -> PathBuf {
    match &marketplace.source {
        MarketplaceSource::Github {
            plugin_root: Some(plugin_root),
            ..
        } => root.join(plugin_root),
        _ if marketplace.kind == MarketplaceKind::Codex => root.join("plugins"),
        _ => root.to_path_buf(),
    }
}

pub fn infer_kind_from_root(root: &Path) -> MarketplaceKind {
    if root
        .join(".claude-plugin")
        .join("marketplace.json")
        .exists()
    {
        MarketplaceKind::Claude
    } else if root
        .join(".cursor-plugin")
        .join("marketplace.json")
        .exists()
    {
        MarketplaceKind::Cursor
    } else if root.join(".roder-plugin").join("marketplace.json").exists() {
        MarketplaceKind::Roder
    } else if root.join("plugins").is_dir()
        || root.join(".codex-plugin").join("plugin.json").exists()
    {
        MarketplaceKind::Codex
    } else {
        MarketplaceKind::Custom
    }
}

pub fn infer_kind_from_source(source: &MarketplaceSource) -> MarketplaceKind {
    match source {
        MarketplaceSource::LocalPath { path } => infer_kind_from_root(Path::new(path)),
        MarketplaceSource::Github {
            repo,
            catalog_path,
            plugin_root,
            ..
        } => infer_kind_from_remote_hints(repo, catalog_path.as_deref(), plugin_root.as_deref()),
        MarketplaceSource::Git {
            url, catalog_path, ..
        } => infer_kind_from_remote_hints(url, catalog_path.as_deref(), None),
        MarketplaceSource::HttpJson { url } => infer_kind_from_remote_hints(url, None, None),
    }
}

fn infer_kind_from_remote_hints(
    source_name: &str,
    catalog_path: Option<&str>,
    plugin_root: Option<&str>,
) -> MarketplaceKind {
    let hint = format!(
        "{} {} {}",
        source_name.to_ascii_lowercase(),
        catalog_path.unwrap_or_default().to_ascii_lowercase(),
        plugin_root.unwrap_or_default().to_ascii_lowercase()
    );
    if hint.contains(".claude-plugin") || hint.contains("claude") || hint.contains("anthropic") {
        MarketplaceKind::Claude
    } else if hint.contains(".cursor-plugin") || hint.contains("cursor") {
        MarketplaceKind::Cursor
    } else if hint.contains(".codex-plugin")
        || hint.contains("codex")
        || hint.contains("openai/plugins")
        || plugin_root.is_some()
    {
        MarketplaceKind::Codex
    } else {
        MarketplaceKind::Custom
    }
}

fn default_catalog_path(root: &Path, kind: MarketplaceKind) -> Option<PathBuf> {
    match kind {
        MarketplaceKind::Claude => Some(root.join(".claude-plugin").join("marketplace.json")),
        MarketplaceKind::Cursor => Some(root.join(".cursor-plugin").join("marketplace.json")),
        MarketplaceKind::Roder | MarketplaceKind::Custom => {
            Some(root.join(".roder-plugin").join("marketplace.json"))
        }
        MarketplaceKind::Codex => None,
    }
}

fn marketplace_source_cache_dir() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("RODER_MARKETPLACE_SOURCE_CACHE_DIR") {
        return Ok(PathBuf::from(path));
    }
    Ok(crate::config_dir().join("marketplaces").join("cache"))
}

fn resolve_github_source(
    marketplace_id: &str,
    repo: &str,
    ref_name: Option<&str>,
) -> anyhow::Result<PathBuf> {
    if let Some(fixture_dir) = std::env::var_os("RODER_MARKETPLACE_GITHUB_FIXTURE_DIR") {
        let root = PathBuf::from(fixture_dir).join(repo);
        if root.exists() {
            return Ok(root);
        }
    }
    let url = format!("https://github.com/{repo}.git");
    resolve_git_source(marketplace_id, &url, ref_name)
}

fn resolve_git_source(
    marketplace_id: &str,
    url: &str,
    ref_name: Option<&str>,
) -> anyhow::Result<PathBuf> {
    let root = marketplace_source_cache_dir()?
        .join(marketplace_id)
        .join("git");
    if root.join(".git").exists() {
        run_git([
            "-C",
            root_string(&root).as_str(),
            "fetch",
            "--all",
            "--tags",
        ])?;
    } else {
        if let Some(parent) = root.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create marketplace source cache {}", parent.display()))?;
        }
        run_git(["clone", "--depth", "1", url, root_string(&root).as_str()])?;
    }
    if let Some(ref_name) = ref_name {
        run_git(["-C", root_string(&root).as_str(), "checkout", ref_name])?;
    }
    Ok(root)
}

fn resolve_http_json_source(marketplace_id: &str, url: &str) -> anyhow::Result<PathBuf> {
    let root = marketplace_source_cache_dir()?
        .join(marketplace_id)
        .join("http-json");
    std::fs::create_dir_all(&root)
        .with_context(|| format!("create marketplace HTTP cache {}", root.display()))?;
    let text = if let Some(path) = url.strip_prefix("file://") {
        std::fs::read_to_string(path).with_context(|| format!("read marketplace JSON {path}"))?
    } else {
        reqwest::blocking::Client::builder()
            .user_agent("roder-marketplace/0.1")
            .build()?
            .get(url)
            .send()
            .with_context(|| format!("fetch marketplace JSON {url}"))?
            .error_for_status()
            .with_context(|| format!("fetch marketplace JSON {url}"))?
            .text()
            .with_context(|| format!("read marketplace JSON response {url}"))?
    };
    let _: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("parse marketplace JSON fetched from {url}"))?;
    std::fs::write(root.join("marketplace.json"), text)
        .with_context(|| format!("write marketplace HTTP cache {}", root.display()))?;
    Ok(root)
}

fn run_git<const N: usize>(args: [&str; N]) -> anyhow::Result<()> {
    let output = Command::new("git")
        .args(args)
        .output()
        .context("run git for marketplace source")?;
    if !output.status.success() {
        anyhow::bail!(
            "git marketplace source command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn root_string(root: &Path) -> String {
    root.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use uuid::Uuid;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn infers_kind_from_local_catalog_markers() {
        let root = tempdir("infers_kind_from_local_catalog_markers");
        std::fs::create_dir_all(root.join(".cursor-plugin")).unwrap();
        std::fs::write(
            root.join(".cursor-plugin").join("marketplace.json"),
            "{\"plugins\":[]}",
        )
        .unwrap();

        assert_eq!(infer_kind_from_root(&root), MarketplaceKind::Cursor);
        assert_eq!(
            infer_kind_from_source(&MarketplaceSource::LocalPath {
                path: root.display().to_string()
            }),
            MarketplaceKind::Cursor
        );
    }

    #[test]
    fn resolves_http_json_sources_into_cache() {
        let _guard = ENV_LOCK.lock().unwrap();
        let root = tempdir("resolves_http_json_sources_into_cache");
        let cache = root.join("cache");
        let catalog = root.join("marketplace.json");
        std::fs::write(&catalog, "{\"plugins\":[]}").unwrap();
        unsafe {
            std::env::set_var("RODER_MARKETPLACE_SOURCE_CACHE_DIR", &cache);
        }

        let descriptor = descriptor(
            "http-local",
            MarketplaceKind::Custom,
            MarketplaceSource::HttpJson {
                url: format!("file://{}", catalog.display()),
            },
        );
        let resolved = resolve_marketplace_source(&descriptor).unwrap();

        assert_eq!(resolved, cache.join("http-local").join("http-json"));
        assert!(resolved.join("marketplace.json").exists());
        unsafe {
            std::env::remove_var("RODER_MARKETPLACE_SOURCE_CACHE_DIR");
        }
    }

    #[test]
    fn resolves_github_sources_from_fixture_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        let root = tempdir("resolves_github_sources_from_fixture_dir");
        let repo = root.join("owner").join("plugins");
        std::fs::create_dir_all(repo.join(".claude-plugin")).unwrap();
        std::fs::write(
            repo.join(".claude-plugin").join("marketplace.json"),
            "{\"plugins\":[]}",
        )
        .unwrap();
        unsafe {
            std::env::set_var("RODER_MARKETPLACE_GITHUB_FIXTURE_DIR", &root);
        }

        let descriptor = descriptor(
            "github-local",
            MarketplaceKind::Claude,
            MarketplaceSource::Github {
                repo: "owner/plugins".to_string(),
                ref_name: None,
                catalog_path: None,
                plugin_root: None,
            },
        );
        let resolved = resolve_marketplace_source(&descriptor).unwrap();

        assert_eq!(resolved, repo);
        unsafe {
            std::env::remove_var("RODER_MARKETPLACE_GITHUB_FIXTURE_DIR");
        }
    }

    #[test]
    fn resolves_git_sources_into_cache() {
        let _guard = ENV_LOCK.lock().unwrap();
        let root = tempdir("resolves_git_sources_into_cache");
        let repo = root.join("repo");
        let cache = root.join("cache");
        std::fs::create_dir_all(repo.join(".cursor-plugin")).unwrap();
        std::fs::write(
            repo.join(".cursor-plugin").join("marketplace.json"),
            "{\"plugins\":[]}",
        )
        .unwrap();
        git(&repo, ["init"]);
        git(&repo, ["config", "user.email", "roder@example.test"]);
        git(&repo, ["config", "user.name", "Roder Test"]);
        git(&repo, ["add", "."]);
        git(&repo, ["commit", "-m", "marketplace"]);
        unsafe {
            std::env::set_var("RODER_MARKETPLACE_SOURCE_CACHE_DIR", &cache);
        }

        let descriptor = descriptor(
            "git-local",
            MarketplaceKind::Cursor,
            MarketplaceSource::Git {
                url: repo.display().to_string(),
                ref_name: None,
                catalog_path: None,
            },
        );
        let resolved = resolve_marketplace_source(&descriptor).unwrap();

        assert_eq!(resolved, cache.join("git-local").join("git"));
        assert!(
            resolved
                .join(".cursor-plugin")
                .join("marketplace.json")
                .exists()
        );
        unsafe {
            std::env::remove_var("RODER_MARKETPLACE_SOURCE_CACHE_DIR");
        }
    }

    fn descriptor(
        id: &str,
        kind: MarketplaceKind,
        source: MarketplaceSource,
    ) -> MarketplaceDescriptor {
        MarketplaceDescriptor {
            id: id.to_string(),
            kind,
            display_name: id.to_string(),
            source,
            homepage: None,
            owner_name: None,
            owner_email: None,
            description: None,
            is_default: false,
            enabled: true,
            state: roder_api::marketplace::MarketplaceState::Installed,
            last_refreshed_at: None,
            content_hash: None,
        }
    }

    fn tempdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("roder-marketplaces-{name}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git<const N: usize>(repo: &Path, args: [&str; N]) {
        let output = Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
