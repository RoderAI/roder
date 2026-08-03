//! The little bit of git this publisher needs.
//!
//! `roder-ext-git`'s `GitRepo` is crate-private, so the few facts required to
//! anchor comments (repo root, remote slug, current branch) are read by
//! shelling out here rather than widening that API.

use std::path::{Path, PathBuf};

use roder_api::review::ReviewPublishError;
use tokio::process::Command;

async fn git(workspace_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// Root the finding paths are relativized against. Fails loudly: publishing
/// with the wrong root would silently produce paths GitHub rejects.
pub async fn repo_root(workspace_root: &Path) -> Result<PathBuf, ReviewPublishError> {
    git(workspace_root, &["rev-parse", "--show-toplevel"])
        .await
        .map(PathBuf::from)
        .ok_or_else(|| {
            ReviewPublishError::DestinationUnresolved(format!(
                "{} is not inside a git repository",
                workspace_root.display()
            ))
        })
}

pub async fn current_branch(workspace_root: &Path) -> Option<String> {
    git(workspace_root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .filter(|branch| branch != "HEAD")
}

pub async fn remote_slug(workspace_root: &Path) -> Option<(String, String)> {
    let url = git(workspace_root, &["remote", "get-url", "origin"]).await?;
    parse_remote_slug(&url)
}

/// Extracts `(owner, repo)` from any of the URL shapes git remotes use.
pub fn parse_remote_slug(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git://github.com/"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = rest.split_once('/')?;
    let repo = repo.split('/').next()?;
    (!owner.is_empty() && !repo.is_empty()).then(|| (owner.to_string(), repo.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_urls_reduce_to_owner_and_repo() {
        let cases: &[(&str, Option<(&str, &str)>)] = &[
            (
                "git@github.com:RoderAI/roder.git",
                Some(("RoderAI", "roder")),
            ),
            ("git@github.com:RoderAI/roder", Some(("RoderAI", "roder"))),
            (
                "https://github.com/RoderAI/roder.git\n",
                Some(("RoderAI", "roder")),
            ),
            (
                "ssh://git@github.com/RoderAI/roder.git",
                Some(("RoderAI", "roder")),
            ),
            (
                "https://github.com/RoderAI/roder/",
                Some(("RoderAI", "roder")),
            ),
            ("https://gitlab.com/RoderAI/roder.git", None),
            ("https://github.com/RoderAI", None),
        ];
        for (url, expected) in cases {
            let expected = expected.map(|(owner, repo)| (owner.to_string(), repo.to_string()));
            assert_eq!(parse_remote_slug(url), expected, "url: {url}");
        }
    }
}
