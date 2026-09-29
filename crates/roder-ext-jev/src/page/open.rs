//! Opening Jev's tab and loading the start page.
//!
//! Upstream ignores what `Page.navigate` returns, so a start page that failed
//! to load was observed as Chrome's error page and cost decisions there. Jev
//! reads its `errorText`, retries a transient network failure twice (0.5 s,
//! then 1 s) and otherwise reports [`LoadFailed`], which the runner turns
//! into a `blocked` result without starting the loop. The retry rule follows
//! fastbrowse's browser/page.py (MIT), reimplemented. Every failure after
//! `Target.createTarget` closes the tab, so nothing is left behind.
//!
//! The tab is created in the background. A foreground task shows it at once,
//! before the page loads, rather than after the first observation as it
//! used to; a background task never brings it forward, since Jev may be
//! attached to the user's own browser. Either way focus emulation keeps the
//! page rendering: measured on a windowed Chrome, a background tab without
//! it reports `document.hidden`, never runs animation frames and stretches a
//! 50 ms timer to a second, and with it behaves as a shown one
//! (`fixture_harness::hidden_tab_tests`).

use std::time::Duration;

use anyhow::Context;
use serde_json::{Value, json};

use super::tabs::{Tab, attach_session, is_gone};
use super::{LOAD_TIMEOUT, Page};
use crate::cdp::Connection;
use crate::engine::StaleObservation;

const VIEWPORT_WIDTH: u32 = 1120;
const VIEWPORT_HEIGHT: u32 = 780;
/// How long closing a tab keeps checking that it went.
const CLOSE_WAIT: Duration = Duration::from_secs(1);
const CLOSE_POLL: Duration = Duration::from_millis(50);
/// How often the commit wait checks for the new document.
const COMMIT_POLL: Duration = Duration::from_millis(50);
/// The waits before each retry of a transient network failure.
const NAVIGATION_RETRIES: [Duration; 2] = [Duration::from_millis(500), Duration::from_secs(1)];

/// The start page did not load: Chrome's `errorText` for the last attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoadFailed(String);

impl LoadFailed {
    /// A refused, reset or timed-out connection, or a proxy failure, means
    /// the site could not be reached. A DNS failure is more often a typo.
    fn unreachable(&self) -> bool {
        net_error(&self.0).is_some_and(|code| {
            [
                "CONNECTION",
                "TIMED_OUT",
                "PROXY",
                "TUNNEL",
                "ADDRESS_UNREACHABLE",
            ]
            .iter()
            .any(|part| code.contains(part))
        })
    }
}

impl std::fmt::Display for LoadFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.unreachable() {
            true => write!(
                formatter,
                "could not load: the site is unreachable ({})",
                self.0
            ),
            false => write!(formatter, "could not load ({})", self.0),
        }
    }
}

impl std::error::Error for LoadFailed {}

/// The code of a Chrome network error, `ERR_…`, when `text` is exactly one.
fn net_error(text: &str) -> Option<&str> {
    let code = text.strip_prefix("net::")?;
    let rest = code.strip_prefix("ERR_")?;
    (!rest.is_empty() && rest.chars().all(|c| c.is_ascii_uppercase() || c == '_')).then_some(code)
}

/// Worth another try: a network error other than a name that does not
/// resolve or a load the page itself abandoned, which fail the same way again.
fn transient(text: &str) -> bool {
    net_error(text).is_some_and(|code| !matches!(code, "ERR_NAME_NOT_RESOLVED" | "ERR_ABORTED"))
}

impl Page {
    /// Open Jev's own background tab and load `url` in it, with
    /// autoconsent injected when `autoconsent`. On any failure the tab is
    /// closed again. The runner does the same in timed phases (see
    /// `runner::drive`).
    #[cfg(test)]
    pub(crate) async fn open(
        connection: Connection,
        url: &str,
        autoconsent: bool,
    ) -> anyhow::Result<Self> {
        let mut page = Self::create(connection, false).await?;
        page.inject_autoconsent(autoconsent);
        if let Err(error) = page.load(url).await {
            page.close().await.ok();
            return Err(error);
        }
        Ok(page)
    }

    /// Create and attach to a blank background tab, and show it when
    /// `foreground`, closing it if either fails.
    #[cfg(test)]
    pub(crate) async fn create(
        mut connection: Connection,
        foreground: bool,
    ) -> anyhow::Result<Self> {
        let target = create_target(&mut connection).await?;
        Self::attach(connection, target, foreground).await
    }

