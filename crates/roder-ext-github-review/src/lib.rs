//! GitHub pull-request review publisher.
//!
//! Turns the structured findings of Roder's read-only `/review` sub-turn into a
//! single GitHub review: a summary body plus inline comments anchored to the
//! pull request's diff. Two transports sit behind one publisher — the `gh` CLI
//! (preferred, because it reuses existing `gh auth` credentials) and direct
//! REST calls with a `GITHUB_TOKEN` — selected by
//! `[review.publishers.github].mode`.
//!
//! GitHub rejects an entire review when a single comment falls outside the
//! diff, so [`map`] validates every comment against the hunk headers of
//! `pulls/{n}/files` first and routes anything unanchorable into the summary.

mod backend;
pub mod config;
mod gh_cli;
mod git;
mod http;
pub mod map;
mod publisher;

use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

pub use backend::PullRequestRef;
pub use config::{
    GITHUB_REVIEW_PUBLISHER_ID, GithubReviewConfig, GithubReviewEvent, GithubReviewMode,
};
pub use map::{PullRequestFile, build_review_payload, parse_hunk_spans};
pub use publisher::GithubReviewPublisher;

/// Installs the GitHub review publisher.
///
/// Registration is unconditional: whether the CLI or a token is usable is
/// decided when a publish is actually attempted, so an unconfigured install
/// reports a precise `NotConfigured` error instead of silently disappearing
/// from `review/publishers/list`.
#[derive(Debug, Default)]
pub struct GithubReviewExtension {
    config: GithubReviewConfig,
}

impl GithubReviewExtension {
    pub fn new(config: GithubReviewConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for GithubReviewExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-github-review".to_string(),
            name: "GitHub Review Publisher".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Publishes review findings as GitHub pull-request review comments.".to_string(),
            ),
            provides: vec![ProvidedService::ReviewPublisher(
                GITHUB_REVIEW_PUBLISHER_ID.to_string(),
            )],
            required_capabilities: vec![
                CapabilityRequest::new("process.exec.gh"),
                CapabilityRequest::new("network.api.github.com"),
                CapabilityRequest::new(format!("secret.read.{}", self.config.token_env)),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.review_publisher(Arc::new(GithubReviewPublisher::new(self.config.clone())));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extension_registers_the_github_publisher() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder
            .install(GithubReviewExtension::default())
            .expect("install");
        let registry = builder.build().expect("build registry");
        let publisher = registry
            .review_publisher(GITHUB_REVIEW_PUBLISHER_ID)
            .expect("github publisher registered");
        assert_eq!(
            publisher.descriptor().display_name,
            "GitHub pull request review"
        );
    }
}
