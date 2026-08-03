//! Handing a finished review to a [`ReviewPublisher`].
//!
//! Core knows nothing about GitHub or any other platform: a publisher is an
//! extension service resolved by id, so a third party ships a `roder-ext-*`
//! crate and everything below keeps working unchanged.

use std::sync::Arc;

use roder_api::events::{ReviewPublished, RoderEvent};
use roder_api::review::{
    ReviewDestination, ReviewId, ReviewPublishError, ReviewPublishRequest, ReviewPublishResult,
    ReviewPublisher,
};
use time::OffsetDateTime;

use crate::runtime::Runtime;

/// `[review]` settings the runtime itself acts on.
#[derive(Debug, Clone, Default)]
pub struct RuntimeReviewConfig {
    /// Publisher used when a publish request names none. `"none"` and the
    /// empty string mean "always ask", matching the config file.
    pub default_publisher: Option<String>,
    /// Model override for review turns.
    pub model: Option<String>,
}

impl RuntimeReviewConfig {
    fn default_publisher(&self) -> Option<&str> {
        self.default_publisher
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty() && !id.eq_ignore_ascii_case("none"))
    }
}

#[derive(Debug, Clone)]
pub struct PublishReviewRequest {
    pub review_id: ReviewId,
    pub destination: ReviewDestination,
    /// Indices into the review's findings; `None` publishes all of them.
    pub selected: Option<Vec<usize>>,
    pub dry_run: bool,
}

/// Separates "the caller asked for something impossible" from "the remote said
/// no", so a transport can map the first to an invalid-params error.
#[derive(Debug)]
pub enum PublishReviewError {
    InvalidRequest(String),
    Publish(ReviewPublishError),
}

impl std::fmt::Display for PublishReviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRequest(message) => f.write_str(message),
            Self::Publish(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for PublishReviewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Publish(error) => Some(error),
            Self::InvalidRequest(_) => None,
        }
    }
}

impl From<ReviewPublishError> for PublishReviewError {
    fn from(error: ReviewPublishError) -> Self {
        Self::Publish(error)
    }
}

impl Runtime {
    /// The live `[review]` settings.
    pub async fn review_config(&self) -> RuntimeReviewConfig {
        self.status().await.review
    }

    /// Publishes a completed review's findings and emits
    /// [`RoderEvent::ReviewPublished`].
    pub async fn publish_review(
        &self,
        request: PublishReviewRequest,
    ) -> Result<ReviewPublishResult, PublishReviewError> {
        // Review history lives in memory, so an id from an earlier process is a
        // caller mistake rather than a server fault.
        let record = self.review_record(&request.review_id).await.ok_or_else(|| {
            PublishReviewError::InvalidRequest(format!(
                "review {} is not in this session's history; re-run the review before publishing",
                request.review_id
            ))
        })?;

        let findings = record.output.findings.len();
        if let Some(index) = request
            .selected
            .iter()
            .flatten()
            .find(|index| **index >= findings)
        {
            return Err(PublishReviewError::InvalidRequest(format!(
                "finding index {index} is out of range; the review produced {findings} findings"
            )));
        }

        let publisher = self
            .resolve_review_publisher(request.destination.publisher_id.as_deref())
            .await?;

        let result = publisher
            .publish(ReviewPublishRequest {
                review_id: record.review_id.clone(),
                workspace_root: record.workspace_root.clone(),
                request: record.request.clone(),
                output: record.output.clone(),
                selected: request.selected,
                destination: request.destination,
                dry_run: request.dry_run,
            })
            .await?;

        self.emit(RoderEvent::ReviewPublished(ReviewPublished {
            thread_id: Some(record.parent_thread_id.clone()),
            review_id: record.review_id.clone(),
            publisher_id: result.publisher_id.clone(),
            url: result.url.clone(),
            published_findings: result.published_findings,
            skipped: result.skipped.clone(),
            dry_run: request.dry_run,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(result)
    }

    /// Explicit id, then `[review].default_publisher`, then the only installed
    /// publisher. Anything else is ambiguous and has to be spelled out.
    async fn resolve_review_publisher(
        &self,
        publisher_id: Option<&str>,
    ) -> Result<Arc<dyn ReviewPublisher>, PublishReviewError> {
        let publishers = self.registry.review_publishers();
        let configured = self.review_config().await;
        let requested = publisher_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .or_else(|| configured.default_publisher());

        let Some(requested) = requested else {
            return match publishers {
                [] => Err(PublishReviewError::InvalidRequest(
                    "no review publisher is installed; add a review publisher extension before publishing findings".to_string(),
                )),
                [only] => Ok(Arc::clone(only)),
                _ => Err(PublishReviewError::InvalidRequest(format!(
                    "several review publishers are installed ({}); pass publisherId or set [review].default_publisher",
                    publisher_ids(publishers)
                ))),
            };
        };

        self.registry.review_publisher(requested).ok_or_else(|| {
            PublishReviewError::InvalidRequest(format!(
                "review publisher '{requested}' is not installed; available publishers: {}",
                publisher_ids(publishers)
            ))
        })
    }
}

fn publisher_ids(publishers: &[Arc<dyn ReviewPublisher>]) -> String {
    if publishers.is_empty() {
        return "none".to_string();
    }
    publishers
        .iter()
        .map(|publisher| publisher.descriptor().id)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_and_blank_default_publishers_mean_ask_explicitly() {
        for value in [None, Some(""), Some("  "), Some("none"), Some("None")] {
            let config = RuntimeReviewConfig {
                default_publisher: value.map(str::to_string),
                model: None,
            };
            assert_eq!(config.default_publisher(), None, "value: {value:?}");
        }
        let config = RuntimeReviewConfig {
            default_publisher: Some(" github ".to_string()),
            model: None,
        };
        assert_eq!(config.default_publisher(), Some("github"));
    }
}
