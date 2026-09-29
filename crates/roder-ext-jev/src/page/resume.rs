//! Picking up the tabs an earlier call left open.
//!
//! A session keeps its tabs' target ids between calls, not a DevTools
//! connection: `cdp.rs` has no reader task, so a socket left idle would queue
//! events unread and leave the page's dialogs unanswered. Each call opens a
//! new connection and re-attaches to the recorded targets. The viewport,
//! focus emulation, the Page domain and the new-document scripts belong to
//! the DevTools session and end with it, so every re-attached tab is set up
//! again, the way an adopted tab is. The document the tab shows already ran
//! autoconsent when it loaded, so only later documents get it again.
//!
//! Tabs stay open, and shown, between calls, so a tab one of them opened in
//! the meantime (the user's click on a `target="_blank"` link, a page's own
//! timer) is the user's, not this call's: every page open at re-attach time
//! is recorded as seen, and only tabs opened after it are adopted or closed.

use std::fmt;

use serde_json::json;

use super::Page;
use super::tabs::{OwnedTab, Tab, attach_session, is_gone};
use crate::cdp::Connection;

/// None of the tabs a session recorded is open any more: the user closed
/// them, or Chrome restarted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TabsGone;

impl fmt::Display for TabsGone {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "none of this session's tabs is open any more")
    }
}

impl std::error::Error for TabsGone {}

impl Page {
    /// Re-attach to `tabs`, oldest first, the last the one to act on, and
    /// set each up again. A tab Chrome no longer has is left out and its
    /// target returned, so the run carries on in the newest tab still open;
    /// [`TabsGone`] when none is. The current tab is shown when
    /// `foreground`. Nothing is loaded or reloaded.
    pub(crate) async fn resume(
        connection: Connection,
        tabs: &[OwnedTab],
        foreground: bool,
        autoconsent: bool,
    ) -> anyhow::Result<(Self, Vec<String>)> {
        let mut page = Self {
            connection,
            tabs: Vec::new(),
            stray: Vec::new(),
            seen: Vec::new(),
            foreground,
            after_input: None,
            opened_tab: false,
            autoconsent,
            ledger: Default::default(),
            frame_text: super::frames::frame_text_on(),
        };
        let mut gone = Vec::new();
        for owned in tabs {
            match page.reattach(owned).await {
                Ok(()) => {}
                Err(error) if is_gone(&error) => gone.push(owned.target.clone()),
                Err(error) => return Err(error),
            }
        }
        if page.tabs.is_empty() {
            return Err(TabsGone.into());
        }
        page.seen = page.open_pages().await?;
        page.record_tabs();
        if foreground {
            page.activate().await?;
        }
        Ok((page, gone))
    }

    /// Attach to one recorded tab and set it up; it becomes current.
    async fn reattach(&mut self, owned: &OwnedTab) -> anyhow::Result<()> {
        let session = attach_session(&mut self.connection, &owned.target).await?;
        self.tabs.push(Tab {
            target: owned.target.clone(),
            session,
            opener: owned.opener.clone(),
        });
        let set_up = async {
            self.prepare().await?;
            self.install_tracker().await?;
            self.install_autoconsent(false).await
        };
        if let Err(error) = set_up.await {
            self.tabs.pop();
            return Err(error);
        }
        Ok(())
    }

    /// Every page target open now that the page does not own.
    async fn open_pages(&mut self) -> anyhow::Result<Vec<String>> {
        let targets = self
            .connection
            .call("Target.getTargets", json!({}), None)
            .await?;
        Ok(targets["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|target| target["type"] == "page")
            .filter_map(|target| target["targetId"].as_str())
            .filter(|id| !self.tabs.iter().any(|tab| tab.target == *id))
            .map(str::to_string)
            .collect())
    }

    /// The address the current tab shows, as Chrome reports it; readable
    /// even while the page itself is busy.
    pub(crate) async fn current_url(&mut self) -> anyhow::Result<String> {
        let target = self.target_id().to_string();
        let info = self
            .connection
            .call("Target.getTargetInfo", json!({"targetId": target}), None)
            .await?;
        Ok(info["targetInfo"]["url"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }
}
