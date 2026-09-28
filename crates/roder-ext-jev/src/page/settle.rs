//! Waiting for the page to finish reacting before it is read.
//!
//! Jev-owned, and a deliberate divergence from upstream, whose settle waits
//! two animation frames or 50 ms and so reads XHR and SPA re-renders half
//! drawn. Here `track.js` keeps a quiet clock inside the page (a
//! MutationObserver plus capture listeners for input), and `settle.js` waits
//! in one `awaitPromise` evaluate for a ready, quiet page with no loading
//! indicator on screen. The idea follows fastbrowse's settle (MIT).

use std::time::Duration;

use serde_json::{Value, json};

use super::{LOAD_TIMEOUT, Page};
use crate::engine::StaleObservation;

pub(super) const TRACK_JS: &str = include_str!("../assets/track.js");
pub(super) const SETTLE_JS: &str = include_str!("../assets/settle.js");
const READY_JS: &str = include_str!("../assets/ready.js");

/// The whole settle, however many documents it spans.
pub(super) const SETTLE_CAP: Duration = Duration::from_millis(2500);
/// A visible loading indicator holds the settle only this long, so a page
/// that spins forever does not pay the whole cap on every step.
const LOADING_GRACE: Duration = Duration::from_millis(1500);
const NAVIGATION_PAUSE: Duration = Duration::from_millis(20);
/// How long past its own cap an in-page wait may run before Rust gives up on
/// it. The page's timers enforce the cap, but a throttled or blocked page may
/// not run them, and the DevTools call would otherwise wait 30 s.
const CAP_SLACK: Duration = Duration::from_millis(250);

/// Why an evaluate failed when the document it ran in went away.
pub(super) const NAVIGATED: &str = "Document navigated during evaluation";

/// Chrome's protocol errors for an evaluate whose document was replaced.
pub(super) fn is_navigation(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    [
        "Execution context was destroyed",
        "Cannot find context with specified id",
        "Inspected target navigated or closed",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn navigated(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<StaleObservation>()
        .is_some_and(|stale| stale.to_string() == NAVIGATED)
}

impl Page {
    /// Put the quiet clock on every document this tab loads from now on.
    pub(super) async fn install_tracker(&mut self) -> anyhow::Result<()> {
        self.call(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": TRACK_JS}),
        )
        .await
        .map(|_| ())
    }

    /// Wait for the document to be past `loading`. Upstream proceeds after
    /// the timeout regardless, and so does this: a slow page is observed as
    /// it stands.
    pub(super) async fn await_ready(&mut self) {
        let deadline = tokio::time::Instant::now() + LOAD_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return;
            }
            let state = tokio::time::timeout(
                remaining + CAP_SLACK,
                self.evaluate_async(&format!("{READY_JS}({})", remaining.as_millis())),
            )
            .await;
            match state {
                // The page never answered in time: observe it as it stands.
                Err(_) => return,
                Ok(Ok(state)) if state.as_str() != Some("loading") => return,
                // The document was replaced mid-wait, or not yet reachable.
                Ok(_) => tokio::time::sleep(NAVIGATION_PAUSE).await,
            }
        }
    }

    /// Wait inside the page until it is ready and quiet, for at most
    /// [`SETTLE_CAP`]. A navigation destroys the waiting promise with its
    /// document; that is progress, so the wait carries on in the new one.
    /// Read-only, so every other failure just ends the wait. The cap is also
    /// held on this side, so a page whose timers stall cannot overrun it by
    /// more than [`CAP_SLACK`]. Returns the page's `{reason, waited_ms}` for
    /// the last document waited on, `{reason: "cap"}` when Rust stopped the
    /// wait, or null.
    pub(crate) async fn settle_page(&mut self, action: Option<&Value>) -> Value {
        let started = tokio::time::Instant::now();
        let mut action = action.cloned();
        loop {
            let waited = started.elapsed();
            let Some(remaining) = SETTLE_CAP.checked_sub(waited) else {
                return Value::Null;
            };
            let options = json!({
                "action": action,
                "capMs": remaining.as_millis() as u64,
                "loadingMs": LOADING_GRACE.saturating_sub(waited).as_millis() as u64,
            });
            let script = format!("{TRACK_JS}\n{SETTLE_JS}({options})");
            let wait = self.evaluate_async(&script);
            let Ok(outcome) = tokio::time::timeout(remaining + CAP_SLACK, wait).await else {
                // The stale reply is skipped by the next call's id match.
                return json!({"reason": "cap", "waited_ms": started.elapsed().as_millis() as u64});
            };
            match outcome {
                Ok(outcome) => return outcome,
                Err(error) if navigated(&error) => {
                    // The acted-on node belonged to the old document.
                    action = None;
                    tokio::time::sleep(NAVIGATION_PAUSE).await;
                }
                Err(_) => return Value::Null,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromes_lost_context_errors_read_as_navigation() {
        for message in [
            "Runtime.evaluate failed: Execution context was destroyed.",
            "Runtime.evaluate failed: Cannot find context with specified id",
            "Runtime.evaluate failed: Inspected target navigated or closed",
        ] {
            assert!(is_navigation(&anyhow::anyhow!("{message}")), "{message}");
        }
        assert!(!is_navigation(&anyhow::anyhow!(
            "Runtime.evaluate timed out"
        )));
        assert!(navigated(&StaleObservation::new(NAVIGATED).into()));
        assert!(!navigated(
            &StaleObservation::new("Document changed during evaluation").into()
        ));
    }
}
