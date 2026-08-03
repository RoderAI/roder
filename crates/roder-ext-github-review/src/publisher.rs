//! The [`ReviewPublisher`] implementation.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use roder_api::review::{
    ReviewPublishError, ReviewPublishRequest, ReviewPublishResult, ReviewPublisher,
    ReviewPublisherCapabilities, ReviewPublisherDescriptor,
};
use serde_json::Value;

use crate::backend::{GithubBackend, PullRequestRef};
use crate::config::{
    GITHUB_REVIEW_PUBLISHER_ID, GithubReviewConfig, GithubReviewEvent, GithubReviewMode,
};
use crate::gh_cli::GhCliBackend;
use crate::git;
use crate::http::HttpBackend;
use crate::map::{self, PayloadOptions};

#[derive(Debug, Default)]
pub struct GithubReviewPublisher {
    config: GithubReviewConfig,
}

impl GithubReviewPublisher {
    pub fn new(config: GithubReviewConfig) -> Self {
        Self { config }
    }

    /// Picks the transport. `auto` prefers the CLI because it needs no
    /// configuration; the error names both ways out when neither is usable.
    fn backend(&self) -> Result<Arc<dyn GithubBackend>, ReviewPublishError> {
        match self.config.mode {
            GithubReviewMode::Cli => Ok(Arc::new(GhCliBackend::new(&self.config))),
            GithubReviewMode::Http => Ok(Arc::new(HttpBackend::new(&self.config)?)),
            GithubReviewMode::Auto => {
                if GhCliBackend::is_installed(&self.config) {
                    return Ok(Arc::new(GhCliBackend::new(&self.config)));
                }
                if HttpBackend::is_configured(&self.config) {
                    return Ok(Arc::new(HttpBackend::new(&self.config)?));
                }
                Err(ReviewPublishError::NotConfigured(format!(
                    "neither the `gh` CLI nor {} is available; install the GitHub CLI or export a token",
                    self.config.token_env
                )))
            }
        }
    }

    /// Per-publish `event` override, e.g. `{"event": "REQUEST_CHANGES"}`.
    fn event(
        &self,
        options: &serde_json::Map<String, Value>,
    ) -> Result<GithubReviewEvent, ReviewPublishError> {
        let Some(event) = options.get("event") else {
            return Ok(self.config.event);
        };
        let event = event.as_str().unwrap_or_default();
        GithubReviewEvent::parse(event).ok_or_else(|| {
            ReviewPublishError::DestinationUnresolved(format!(
                "unknown review event '{event}'; expected COMMENT, REQUEST_CHANGES or APPROVE"
            ))
        })
    }
}

#[async_trait]
impl ReviewPublisher for GithubReviewPublisher {
    fn descriptor(&self) -> ReviewPublisherDescriptor {
        ReviewPublisherDescriptor {
            id: GITHUB_REVIEW_PUBLISHER_ID.to_string(),
            display_name: "GitHub pull request review".to_string(),
            capabilities: ReviewPublisherCapabilities {
                inline_comments: true,
                summary_body: true,
                autodetect_destination: true,
                dry_run: true,
            },
        }
    }

    async fn is_available(&self, _workspace_root: &Path) -> bool {
        match self.config.mode {
            GithubReviewMode::Cli => GhCliBackend::is_installed(&self.config),
            GithubReviewMode::Http => HttpBackend::is_configured(&self.config),
            GithubReviewMode::Auto => {
                GhCliBackend::is_installed(&self.config) || HttpBackend::is_configured(&self.config)
            }
        }
    }

    async fn publish(
        &self,
        request: ReviewPublishRequest,
    ) -> Result<ReviewPublishResult, ReviewPublishError> {
        let workspace_root = request.workspace_root.as_path();
        let repo_root = git::repo_root(workspace_root).await?;

        let explicit = match request.destination.target.as_deref() {
            Some(target) => Some(PullRequestRef::parse(target).ok_or_else(|| {
                ReviewPublishError::DestinationUnresolved(format!(
                    "'{target}' is not a pull request; expected owner/repo#number"
                ))
            })?),
            None => None,
        };
        let event = self.event(&request.destination.options)?;

        let backend = self.backend()?;
        let context = backend
            .resolve_pull_request(workspace_root, explicit)
            .await?;
        let files = backend.list_files(&context.pull_request).await?;

        let mapped = map::build_review_payload(
            &request.output,
            request.selected.as_deref(),
            &PayloadOptions {
                commit_id: &context.head_sha,
                repo_root: &repo_root,
                files: &files,
                event: event.as_str(),
                min_priority: self.config.min_priority,
            },
        );

        if request.dry_run {
            return Ok(ReviewPublishResult {
                publisher_id: GITHUB_REVIEW_PUBLISHER_ID.to_string(),
                url: context.url,
                published_findings: mapped.comment_count,
                skipped: mapped.skipped,
                payload_preview: Some(mapped.payload),
            });
        }

        let submitted = backend
            .submit_review(&context.pull_request, &mapped.payload)
            .await?;
        Ok(ReviewPublishResult {
            publisher_id: GITHUB_REVIEW_PUBLISHER_ID.to_string(),
            url: submitted.html_url.or(context.url),
            published_findings: mapped.comment_count,
            skipped: mapped.skipped,
            payload_preview: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::review::{ReviewDestination, ReviewPriority};

    #[test]
    fn descriptor_advertises_inline_comments_and_dry_run() {
        let descriptor = GithubReviewPublisher::default().descriptor();
        assert_eq!(descriptor.id, "github");
        assert!(descriptor.capabilities.inline_comments);
        assert!(descriptor.capabilities.dry_run);
        assert!(descriptor.capabilities.autodetect_destination);
    }

    #[test]
    fn destination_options_override_the_configured_event() {
        let publisher = GithubReviewPublisher::default();
        let mut destination = ReviewDestination::default();
        assert_eq!(
            publisher.event(&destination.options).expect("default"),
            GithubReviewEvent::Comment
        );
        destination
            .options
            .insert("event".to_string(), Value::String("APPROVE".to_string()));
        assert_eq!(
            publisher.event(&destination.options).expect("override"),
            GithubReviewEvent::Approve
        );
        destination
            .options
            .insert("event".to_string(), Value::String("shipit".to_string()));
        assert!(publisher.event(&destination.options).is_err());
    }

    #[tokio::test]
    async fn an_unparseable_target_fails_before_any_network_call() {
        let publisher = GithubReviewPublisher::new(GithubReviewConfig {
            min_priority: ReviewPriority::P3,
            ..GithubReviewConfig::default()
        });
        let error = publisher
            .publish(ReviewPublishRequest {
                review_id: "review-1".to_string(),
                workspace_root: std::env::current_dir().expect("cwd"),
                request: roder_api::review::ReviewRequest {
                    target: roder_api::review::ReviewTarget::default(),
                    label: "uncommitted changes".to_string(),
                    prompt: String::new(),
                    base_sha: None,
                    head_sha: None,
                    workspace_root: std::env::current_dir().expect("cwd"),
                },
                output: roder_api::review::ReviewOutput::default(),
                selected: None,
                destination: ReviewDestination {
                    target: Some("not-a-pull-request".to_string()),
                    ..ReviewDestination::default()
                },
                dry_run: true,
            })
            .await
            .expect_err("unparseable target");
        assert!(matches!(
            error,
            ReviewPublishError::DestinationUnresolved(_)
        ));
    }
}
