//! `gh` CLI backend.
//!
//! Preferred when the binary is installed because it reuses whatever
//! credentials `gh auth` already holds, so nothing has to be configured for the
//! common case.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use roder_api::review::ReviewPublishError;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::backend::{
    GithubBackend, PullRequestContext, PullRequestRef, SubmittedReview, parse_file_pages,
};
use crate::config::GithubReviewConfig;
use crate::map::PullRequestFile;

/// Fields of `gh pr view`; `url` is what the owner/repo slug is recovered from.
const PR_VIEW_FIELDS: &str = "number,headRefOid,url,baseRefName";

const MAX_STDERR: usize = 600;

pub struct GhCliBackend {
    gh_bin: PathBuf,
}

impl GhCliBackend {
    pub fn new(config: &GithubReviewConfig) -> Self {
        Self {
            gh_bin: config.gh_bin.clone(),
        }
    }

    /// Whether the configured binary can actually be executed.
    pub fn is_installed(config: &GithubReviewConfig) -> bool {
        let gh_bin = &config.gh_bin;
        if gh_bin.components().count() > 1 {
            return gh_bin.is_file();
        }
        which::which(gh_bin).is_ok()
    }

    async fn run(
        &self,
        cwd: Option<&Path>,
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> Result<String, ReviewPublishError> {
        let mut command = Command::new(&self.gh_bin);
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        command.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });

        let mut child = command.spawn().map_err(|error| {
            ReviewPublishError::NotConfigured(format!(
                "could not run {} ({error}); install the GitHub CLI or set mode = \"http\"",
                self.gh_bin.display()
            ))
        })?;
        if let Some(stdin) = stdin
            && let Some(mut pipe) = child.stdin.take()
        {
            pipe.write_all(stdin)
                .await
                .map_err(|error| ReviewPublishError::Other(error.into()))?;
            pipe.shutdown()
                .await
                .map_err(|error| ReviewPublishError::Other(error.into()))?;
        }
        let output = child
            .wait_with_output()
            .await
            .map_err(|error| ReviewPublishError::Other(error.into()))?;
        if !output.status.success() {
            return Err(ReviewPublishError::Remote(format!(
                "gh {} failed: {}",
                args.join(" "),
                truncate_stderr(&output.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

fn truncate_stderr(stderr: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(stderr).trim().to_string();
    if text.len() > MAX_STDERR {
        text.truncate(MAX_STDERR);
        text.push('…');
    }
    if text.is_empty() {
        text.push_str("no error output");
    }
    text
}

#[async_trait]
impl GithubBackend for GhCliBackend {
    async fn resolve_pull_request(
        &self,
        workspace_root: &Path,
        explicit: Option<PullRequestRef>,
    ) -> Result<PullRequestContext, ReviewPublishError> {
        // Without an explicit reference `gh` infers the pull request from the
        // checked-out branch, which is the common case.
        let slug = explicit.as_ref().map(PullRequestRef::slug);
        let number = explicit.as_ref().map(|pr| pr.number.to_string());
        let mut args: Vec<&str> = vec!["pr", "view"];
        if let (Some(slug), Some(number)) = (slug.as_deref(), number.as_deref()) {
            args.extend([number, "--repo", slug]);
        }
        args.extend(["--json", PR_VIEW_FIELDS]);

        let stdout = self.run(Some(workspace_root), &args, None).await?;
        let view: Value = serde_json::from_str(&stdout).map_err(|error| {
            ReviewPublishError::Remote(format!("could not read `gh pr view` output: {error}"))
        })?;
        parse_pr_view(&view, explicit)
    }

    async fn list_files(
        &self,
        pull_request: &PullRequestRef,
    ) -> Result<Vec<PullRequestFile>, ReviewPublishError> {
        let path = pull_request.api_path("/files");
        let stdout = self.run(None, &["api", &path, "--paginate"], None).await?;
        parse_file_pages(&stdout)
    }

    async fn submit_review(
        &self,
        pull_request: &PullRequestRef,
        payload: &Value,
    ) -> Result<SubmittedReview, ReviewPublishError> {
        let path = pull_request.api_path("/reviews");
        let body = serde_json::to_vec(payload).map_err(|error| {
            ReviewPublishError::Other(anyhow::anyhow!("serializing the review payload: {error}"))
        })?;
        let stdout = self
            .run(
                None,
                &["api", "--method", "POST", &path, "--input", "-"],
                Some(&body),
            )
            .await?;
        serde_json::from_str(&stdout).map_err(|error| {
            ReviewPublishError::Remote(format!("could not read the submitted review: {error}"))
        })
    }
}

/// Turns `gh pr view --json number,headRefOid,url` output into a context. The
/// owner/repo slug is not one of the JSON fields, so it comes from `url`.
pub fn parse_pr_view(
    view: &Value,
    explicit: Option<PullRequestRef>,
) -> Result<PullRequestContext, ReviewPublishError> {
    let url = view.get("url").and_then(Value::as_str);
    let head_sha = view
        .get("headRefOid")
        .and_then(Value::as_str)
        .filter(|sha| !sha.is_empty())
        .ok_or_else(|| {
            ReviewPublishError::DestinationUnresolved(
                "`gh pr view` returned no headRefOid for this branch".to_string(),
            )
        })?;
    let pull_request = explicit
        .or_else(|| url.and_then(PullRequestRef::parse))
        .ok_or_else(|| {
            ReviewPublishError::DestinationUnresolved(
                "could not work out which pull request to review; pass destination.target as owner/repo#number"
                    .to_string(),
            )
        })?;
    Ok(PullRequestContext {
        pull_request,
        head_sha: head_sha.to_string(),
        url: url.map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pr_view_output_yields_the_slug_from_the_url() {
        let view = json!({
            "number": 123,
            "headRefOid": "abc123",
            "url": "https://github.com/RoderAI/roder/pull/123",
            "baseRefName": "master",
        });
        let context = parse_pr_view(&view, None).expect("resolve");
        assert_eq!(context.pull_request.slug(), "RoderAI/roder");
        assert_eq!(context.pull_request.number, 123);
        assert_eq!(context.head_sha, "abc123");
    }

    #[test]
    fn pr_view_without_a_head_sha_is_a_destination_error() {
        let error = parse_pr_view(&json!({ "url": "https://github.com/o/r/pull/1" }), None)
            .expect_err("missing headRefOid");
        assert!(matches!(
            error,
            ReviewPublishError::DestinationUnresolved(_)
        ));
    }

    #[test]
    fn an_explicit_reference_wins_over_the_url() {
        let explicit = PullRequestRef::parse("other/repo#7").expect("parse");
        let view = json!({
            "headRefOid": "abc123",
            "url": "https://github.com/RoderAI/roder/pull/123",
        });
        let context = parse_pr_view(&view, Some(explicit)).expect("resolve");
        assert_eq!(context.pull_request.slug(), "other/repo");
        assert_eq!(context.pull_request.number, 7);
    }
}
