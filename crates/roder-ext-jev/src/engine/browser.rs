//! Browser capabilities used by the bounded decision loop.
use super::{JevActOutcome, JevPageFacts};
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

/// Browser operations needed by the JEV loop.
///
/// An observation may carry `dialogs`, the [`super::JevDialog`]s answered since the
/// last one, and `opened_tab: true` when the last action opened a tab the
/// browser now reads; the loop records both on the step that preceded them.
/// A fill action whose `input_type` is `password` or `one-time-code` is a
/// secret field: its `value` should be empty and `filled` say whether it
/// holds anything, and the loop records what it types there as `[secret]`.
#[async_trait]
pub trait JevBrowser: Send {
    /// Optional current viewport as an inline image. The engine requests this
    /// only for visual decision clients and suppresses capture around secrets.
    async fn screenshot(&mut self) -> anyhow::Result<Option<String>> {
        Ok(None)
    }

    async fn observe(&mut self) -> anyhow::Result<Value>;

    async fn fresh(&mut self, observation: &Value, action: Option<&Value>) -> anyhow::Result<bool>;

    async fn act(
        &mut self,
        action: &Value,
        observation: &Value,
        text: Option<&str>,
        wait: Duration,
    ) -> anyhow::Result<JevActOutcome>;

    async fn close(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    /// With cookie-banner refusal on (the default; see
    /// [`super::JevEngineConfig::with_cookie_banner_refusal`]), the loop calls this
    /// before every observation. Once per document, after the page settles,
    /// a browser that supports it refuses a cookie or consent banner (never
    /// accepting or opening settings) and returns what it clicked, as page
    /// text: the button's label, or the consent platform it refused on.
    /// `None` when it did nothing new, and always for a browser that does
    /// not support it, which is the default.
    async fn refuse_cookie_banner(&mut self) -> anyhow::Result<Option<String>> {
        Ok(None)
    }

    /// What the page shows besides its observation: the main document's
    /// HTTP status and visible headings. The loop asks once after the first
    /// observation, to stop on a page that refused automated access before
    /// any decision, and once after the run, for the result; each ask is
    /// bounded to a second and may fail without harm. `None`, the default,
    /// for a browser that cannot tell.
    async fn describe(&mut self) -> anyhow::Result<Option<JevPageFacts>> {
        Ok(None)
    }
}
