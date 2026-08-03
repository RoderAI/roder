//! Direct REST backend, used when the `gh` CLI is unavailable.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use reqwest::{Client, Response, StatusCode};
use roder_api::review::ReviewPublishError;
use serde_json::Value;

use crate::backend::{GithubBackend, PullRequestContext, PullRequestRef, SubmittedReview};
use crate::config::GithubReviewConfig;
use crate::git;
use crate::map::PullRequestFile;

const API_VERSION_HEADER: &str = "x-github-api-version";
const API_VERSION: &str = "2022-11-28";
const USER_AGENT_VALUE: &str = "roder-ext-github-review/0.1";
const PER_PAGE: usize = 100;
/// GitHub caps `pulls/{n}/files` at 3000 entries; stop well before looping.
const MAX_PAGES: usize = 30;

pub struct HttpBackend {
    client: Client,
    base_url: String,
}

impl HttpBackend {
    pub fn new(config: &GithubReviewConfig) -> Result<Self, ReviewPublishError> {
        let token = config.token().ok_or_else(|| {
            ReviewPublishError::NotConfigured(format!(
                "{} is not set; export a GitHub token or install the `gh` CLI",
                config.token_env
            ))
        })?;
        let mut headers = HeaderMap::new();
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
                ReviewPublishError::NotConfigured(format!(
                    "{} does not contain a usable token",
                    config.token_env
                ))
            })?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            HeaderName::from_static(API_VERSION_HEADER),
            HeaderValue::from_static(API_VERSION),
        );
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));

        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(config.timeout_seconds))
            .build()
            .map_err(|error| ReviewPublishError::Other(error.into()))?;
        Ok(Self {
            client,
            base_url: config.api_base_url().to_string(),
        })
    }

    /// Whether HTTP mode could run at all.
    pub fn is_configured(config: &GithubReviewConfig) -> bool {
        config.token().is_some()
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    async fn get(&self, path: &str) -> Result<Value, ReviewPublishError> {
        let response = self
            .client
            .get(self.url(path))
            .send()
            .await
            .map_err(|error| ReviewPublishError::Other(error.into()))?;
        decode(response, path).await
    }
}

/// GitHub's `errors[]` messages ("Line could not be resolved") are the only
/// actionable signal on a 422, so they are surfaced verbatim.
async fn decode(response: Response, path: &str) -> Result<Value, ReviewPublishError> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status.is_success() {
        if body.trim().is_empty() {
            return Ok(Value::Null);
        }
        return serde_json::from_str(&body).map_err(|error| {
            ReviewPublishError::Remote(format!("could not read the response of {path}: {error}"))
        });
    }
    let detail = describe_error(&body);
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return Err(ReviewPublishError::NotConfigured(format!(
            "GitHub rejected the token for {path} ({status}): {detail}"
        )));
    }
    if status == StatusCode::NOT_FOUND {
        return Err(ReviewPublishError::DestinationUnresolved(format!(
            "{path} was not found ({status}): {detail}"
        )));
    }
    Err(ReviewPublishError::Remote(format!(
        "{path} failed with {status}: {detail}"
    )))
}

/// Flattens `{ "message": …, "errors": [{ "message": … }] }` into one line.
pub fn describe_error(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        let trimmed = body.trim();
        return if trimmed.is_empty() {
            "no response body".to_string()
        } else {
            trimmed.to_string()
        };
    };
    let mut parts = Vec::new();
    if let Some(message) = value.get("message").and_then(Value::as_str) {
        parts.push(message.to_string());
    }
    if let Some(errors) = value.get("errors").and_then(Value::as_array) {
        for error in errors {
            let detail = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            parts.push(detail);
        }
    }
    if parts.is_empty() {
        body.trim().to_string()
    } else {
        parts.join("; ")
    }
}

