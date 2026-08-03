//! `[review.publishers.github]` settings.

use std::path::PathBuf;

use roder_api::review::ReviewPriority;
use serde::{Deserialize, Serialize};

/// Publisher id under which this extension registers itself.
pub const GITHUB_REVIEW_PUBLISHER_ID: &str = "github";

pub const DEFAULT_API_BASE_URL: &str = "https://api.github.com";
pub const DEFAULT_TOKEN_ENV: &str = "GITHUB_TOKEN";
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 20;

/// Which transport talks to GitHub.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubReviewMode {
    /// Prefer the `gh` CLI, fall back to HTTP when it is not installed.
    #[default]
    Auto,
    Cli,
    Http,
}

impl GithubReviewMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "cli" | "gh" => Some(Self::Cli),
            "http" | "api" => Some(Self::Http),
            _ => None,
        }
    }
}

/// The `event` field of the submitted review. `Comment` never changes a pull
/// request's state, which is why it is the default: approving or requesting
/// changes on the user's behalf must be opt-in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GithubReviewEvent {
    #[default]
    Comment,
    RequestChanges,
    Approve,
}

impl GithubReviewEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Comment => "COMMENT",
            Self::RequestChanges => "REQUEST_CHANGES",
            Self::Approve => "APPROVE",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().replace('-', "_").as_str() {
            "COMMENT" => Some(Self::Comment),
            "REQUEST_CHANGES" => Some(Self::RequestChanges),
            "APPROVE" => Some(Self::Approve),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubReviewConfig {
    pub mode: GithubReviewMode,
    pub event: GithubReviewEvent,
    /// Least severe priority still worth publishing. `P3` keeps everything.
    pub min_priority: ReviewPriority,
    /// Environment variable holding the API token (HTTP mode only).
    pub token_env: String,
    /// `gh` executable, resolved on `PATH` when it is a bare name.
    pub gh_bin: PathBuf,
    pub api_base_url: String,
    pub timeout_seconds: u64,
}

impl Default for GithubReviewConfig {
    fn default() -> Self {
        Self {
            mode: GithubReviewMode::default(),
            event: GithubReviewEvent::default(),
            min_priority: ReviewPriority::P3,
            token_env: DEFAULT_TOKEN_ENV.to_string(),
            gh_bin: PathBuf::from("gh"),
            api_base_url: DEFAULT_API_BASE_URL.to_string(),
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        }
    }
}

/// Wire shape of `[review.publishers.github]`. Every field is optional; an
/// absent one keeps the publisher default. This lives here rather than in
/// `roder-config` so that adding a publisher never edits the core config.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GithubReviewConfigToml {
    /// `auto` (prefer the `gh` CLI), `cli`, or `http`.
    pub mode: Option<String>,
    /// `COMMENT` (default), `REQUEST_CHANGES`, or `APPROVE`.
    pub event: Option<String>,
    /// Least severe priority still published, `p0`..`p3`.
    pub min_priority: Option<String>,
    /// Environment variable holding the API token; `http` mode only.
    pub token_env: Option<String>,
    /// `gh` executable to run; resolved on `PATH` when it is a bare name.
    pub gh_bin: Option<String>,
    /// API root, for GitHub Enterprise.
    pub api_base_url: Option<String>,
    pub timeout_seconds: Option<u64>,
}

impl GithubReviewConfig {
    /// Resolves a raw `[review.publishers.github]` block.
    ///
    /// Unknown values fail rather than being silently ignored: a typo in
    /// `event = "REQEUST_CHANGES"` must not quietly downgrade a review to a
    /// plain comment.
    pub fn from_toml(value: &toml::Value) -> Result<Self, String> {
        let raw = value
            .clone()
            .try_into::<GithubReviewConfigToml>()
            .map_err(|err| format!("[review.publishers.github]: {err}"))?;
        Self::from_raw(&raw)
    }