    /// Attach to a tab [`create_target`] made, and show it when
    /// `foreground`, closing it if either fails. A caller that can cut this
    /// off closes the tab by id itself (see `runner::drive`).
    pub(crate) async fn attach(
        mut connection: Connection,
        target_id: String,
        foreground: bool,
    ) -> anyhow::Result<Self> {
        let session = match attach_session(&mut connection, &target_id).await {
            Ok(session) => session,
            Err(error) => {
                close_target(&mut connection, &target_id).await.ok();
                return Err(error);
            }
        };
        let page = Self {
            connection,
            tabs: vec![Tab {
                target: target_id,
                session,
                opener: None,
            }],
            stray: Vec::new(),
            seen: Vec::new(),
            foreground,
            after_input: None,
            opened_tab: false,
            autoconsent: false,
            ledger: Default::default(),
            frame_text: super::frames::frame_text_on(),
        };
        page.record_tabs();
        let mut page = page;
        if foreground && let Err(error) = page.activate().await {
            page.close().await.ok();
            return Err(error);
        }
        Ok(page)
    }

    /// Set the tab up and load `url`, waiting for it to settle. The caller
    /// closes the tab if this fails.
    pub(crate) async fn load(&mut self, url: &str) -> anyhow::Result<()> {
        self.prepare().await?;
        self.install_tracker().await?;
        self.install_autoconsent(false).await?;
        self.load_in(url, false).await
    }

    /// Load `url` in the tab, which is already set up and showing a page,
    /// and wait for the new document to commit, be ready and settle. Without
    /// the commit wait, a tab that already showed a page read the old
    /// document as ready and settled while the new one was still on its way.
    pub(crate) async fn goto(&mut self, url: &str) -> anyhow::Result<()> {
        self.load_in(url, true).await
    }

    /// [`Self::goto`]; `shown` when the tab shows a page already, whose own
    /// activity can abandon the load once (see [`Self::navigate`]).
    async fn load_in(&mut self, url: &str, shown: bool) -> anyhow::Result<()> {
        let stamp = self.stamp_document().await;
        let navigated = self.navigate(url, shown).await?;
        // A `loaderId` means a new document; a same-document navigation
        // (a fragment, a history push) has none and keeps the stamp.
        if let Some(stamp) = stamp
            && navigated.get("loaderId").is_some_and(|id| !id.is_null())
        {
            self.await_commit(&stamp).await;
        }
        self.await_ready().await;
        self.settle_page(None).await;
        Ok(())
    }

    /// Mark the document the tab shows now, so the next one can be told
    /// apart from it. `None` when the page could not be marked.
    async fn stamp_document(&mut self) -> Option<String> {
        let stamp = format!("{:x}", rand_stamp());
        let script = format!("window.__jevDoc = {:?}", stamp);
        self.evaluate(&script).await.ok().map(|_| stamp)
    }

    /// Wait, for at most the load timeout, until the tab no longer shows
    /// the document marked with `stamp`.
    async fn await_commit(&mut self, stamp: &str) {
        let deadline = tokio::time::Instant::now() + LOAD_TIMEOUT;
        let check = format!("window.__jevDoc === {stamp:?}");
        while tokio::time::Instant::now() < deadline {
            match self.evaluate(&check).await {
                Ok(Value::Bool(true)) => {}
                // A new document, or none reachable yet between the two.
                Ok(_) => return,
                Err(error) if error.is::<StaleObservation>() => {}
                Err(_) => return,
            }
            tokio::time::sleep(COMMIT_POLL).await;
        }
    }

    /// What every Jev tab needs before it is read: the viewport, focus
    /// emulation, and the Page domain for dialogs.
    pub(super) async fn prepare(&mut self) -> anyhow::Result<()> {
        self.call(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": VIEWPORT_WIDTH,
                "height": VIEWPORT_HEIGHT,
                "deviceScaleFactor": 1,
                "mobile": false,
            }),
        )
        .await?;
        // Keep rAF and menus rendering in an owned background tab.
        self.call(
            "Emulation.setFocusEmulationEnabled",
            json!({"enabled": true}),
        )
        .await?;
        // Dialogs are only reported, and so answered, with the Page domain on.
        self.call("Page.enable", json!({})).await.map(|_| ())
    }

    /// Turn focus emulation off or on again, to measure a hidden tab.
    #[cfg(test)]
    pub(crate) async fn emulate_focus(&mut self, enabled: bool) -> anyhow::Result<()> {
        self.call(
            "Emulation.setFocusEmulationEnabled",
            json!({"enabled": enabled}),
        )
        .await
        .map(|_| ())
    }

    /// `Page.navigate`, retrying a transient network failure; any other
    /// `errorText` is a [`LoadFailed`]. Returns Chrome's reply.
    ///
    /// In a tab that `shown` a page already, an abandoned load
    /// (`net::ERR_ABORTED`, not a download) is tried once more: the live
    /// booking benchmark had a reused tab abandon a plain redirect to the
    /// next site that loaded at once in a fresh tab. A fresh tab's abandoned
    /// load (a 204 reply) fails the same way again, so it is not retried.
    async fn navigate(&mut self, url: &str, shown: bool) -> anyhow::Result<Value> {
        let mut retries = NAVIGATION_RETRIES.iter();
        let mut abandoned_once = false;
        loop {
            let result = self.call("Page.navigate", json!({"url": url})).await?;
            let Some(error) = error_text(&result) else {
                return Ok(result);
            };
            let abandoned = shown
                && !abandoned_once
                && net_error(error) == Some("ERR_ABORTED")
                && result["isDownload"] != Value::Bool(true);
            if abandoned {
                abandoned_once = true;
                tokio::time::sleep(NAVIGATION_RETRIES[0]).await;
                continue;
            }
            match retries.next() {
                Some(delay) if transient(error) => tokio::time::sleep(*delay).await,
                _ => return Err(LoadFailed(error.to_string()).into()),
            }
        }
    }
}

