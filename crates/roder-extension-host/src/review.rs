//! `[review.publishers.<id>]` → publisher settings.
//!
//! `roder-config` stores each publisher's block as an opaque `toml::Value`
//! keyed by publisher id, so the shape of a block is owned by the publisher's
//! own crate. Registering a new publisher means adding its crate and one arm
//! here — never editing the core config types.

use anyhow::Context;
use roder_config::ReviewConfig;
use roder_ext_github_review::{GITHUB_REVIEW_PUBLISHER_ID, GithubReviewConfig};

/// `[review.publishers.github]`.
pub fn github_review_config(review: Option<&ReviewConfig>) -> anyhow::Result<GithubReviewConfig> {
    let Some(block) = review.and_then(|review| review.publisher(GITHUB_REVIEW_PUBLISHER_ID)) else {
        return Ok(GithubReviewConfig::default());
    };
    GithubReviewConfig::from_toml(block)
        .map_err(anyhow::Error::msg)
        .context("invalid [review.publishers.github]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::review::ReviewPriority;
    use roder_ext_github_review::{GithubReviewEvent, GithubReviewMode};

    fn review(toml: &str) -> ReviewConfig {
        toml::from_str::<roder_config::Config>(toml)
            .expect("parse config")
            .review
            .expect("[review] block")
    }

    #[test]
    fn an_absent_block_yields_the_publisher_defaults() {
        assert_eq!(
            github_review_config(None).expect("defaults"),
            GithubReviewConfig::default()
        );
        assert_eq!(
            github_review_config(Some(&ReviewConfig::default())).expect("defaults"),
            GithubReviewConfig::default()
        );
    }

    #[test]
    fn a_configured_block_reaches_the_publisher() {
        let review = review(
            r#"
            [review]
            default_publisher = "github"
            model = "opus"

            [review.publishers.github]
            mode = "http"
            event = "REQUEST_CHANGES"
            min_priority = "p1"
            timeout_seconds = 45
            "#,
        );
        let resolved = github_review_config(Some(&review)).expect("resolve");
        assert_eq!(resolved.mode, GithubReviewMode::Http);
        assert_eq!(resolved.event, GithubReviewEvent::RequestChanges);
        assert_eq!(resolved.min_priority, ReviewPriority::P1);
        assert_eq!(resolved.timeout_seconds, 45);
    }

    #[test]
    fn a_block_for_another_publisher_is_ignored_not_rejected() {
        let review = review(
            r#"
            [review]
            default_publisher = "vex"

            [review.publishers.vex]
            endpoint = "https://vex.example"
            "#,
        );
        assert_eq!(
            github_review_config(Some(&review)).expect("defaults"),
            GithubReviewConfig::default()
        );
    }

    #[test]
    fn unknown_values_fail_the_build_instead_of_being_ignored() {
        let review = review("[review.publishers.github]\nevent = \"REQEUST_CHANGES\"\n");
        assert!(github_review_config(Some(&review)).is_err());
    }
}
