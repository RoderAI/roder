//! Looking at a page before believing what it seems to say: an empty first
//! read is read again, and a page that refused automated access ends the
//! run before any decision is spent on it (see [`crate::block`]). A run
//! that stops short at a sign-in it cannot make ends `needs_login` with the
//! wall it stands at (see [`crate::wall`]).

use std::time::Duration;

use super::Agent;
use crate::block;
use crate::engine::{JevPageFacts, JevStatus, JevStopCause, JevWall, StaleObservation};
use crate::space::action_space;
use crate::wall;

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

    /// When the page refused automated access, or answered 401 with nothing
    /// to act on, end the run `access_denied` or `needs_login` and say why.
    fn stop_refused(&mut self) -> Option<String> {
        let block = block::classify(
            self.facts.as_ref().and_then(|facts| facts.http_status),
            self.observation["url"].as_str().unwrap_or_default(),
            self.observation["title"].as_str().unwrap_or_default(),
            self.observation["text"].as_str().unwrap_or_default(),
            self.elements(),
        )?;
        self.status = block.status();
        if block.status() == JevStatus::NeedsLogin {
            self.wall = Some(wall::unauthorized(&self.observation));
        }
        Some(self.secrets.scrub(&block.reason()))
    }

    /// On the first observation: why the page refused automated access, if
    /// it did, so the run ends before any decision.
    pub(super) async fn refused_access(&mut self) -> Option<String> {
        self.facts = self.describe().await;
        self.stop_refused()
    }

    /// The sign-in wall a run that stopped short stands at, on the page's
    /// own evidence (see [`crate::wall`]): behind the field the goal gave
    /// no value for, on the page a `blocked` run ended on, or behind a stop
    /// an embedder already called `needs_login`.
    fn sign_in_wall(&self) -> Option<JevWall> {
        let headings = self
            .facts
            .as_ref()
            .map_or(&[][..], |facts| facts.headings.as_slice());
        match self.status {
            JevStatus::NeedsInput => self
                .missing_field
                .as_ref()
                .and_then(|field| wall::behind(&self.observation, field)),
            JevStatus::Blocked
                if !matches!(
                    self.cause,
                    Some(JevStopCause::OutsideScope | JevStopCause::NotLoaded)
                ) =>
            {
                wall::shown(&self.observation, headings)
            }
            JevStatus::NeedsLogin => Some(
                wall::shown(&self.observation, headings)
                    .unwrap_or_else(|| wall::unauthorized(&self.observation)),
            ),
            _ => None,
        }
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
        // Stopped on the first page, which was described then.
        if self.status == JevStatus::AccessDenied || self.wall.is_some() {
            return stopped_because;
        }
        self.facts = self.describe().await;
        if self.status == JevStatus::Blocked
            && let Some(reason) = self.stop_refused()
        {
            return Some(reason);
        }
        let Some(wall) = self.sign_in_wall() else {
            return stopped_because;
        };
        let reason = match self.status {
            // An embedder's own reason already says it.
            JevStatus::NeedsLogin => stopped_because,
            _ => Some(wall.reason(stopped_because.as_deref())),
        };
        self.status = JevStatus::NeedsLogin;
        self.wall = Some(wall);
        reason
    }
}
