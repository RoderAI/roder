//! The narrow GitHub surface this publisher needs, shared by both transports.

use std::path::Path;

use async_trait::async_trait;
use roder_api::review::ReviewPublishError;
use serde::Deserialize;
use serde_json::Value;

use crate::map::PullRequestFile;

/// A pull request identified well enough to address the REST API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestRef {
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

impl PullRequestRef {
    /// Accepts `owner/repo#123`, `owner/repo/123` and a browser PR URL.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        let value = value
            .strip_prefix("https://github.com/")
            .or_else(|| value.strip_prefix("http://github.com/"))
            .unwrap_or(value);
        let (slug, number) = value
            .split_once('#')
            .or_else(|| value.rsplit_once("/pull/"))
            .or_else(|| value.rsplit_once('/'))?;
        let (owner, repo) = slug.trim_end_matches('/').split_once('/')?;
        let number = number.trim().split('/').next()?.parse::<u64>().ok()?;
        (!owner.is_empty() && !repo.is_empty() && number > 0).then(|| Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number,
        })
    }

    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// `repos/{owner}/{repo}/pulls/{number}{suffix}` — the shape both the `gh`
    /// CLI and the REST API take.
    pub fn api_path(&self, suffix: &str) -> String {
        format!(
            "repos/{}/{}/pulls/{}{suffix}",
            self.owner, self.repo, self.number
        )
    }
}

/// A resolved pull request plus what is needed to anchor comments to it.
#[derive(Debug, Clone)]
pub struct PullRequestContext {
    pub pull_request: PullRequestRef,
    /// `headRefOid`; GitHub rejects a review anchored to any other commit.
    pub head_sha: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SubmittedReview {
    #[serde(default)]
    pub html_url: Option<String>,
}

#[async_trait]
pub trait GithubBackend: Send + Sync {
    async fn resolve_pull_request(
        &self,
        workspace_root: &Path,
        explicit: Option<PullRequestRef>,
    ) -> Result<PullRequestContext, ReviewPublishError>;

    async fn list_files(
        &self,
        pull_request: &PullRequestRef,
    ) -> Result<Vec<PullRequestFile>, ReviewPublishError>;

    async fn submit_review(
        &self,
        pull_request: &PullRequestRef,
        payload: &Value,
    ) -> Result<SubmittedReview, ReviewPublishError>;
}

/// `gh api --paginate` emits one JSON array per page, concatenated, so a plain
/// `from_str` would stop after the first page. Streaming values handles both
/// the single-array and the concatenated case.
pub fn parse_file_pages(stdout: &str) -> Result<Vec<PullRequestFile>, ReviewPublishError> {
    let mut files = Vec::new();
    let stream = serde_json::Deserializer::from_str(stdout).into_iter::<Vec<PullRequestFile>>();
    for page in stream {
        let page = page.map_err(|error| {
            ReviewPublishError::Remote(format!(
                "could not read the pull request file list: {error}"
            ))
        })?;
        files.extend(page);
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_request_references_parse_from_every_shape_a_user_might_type() {
        let expected = PullRequestRef {
            owner: "RoderAI".to_string(),
            repo: "roder".to_string(),
            number: 123,
        };
        for value in [
            "RoderAI/roder#123",
            " RoderAI/roder#123 ",
            "RoderAI/roder/123",
            "https://github.com/RoderAI/roder/pull/123",
            "https://github.com/RoderAI/roder/pull/123/files",
        ] {
            assert_eq!(
                PullRequestRef::parse(value).as_ref(),
                Some(&expected),
                "value: {value}"
            );
        }
        for value in ["RoderAI/roder", "roder#123", "RoderAI/roder#0", ""] {
            assert_eq!(PullRequestRef::parse(value), None, "value: {value}");
        }
        assert_eq!(
            expected.api_path("/files"),
            "repos/RoderAI/roder/pulls/123/files"
        );
    }

    #[test]
    fn concatenated_pages_flatten_into_one_file_list() {
        let stdout = r#"[{"filename":"a.rs","patch":"@@ -1 +1 @@"}]
[{"filename":"b.rs"}]"#;
        let files = parse_file_pages(stdout).expect("parse pages");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].filename, "a.rs");
        assert!(files[1].patch.is_none());
        assert!(parse_file_pages("").expect("empty").is_empty());
        assert!(parse_file_pages("not json").is_err());
    }
}