/// Create a blank background tab for Jev and return its target id. Chrome
/// answers at once.
pub(crate) async fn create_target(connection: &mut Connection) -> anyhow::Result<String> {
    let target = connection
        .call(
            "Target.createTarget",
            json!({"url": "about:blank", "background": true}),
            None,
        )
        .await?;
    Ok(target["targetId"]
        .as_str()
        .context("Chrome did not return a target")?
        .to_string())
}

/// Close a tab and wait until Chrome no longer lists it. Chrome answers
/// `Target.closeTarget` with success but drops the close while a failed
/// navigation is swapping in its error page, which leaked the tab of every
/// start page that did not load; so the close is sent again until the tab is
/// gone, for up to a second. A close Chrome refuses because it no longer has
/// the tab ("No target with given id") found it closed: a repeat after the
/// earlier close landed, or a tab that closed itself.
pub(crate) async fn close_target(
    connection: &mut Connection,
    target_id: &str,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + CLOSE_WAIT;
    loop {
        match connection
            .call("Target.closeTarget", json!({"targetId": target_id}), None)
            .await
        {
            Err(error) if is_gone(&error) => return Ok(()),
            result => result?,
        };
        tokio::time::sleep(CLOSE_POLL).await;
        let targets = connection
            .call("Target.getTargets", json!({}), None)
            .await?;
        let open = targets["targetInfos"]
            .as_array()
            .is_some_and(|targets| targets.iter().any(|target| target["targetId"] == target_id));
        if !open {
            return Ok(());
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "Chrome did not close the tab"
        );
    }
}

/// A value unlikely to repeat between two navigations of one tab.
fn rand_stamp() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos()),
    );
    hasher.finish()
}

fn error_text(result: &Value) -> Option<&str> {
    result["errorText"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transient_network_errors_are_retried() {
        for retried in [
            "net::ERR_CONNECTION_REFUSED",
            "net::ERR_CONNECTION_RESET",
            "net::ERR_TIMED_OUT",
            "net::ERR_EMPTY_RESPONSE",
            "net::ERR_PROXY_CONNECTION_FAILED",
        ] {
            assert!(transient(retried), "{retried}");
        }
        for final_error in [
            "net::ERR_NAME_NOT_RESOLVED",
            "net::ERR_ABORTED",
            "Cannot navigate to invalid URL",
            "net::ERR_",
            "net::ERR_lower",
            "net::ERR_CONNECTION_REFUSED extra",
        ] {
            assert!(!transient(final_error), "{final_error}");
        }
    }

    #[test]
    fn unreachable_is_said_only_of_connection_timeout_and_proxy_errors() {
        let message = |text: &str| LoadFailed(text.into()).to_string();
        assert_eq!(
            message("net::ERR_CONNECTION_REFUSED"),
            "could not load: the site is unreachable (net::ERR_CONNECTION_REFUSED)"
        );
        assert!(message("net::ERR_TIMED_OUT").contains("unreachable"));
        assert!(message("net::ERR_PROXY_CONNECTION_FAILED").contains("unreachable"));
        assert_eq!(
            message("net::ERR_NAME_NOT_RESOLVED"),
            "could not load (net::ERR_NAME_NOT_RESOLVED)"
        );
        assert_eq!(
            message("net::ERR_ABORTED"),
            "could not load (net::ERR_ABORTED)"
        );
        assert_eq!(
            message("Cannot navigate to invalid URL"),
            "could not load (Cannot navigate to invalid URL)"
        );
    }

    #[test]
    fn a_blank_error_text_is_a_successful_navigation() {
        assert_eq!(error_text(&json!({"frameId": "f"})), None);
        assert_eq!(error_text(&json!({"errorText": ""})), None);
        assert_eq!(
            error_text(&json!({"errorText": "net::ERR_ABORTED"})),
            Some("net::ERR_ABORTED")
        );
    }
}
