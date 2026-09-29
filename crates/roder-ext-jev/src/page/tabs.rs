//! Following a tab that an action opened.
//!
//! A link with `target="_blank"` or a `window.open` leaves the page Jev reads
//! unchanged, so upstream read the click as a no-op and three of them ended
//! the run `blocked`, while the new tab was left open in the background.
//! After each input settles, Jev lists the browser's targets once and adopts
//! a page opened by one of its own tabs: it attaches, repeats the tab setup
//! (viewport, focus emulation, dialogs, the quiet clock, autoconsent), waits for it to
//! load, and reads it from then on. The observation after that says so as
//! `opened_tab`, and the loop records it on the step. A tab that closes
//! itself hands the run back to the tab that opened it. The page keeps a
//! [`TabLedger`] of the tabs it owns, shared with whoever opened it, so a
//! session can keep them open for its next call after the engine has taken
//! the page; closing the page still closes every tab it owns. The approach follows fastbrowse's
//! browser/session.py (MIT), polling `Target.getTargets` instead of
//! subscribing to target events; Jev does not pass
//! `--disable-popup-blocking`, since DevTools input already counts as a user
//! gesture and the flag would change the user's own browser.

use std::sync::{Arc, Mutex};

use anyhow::Context;
use serde_json::{Value, json};

use super::{Page, close_target};

/// One tab Jev owns: its target, the session attached to it, and the owned
/// tab that opened it, if one did.
#[derive(Debug, Clone)]
pub(super) struct Tab {
    pub(super) target: String,
    pub(super) session: String,
    pub(super) opener: Option<String>,
}

/// The tabs a page owns, as its owner can read them after handing the page
/// away: in the order they opened, the last the one the run is on, and the
/// tabs they opened that the page never adopted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TabLedger {
    pub(crate) tabs: Vec<OwnedTab>,
    pub(crate) stray: Vec<String>,
}

/// One tab in a [`TabLedger`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedTab {
    pub(crate) target: String,
    /// The owned tab that opened this one.
    pub(crate) opener: Option<String>,
}

/// A page's ledger, shared with the caller that opened the page.
pub(crate) type SharedLedger = Arc<Mutex<TabLedger>>;

