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

    /// Whether this browser can read rendered document text without scrolling.
    fn document_read_available(&self) -> bool {
        false
    }

    /// Raw bounded document text; the engine scrubs complete records before paging.
    async fn document_text(&mut self) -> anyhow::Result<Option<Value>> {
        Ok(None)
    }

    async fn observe(&mut self) -> anyhow::Result<Value>;

    /// Whether the same document is still loaded. Interaction completion uses
    /// the stronger full-observation check instead.
    async fn fresh_document(&mut self, observation: &Value) -> anyhow::Result<bool> {
        self.fresh(observation, None).await
    }

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

    /// Load `url`, the address the page shows, again with a fresh GET (never
    /// a reload, which would post a posted form again) and wait for it to
    /// settle; whether it was. The loop asks at most once, before any
    /// action, of a start page that still shows nothing after the first
    /// reads: a cold app can leave its first document blank, and its scripts
    /// get another chance. Never after an action, which a reload could
    /// repeat. `false`, the default, for a browser that cannot.
    async fn reload(&mut self, _url: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    /// Wait, for at most `within`, until the page moves on: its document
    /// changes or another document replaces it. The loop asks this while it
    /// watches an opening page that may clear itself (a challenge, a 503),
    /// and reads the page again after each return. Waiting the whole time,
    /// the default, is right for a browser that cannot tell.
    async fn wait_for_change(&mut self, within: Duration) -> anyhow::Result<()> {
        tokio::time::sleep(within).await;
        Ok(())
    }
}
