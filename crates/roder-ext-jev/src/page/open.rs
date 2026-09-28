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

use super::Page;
use super::tabs::{Tab, is_gone};
use crate::cdp::Connection;

const VIEWPORT_WIDTH: u32 = 1120;
const VIEWPORT_HEIGHT: u32 = 780;
/// How long closing a tab keeps checking that it went.
const CLOSE_WAIT: Duration = Duration::from_secs(1);
const CLOSE_POLL: Duration = Duration::from_millis(50);
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
        let attached = connection
            .call(
                "Target.attachToTarget",
                json!({"targetId": target_id, "flatten": true}),
                None,
            )
            .await
            .and_then(|attached| {
                attached["sessionId"]
                    .as_str()
                    .map(str::to_string)
                    .context("Chrome did not return a session")
            });
        let session = match attached {
            Ok(session) => session,
            Err(error) => {
                close_target(&mut connection, &target_id).await.ok();
                return Err(error);
            }
        };
        let mut page = Self {
            connection,
            tabs: vec![Tab {
                target: target_id,
                session,
            }],
            stray: Vec::new(),
            foreground,
            after_input: None,
            opened_tab: false,
            autoconsent: false,
        };
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
        self.navigate(url).await?;
        self.await_ready().await;
        self.settle_page(None).await;
        Ok(())
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
    /// `errorText` is a [`LoadFailed`].
    async fn navigate(&mut self, url: &str) -> anyhow::Result<()> {
        let mut retries = NAVIGATION_RETRIES.iter();
        loop {
            let result = self.call("Page.navigate", json!({"url": url})).await?;
            let Some(error) = error_text(&result) else {
                return Ok(());
            };
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