/// Chrome's protocol errors for a call whose tab or session went away.
pub(super) fn is_gone(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    [
        "Session with given id not found",
        "No target with given id",
        "Target closed",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

impl Page {
    /// The ledger this page keeps of its tabs, for its owner to read after
    /// the engine has taken the page.
    pub(crate) fn ledger(&self) -> SharedLedger {
        self.ledger.clone()
    }

    /// Bring the shared ledger up to date with the page's own tab list.
    pub(super) fn record_tabs(&self) {
        let ledger = TabLedger {
            tabs: self
                .tabs
                .iter()
                .map(|tab| OwnedTab {
                    target: tab.target.clone(),
                    opener: tab.opener.clone(),
                })
                .collect(),
            stray: self.stray.clone(),
        };
        if let Ok(mut shared) = self.ledger.lock() {
            *shared = ledger;
        }
    }

    /// Adopt the newest page a Jev tab opened since the last look, if any.
    /// A failure leaves the run on the tab it was on.
    pub(super) async fn follow_popup(&mut self) {
        let Ok(Some(target)) = self.opened_target().await else {
            return;
        };
        if self.adopt(&target).await.is_ok() {
            self.opened_tab = true;
        }
    }

    /// The newest page target whose opener is a Jev tab and that Jev does
    /// not own yet. Older ones are taken as owned too, to be closed later.
    /// A tab already open when a continued call began is never either.
    async fn opened_target(&mut self) -> anyhow::Result<Option<String>> {
        let targets = self
            .connection
            .call("Target.getTargets", json!({}), None)
            .await?;
        let owned = |id: &str| self.tabs.iter().any(|tab| tab.target == id);
        let opened = targets["targetInfos"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter(|target| target["type"] == "page")
            .filter(|target| target["openerId"].as_str().is_some_and(owned))
            .filter_map(|target| target["targetId"].as_str())
            .filter(|id| !owned(id) && !self.stray.iter().any(|stray| stray == id))
            .filter(|id| !self.seen.iter().any(|seen| seen == id))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let Some((newest, older)) = opened.split_last() else {
            return Ok(None);
        };
        self.stray.extend(older.iter().cloned());
        self.record_tabs();
        Ok(Some(newest.clone()))
    }

    /// Attach to `target`, set it up as Jev's own tab and read from it. The
    /// run moves to the tab only once its setup has succeeded; a tab whose
    /// setup failed is kept only to be closed with the page.
    pub(crate) async fn adopt(&mut self, target: &str) -> anyhow::Result<()> {
        // Owned before anything can fail, so closing the page closes it.
        self.stray.push(target.to_string());
        self.record_tabs();
        let session = attach_session(&mut self.connection, target).await?;
        // Setting up goes through the current tab's session, so the new tab
        // is current while it runs, and stops being current if it fails.
        let opener = Some(self.target_id().to_string());
        self.tabs.push(Tab {
            target: target.to_string(),
            session,
            opener,
        });
        self.after_input = None;
        if let Err(error) = self.set_up_adopted().await {
            self.tabs.pop();
            return Err(error);
        }
        self.stray.retain(|stray| stray != target);
        self.record_tabs();
        self.await_ready().await;
        self.settle_page(None).await;
        Ok(())
    }

    async fn set_up_adopted(&mut self) -> anyhow::Result<()> {
        self.prepare().await?;
        self.install_tracker().await?;
        self.install_autoconsent(true).await?;
        if self.foreground {
            self.activate().await?;
        }
        Ok(())
    }

    /// Back to the tab that opened the current one, when the current one is
    /// gone. Whether there was one to go back to.
    pub(super) fn return_to_opener(&mut self) -> bool {
        if self.tabs.len() < 2 {
            return false;
        }
        self.tabs.pop();
        self.after_input = None;
        self.record_tabs();
        true
    }

    /// Close every tab Jev owns, the current one last.
    pub(super) async fn close_all(&mut self) -> anyhow::Result<()> {
        let mut first_error = None;
        let others = self.tabs[..self.tabs.len() - 1]
            .iter()
            .map(|tab| tab.target.clone())
            .chain(self.stray.drain(..))
            .collect::<Vec<_>>();
        for target in others {
            if let Err(error) = close_target(&mut self.connection, &target).await
                && !is_gone(&error)
            {
                first_error.get_or_insert(error);
            }
        }
        let current = self.target_id().to_string();
        let closed = close_target(&mut self.connection, &current).await;
        // Whatever failed to close is no longer the page's to keep.
        if let Ok(mut shared) = self.ledger.lock() {
            *shared = TabLedger::default();
        }
        closed?;
        first_error.map_or(Ok(()), Err)
    }

    /// Put a just-adopted tab's news on its first observation.
    pub(super) fn add_opened_tab(&mut self, observation: &mut Value) {
        if std::mem::take(&mut self.opened_tab) {
            observation["opened_tab"] = json!(true);
        }
    }
}

/// Attach a flat DevTools session to `target` and return its id.
pub(super) async fn attach_session(
    connection: &mut crate::cdp::Connection,
    target: &str,
) -> anyhow::Result<String> {
    let attached = connection
        .call(
            "Target.attachToTarget",
            json!({"targetId": target, "flatten": true}),
            None,
        )
        .await?;
    Ok(attached["sessionId"]
        .as_str()
        .context("Chrome did not return a session")?
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromes_lost_tab_errors_read_as_gone() {
        for message in [
            "Runtime.evaluate failed: Session with given id not found.",
            "Target.closeTarget failed: No target with given id found",
        ] {
            assert!(is_gone(&anyhow::anyhow!("{message}")), "{message}");
        }
        assert!(!is_gone(&anyhow::anyhow!("Runtime.evaluate timed out")));
    }
}