    pub fn from_raw(raw: &GithubReviewConfigToml) -> Result<Self, String> {
        let mut resolved = Self::default();
        if let Some(mode) = raw.mode.as_deref() {
            resolved.mode = GithubReviewMode::parse(mode).ok_or_else(|| {
                format!("[review.publishers.github].mode = \"{mode}\"; expected auto, cli or http")
            })?;
        }
        if let Some(event) = raw.event.as_deref() {
            resolved.event = GithubReviewEvent::parse(event).ok_or_else(|| {
                format!(
                    "[review.publishers.github].event = \"{event}\"; expected COMMENT, REQUEST_CHANGES or APPROVE"
                )
            })?;
        }
        if let Some(min_priority) = raw.min_priority.as_deref() {
            resolved.min_priority = ReviewPriority::parse(min_priority).ok_or_else(|| {
                format!(
                    "[review.publishers.github].min_priority = \"{min_priority}\"; expected p0, p1, p2 or p3"
                )
            })?;
        }
        if let Some(token_env) = non_empty(raw.token_env.as_deref()) {
            resolved.token_env = token_env.to_string();
        }
        if let Some(gh_bin) = non_empty(raw.gh_bin.as_deref()) {
            resolved.gh_bin = gh_bin.into();
        }
        if let Some(api_base_url) = non_empty(raw.api_base_url.as_deref()) {
            resolved.api_base_url = api_base_url.to_string();
        }
        if let Some(timeout_seconds) = raw.timeout_seconds.filter(|seconds| *seconds > 0) {
            resolved.timeout_seconds = timeout_seconds;
        }
        Ok(resolved)
    }

    /// The configured token, when the environment actually carries one.
    pub fn token(&self) -> Option<String> {
        std::env::var(&self.token_env)
            .ok()
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty())
    }

    pub fn api_base_url(&self) -> &str {
        self.api_base_url.trim_end_matches('/')
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(document: &str) -> toml::Value {
        toml::Value::Table(toml::from_str(document).expect("parse block"))
    }

    #[test]
    fn every_field_maps_onto_the_publisher_config() {
        let resolved = GithubReviewConfig::from_toml(&block(
            r#"
            mode = "http"
            event = "REQUEST_CHANGES"
            min_priority = "p1"
            token_env = "GH_ENTERPRISE_TOKEN"
            gh_bin = "/opt/homebrew/bin/gh"
            api_base_url = "https://github.acme.dev/api/v3"
            timeout_seconds = 45
            "#,
        ))
        .expect("resolve");
        assert_eq!(resolved.mode, GithubReviewMode::Http);
        assert_eq!(resolved.event, GithubReviewEvent::RequestChanges);
        assert_eq!(resolved.min_priority, ReviewPriority::P1);
        assert_eq!(resolved.token_env, "GH_ENTERPRISE_TOKEN");
        assert_eq!(resolved.gh_bin.to_string_lossy(), "/opt/homebrew/bin/gh");
        assert_eq!(resolved.api_base_url(), "https://github.acme.dev/api/v3");
        assert_eq!(resolved.timeout_seconds, 45);
    }

    #[test]
    fn an_empty_block_yields_the_defaults() {
        assert_eq!(
            GithubReviewConfig::from_toml(&block("")).expect("defaults"),
            GithubReviewConfig::default()
        );
    }

    #[test]
    fn unknown_values_and_unknown_keys_are_rejected() {
        for bad in [
            "mode = \"telepathy\"",
            "event = \"REQEUST_CHANGES\"",
            "min_priority = \"urgent\"",
            "moed = \"cli\"",
        ] {
            assert!(
                GithubReviewConfig::from_toml(&block(bad)).is_err(),
                "{bad} should be rejected"
            );
        }
    }

    #[test]
    fn modes_and_events_parse_leniently() {
        assert_eq!(
            GithubReviewMode::parse("  GH "),
            Some(GithubReviewMode::Cli)
        );
        assert_eq!(GithubReviewMode::parse("API"), Some(GithubReviewMode::Http));
        assert_eq!(GithubReviewMode::parse("nope"), None);
        assert_eq!(
            GithubReviewEvent::parse("request-changes"),
            Some(GithubReviewEvent::RequestChanges)
        );
        assert_eq!(GithubReviewEvent::default().as_str(), "COMMENT");
    }

    #[test]
    fn defaults_publish_every_priority_as_a_plain_comment() {
        let config = GithubReviewConfig::default();
        assert_eq!(config.min_priority, ReviewPriority::P3);
        assert_eq!(config.event.as_str(), "COMMENT");
        assert_eq!(config.api_base_url(), "https://api.github.com");
    }
}
