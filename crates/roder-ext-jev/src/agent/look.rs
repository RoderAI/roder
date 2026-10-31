//! Looking at a page before believing what it seems to say: an empty first
//! read is read again, and a page that refused automated access ends the
//! run before any decision is spent on it (see [`crate::block`]). A run
//! that stops short at a sign-in it cannot make ends `needs_login` with the
//! wall it stands at (see [`crate::wall`]).
//!
//! The opening page gets two more chances before it is believed, both
//! costing no decision. A start page that still shows nothing after the
//! reads is loaded once more, unless it was served as an error: a cold app
//! can leave its first document blank. And a page that may clear itself (a
//! challenge, or a 503, with nothing to act on; see [`block::may_clear`]) is
//! read again each time it changes, and at least once a second, for up to
//! 12 s, until it no longer looks like one: a bot check that runs its script
//! and moves on, or a busy site that retries, ended runs `access_denied` or
//! `blocked` that waiting finishes. Each new content is described again, so a ticking countdown on
//! the same challenge does not end the wait. The rules follow fastbrowse's
//! agent (MIT, `driving.rs outwait`, `settle_blank_opening`).

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
/// How long a page that may clear itself is watched.
const INTERSTITIAL_WAIT: Duration = Duration::from_secs(12);
/// The longest it goes unread meanwhile: it is read again as soon as it
/// changes, and at least this often.
const INTERSTITIAL_POLL: Duration = Duration::from_secs(1);

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

    /// The page's HTTP status, as last described.
    fn http_status(&self) -> Option<u16> {
        self.facts.as_ref().and_then(|facts| facts.http_status)
    }

    /// Whether the page refused automated access, on its own evidence.
    fn block(&self) -> Option<block::AccessBlock> {
        block::classify(
            self.http_status(),
            self.observation["url"].as_str().unwrap_or_default(),
            self.observation["title"].as_str().unwrap_or_default(),
            self.observation["text"].as_str().unwrap_or_default(),
            self.elements(),
        )
    }

    /// When the page refused automated access, or answered 401 with nothing
    /// to act on, end the run `access_denied` or `needs_login` and say why.
    pub(super) fn stop_refused(&mut self) -> Option<String> {
        let block = self.block()?;
        self.status = block.status();
        if block.status() == JevStatus::NeedsLogin {
            self.wall = Some(wall::unauthorized(&self.observation));
        }
        Some(self.secrets.scrub(&block.reason()))
    }

    /// Look at the opening page before any decision: read an empty one
    /// again, load a start page that still shows nothing once more, and
    /// watch one that may clear itself. Leaves it described, for
    /// [`Self::stop_refused`].
    pub(super) async fn look_at_opening(&mut self) -> anyhow::Result<()> {
        self.look_again_while_empty().await?;
        self.facts = self.describe().await;
        self.reload_blank_opening().await?;
        self.outwait().await?;
        self.elapsed();
        Ok(())
    }

    /// A start page that still shows nothing is loaded once more, when it
    /// is a web page that was not served as an error.
    async fn reload_blank_opening(&mut self) -> anyhow::Result<()> {
        let url = self.observation["url"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let web = url.starts_with("http://") || url.starts_with("https://");
        let served = self.http_status().is_none_or(|status| status < 400);
        if !self.shows_nothing() || !web || !served {
            return Ok(());
        }
        // A load that fails leaves an error page to read, not a failed run:
        // the blank page would have been read as it stood.
        if !self.browser.reload(&url).await.unwrap_or(true) {
            return Ok(());
        }
        match self.observe().await {
            Err(error) if !error.is::<StaleObservation>() => return Err(error),
            _ => {}
        }
        self.look_again_while_empty().await?;
        self.facts = self.describe().await;
        Ok(())
    }

    /// Whether the page offers nothing to act on and may clear itself.
    fn may_clear(&self) -> bool {
        block::may_clear(self.http_status(), self.block().as_ref(), self.elements())
    }

    /// Watch a page that may clear itself, reading it again until it no
    /// longer may, for up to [`INTERSTITIAL_WAIT`]. New content is described
    /// again; the same content is not.
    async fn outwait(&mut self) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + INTERSTITIAL_WAIT;
        let shown = |observation: &serde_json::Value| {
            (
                observation["url"].clone(),
                observation["fingerprint"].clone(),
            )
        };
        let mut seen = shown(&self.observation);
        while self.may_clear() {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            self.browser
                .wait_for_change(INTERSTITIAL_POLL.min(remaining))
                .await?;
            match self.observe().await {
                Ok(()) => {}
                // On its way to the next document.
                Err(error) if error.is::<StaleObservation>() => continue,
                Err(error) => return Err(error),
            }
            if shown(&self.observation) != seen {
                seen = shown(&self.observation);
                self.facts = self.describe().await;
            }
        }
        Ok(())
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
