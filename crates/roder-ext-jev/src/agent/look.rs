//! Looking at a page before believing what it seems to say: an empty first
//! read is read again, and a page that refused automated access ends the
//! run before any decision is spent on it (see [`crate::block`]).

use std::time::Duration;

use super::Agent;
use crate::block;
use crate::engine::{JevPageFacts, JevStatus, StaleObservation};
use crate::space::action_space;

/// Below this much text, a page with nothing to act on shows nothing yet.
const EMPTY_TEXT_CHARS: usize = 20;
/// The pauses before each new read of a page that shows nothing, about
/// 2.6 s in all: a document that commits before it renders (a client-side
/// app, an interstitial) was read as blank and the model answered BLOCKED.
const LOOK_PAUSES: [Duration; 3] = [
    Duration::from_millis(300),
    Duration::from_millis(800),
    Duration::from_millis(1500),
];
/// How long the browser may take to describe the page.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(1);

impl Agent {
    /// How many elements the current observation offers to act on.
    fn elements(&self) -> usize {
        action_space(
            self.observation["actions"]
                .as_array()
                .map_or(&[][..], Vec::as_slice),
        )
        .elements
        .len()
    }

    /// Nothing to act on and next to no text: a page that has not rendered.
    pub(super) fn shows_nothing(&self) -> bool {
        let text = self.observation["text"].as_str().unwrap_or_default();
        self.elements() == 0 && text.trim().chars().count() < EMPTY_TEXT_CHARS
    }

    /// Read a page that shows nothing again, a few times over about 2.6 s,
    /// until it shows something. Costs no decision. A page read again this
    /// way is not read again for a BLOCKED about it.
    pub(super) async fn look_again_while_empty(&mut self) -> anyhow::Result<()> {
        if !self.shows_nothing() {
            return Ok(());
        }
        self.looked_again = true;
        for pause in LOOK_PAUSES {
            tokio::time::sleep(pause).await;
            match self.observe().await {
                Ok(()) => {}
                // Still on its way; the next read tries again.
                Err(error) if error.is::<StaleObservation>() => continue,
                Err(error) => return Err(error),
            }
            if !self.shows_nothing() {
                break;
            }
        }
        self.elapsed();
        Ok(())
    }

    /// Ask the browser what else the page shows, bounded and best effort,
    /// with any typed secret scrubbed from it.
    async fn describe(&mut self) -> Option<JevPageFacts> {
        let facts = tokio::time::timeout(DESCRIBE_TIMEOUT, self.browser.describe())
            .await
            .ok()?
            .ok()??;
        Some(JevPageFacts {
            headings: facts
                .headings
                .iter()
                .map(|heading| self.secrets.scrub(heading))
                .collect(),
            ..facts
        })
    }

    /// Why the page refused automated access, when it did.
    fn refusal(&self) -> Option<String> {
        let block = block::classify(
            self.facts.as_ref().and_then(|facts| facts.http_status),
            self.observation["url"].as_str().unwrap_or_default(),
            self.observation["title"].as_str().unwrap_or_default(),
            self.observation["text"].as_str().unwrap_or_default(),
            self.elements(),
        )?;
        Some(self.secrets.scrub(&block.reason()))
    }

    /// On the first observation: why the page refused automated access, if
    /// it did, so the run ends before any decision.
    pub(super) async fn refused_access(&mut self) -> Option<String> {
        self.facts = self.describe().await;
        self.refusal()
    }

    /// After the loop: describe the final page for the result, and name a
    /// blocked run whose page refused automated access for what it is. A
    /// timed-out run has spent its time and goes as it is.
    pub(crate) async fn finish(&mut self, stopped_because: Option<String>) -> Option<String> {
        if self.status == JevStatus::TimedOut {
            // The first page's facts say nothing of a page acted on since.
            if !self.history.is_empty() {
                self.facts = None;
            }
            return stopped_because;
        }
        if self.status == JevStatus::AccessDenied {
            return stopped_because;
        }
        self.facts = self.describe().await;
        if self.status == JevStatus::Blocked
            && let Some(reason) = self.refusal()
        {
            self.status = JevStatus::AccessDenied;
            return Some(reason);
        }
        stopped_because
    }
}