#[async_trait]
impl GithubBackend for HttpBackend {
    async fn resolve_pull_request(
        &self,
        workspace_root: &Path,
        explicit: Option<PullRequestRef>,
    ) -> Result<PullRequestContext, ReviewPublishError> {
        let pull_request = match explicit {
            Some(explicit) => explicit,
            None => self.discover_pull_request(workspace_root).await?,
        };
        let pull = self.get(&pull_request.api_path("")).await?;
        let head_sha = pull
            .pointer("/head/sha")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ReviewPublishError::DestinationUnresolved(format!(
                    "pull request {}#{} has no head commit",
                    pull_request.slug(),
                    pull_request.number
                ))
            })?
            .to_string();
        let url = pull
            .get("html_url")
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(PullRequestContext {
            pull_request,
            head_sha,
            url,
        })
    }

    async fn list_files(
        &self,
        pull_request: &PullRequestRef,
    ) -> Result<Vec<PullRequestFile>, ReviewPublishError> {
        let mut files = Vec::new();
        for page in 1..=MAX_PAGES {
            let path = format!(
                "{}?per_page={PER_PAGE}&page={page}",
                pull_request.api_path("/files")
            );
            let value = self.get(&path).await?;
            let batch: Vec<PullRequestFile> = serde_json::from_value(value).map_err(|error| {
                ReviewPublishError::Remote(format!(
                    "could not read the pull request file list: {error}"
                ))
            })?;
            let exhausted = batch.len() < PER_PAGE;
            files.extend(batch);
            if exhausted {
                break;
            }
        }
        Ok(files)
    }

    async fn submit_review(
        &self,
        pull_request: &PullRequestRef,
        payload: &Value,
    ) -> Result<SubmittedReview, ReviewPublishError> {
        let path = pull_request.api_path("/reviews");
        let response = self
            .client
            .post(self.url(&path))
            .json(payload)
            .send()
            .await
            .map_err(|error| ReviewPublishError::Other(error.into()))?;
        let value = decode(response, &path).await?;
        serde_json::from_value(value).map_err(|error| {
            ReviewPublishError::Remote(format!("could not read the submitted review: {error}"))
        })
    }
}

impl HttpBackend {
    /// Finds the open pull request for the checked-out branch of `origin`.
    async fn discover_pull_request(
        &self,
        workspace_root: &Path,
    ) -> Result<PullRequestRef, ReviewPublishError> {
        let unresolved = || {
            ReviewPublishError::DestinationUnresolved(
                "could not work out which pull request to review; pass destination.target as owner/repo#number"
                    .to_string(),
            )
        };
        let (owner, repo) = git::remote_slug(workspace_root)
            .await
            .ok_or_else(unresolved)?;
        let branch = git::current_branch(workspace_root)
            .await
            .ok_or_else(unresolved)?;
        let path = format!("repos/{owner}/{repo}/pulls?state=open&head={owner}:{branch}");
        let pulls = self.get(&path).await?;
        let number = pulls
            .as_array()
            .and_then(|pulls| pulls.first())
            .and_then(|pull| pull.get("number"))
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                ReviewPublishError::DestinationUnresolved(format!(
                    "no open pull request for {owner}/{repo} branch {branch}"
                ))
            })?;
        Ok(PullRequestRef {
            owner,
            repo,
            number,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_error_payloads_flatten_to_one_actionable_line() {
        let body = r#"{"message":"Validation Failed","errors":[{"message":"Line could not be resolved"},{"message":"Path could not be resolved"}]}"#;
        assert_eq!(
            describe_error(body),
            "Validation Failed; Line could not be resolved; Path could not be resolved"
        );
        assert_eq!(describe_error("boom"), "boom");
        assert_eq!(describe_error("   "), "no response body");
        assert_eq!(describe_error(r#"{"message":"Not Found"}"#), "Not Found");
    }

    #[test]
    fn http_mode_reports_a_missing_token_as_configuration() {
        let config = GithubReviewConfig {
            token_env: "RODER_TEST_GITHUB_TOKEN_THAT_IS_UNSET".to_string(),
            ..GithubReviewConfig::default()
        };
        assert!(!HttpBackend::is_configured(&config));
        let error = HttpBackend::new(&config).err().expect("missing token");
        assert!(matches!(error, ReviewPublishError::NotConfigured(_)));
    }
}
