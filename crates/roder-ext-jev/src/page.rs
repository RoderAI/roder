//! Observing and acting on one tab.
//!
//! A port of upstream Jev's `browser.py`. The scripts that must run inside
//! the page — the observation snapshot and the hit-tested act resolver among
//! them — live under `assets/`; they started as upstream's and are now
//! Jev-owned (both keep offscreen controls, and act.js scrolls to and
//! multi-point hit-tests its target). Everything around them is Rust over a
//! direct CDP connection. Waiting for the page to settle ([`settle`]) and
//! executing an action ([`act`], [`fill`]) are Jev's own.

mod act;
mod consent;
mod fill;
mod fingerprint;
mod frames;
mod open;
mod resume;
mod settle;
mod tabs;
mod uncover;

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::cdp::Connection;
use crate::engine::{JevActOutcome, JevBrowser, JevDialog, JevPageFacts, StaleObservation};
use fingerprint::fingerprint_with_frames;
pub(crate) use open::{LoadFailed, close_target, create_target};
pub(crate) use resume::TabsGone;
use settle::{NAVIGATED, TRACK_JS, is_navigation};
pub(crate) use tabs::{OwnedTab, TabLedger};
use tabs::{SharedLedger, Tab};

/// The observation: `snapshot.js` called with `tree.js`'s helpers for the
/// composed tree (open shadow roots and same-origin frames) and its parts:
/// `names.js` (naming), `reach.js` (what scrolling can reach) and `text.js`
/// (the page text).
pub(crate) const SNAPSHOT_JS: &str = concat!(
    "(",
    include_str!("assets/snapshot.js"),
    ")(",
    include_str!("assets/tree.js"),
    ",{names:",
    include_str!("assets/names.js"),
    ",reach:",
    include_str!("assets/reach.js"),
    ",text:",
    include_str!("assets/text.js"),
    "})"
);
/// Names controls that read alike; applied to the snapshot in `observe` only,
/// so `fresh()` never pays for it.
pub(crate) const CONTEXT_JS: &str = include_str!("assets/context.js");

const LOAD_TIMEOUT: Duration = Duration::from_secs(15);
const OBSERVE_ATTEMPTS: usize = 10;

pub(crate) struct Page {
    connection: Connection,
    /// Every tab Jev owns and reads, in the order they opened; the last is
    /// the one the run is on.
    tabs: Vec<Tab>,
    /// Tabs Jev's tabs opened that it has not attached to, closed with it.
    stray: Vec<String>,
    /// Tabs already open when a continued call re-attached: opened between
    /// calls (the user's own clicks, a page's timer), never adopted or
    /// closed as if this call's inputs had opened them.
    seen: Vec<String>,
    /// Whether the task's tab is shown, so an adopted one is shown too.
    foreground: bool,
    /// The action whose effects still have to settle before the next read.
    after_input: Option<Value>,
    /// The last action opened a tab that the run now reads.
    opened_tab: bool,
    /// Inject DuckDuckGo's autoconsent into every document (see `consent`).
    autoconsent: bool,
    /// The tabs above as the page's owner reads them (see `tabs`).
    ledger: SharedLedger,
    /// Read the text of frames of another origin (see `frames`).
    frame_text: bool,
}

impl Page {
    fn tab(&self) -> &Tab {
        self.tabs.last().expect("a page always has a tab")
    }

    pub(crate) async fn call(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let session = self.tab().session.clone();
        self.connection.call(method, params, Some(&session)).await
    }

    /// Bring the tab to the front.
    pub(crate) async fn activate(&mut self) -> anyhow::Result<()> {
        let target_id = self.tab().target.clone();
        self.connection
            .call(
                "Target.activateTarget",
                json!({"targetId": target_id}),
                None,
            )
            .await
            .map(|_| ())
    }

    /// The tab's DevTools target, so a caller can close a tab it handed away.
    pub(crate) fn target_id(&self) -> &str {
        &self.tab().target
    }

    /// Close every tab Jev owns.
    pub(crate) async fn close(&mut self) -> anyhow::Result<()> {
        self.close_all().await
    }

    /// `Runtime.evaluate`, mapping a page-side exception, or the document
    /// going away mid-call, to a stale page.
    pub(crate) async fn evaluate(&mut self, expression: &str) -> anyhow::Result<Value> {
        self.run_script(expression, false, false).await
    }

    pub(crate) async fn evaluate_async(&mut self, expression: &str) -> anyhow::Result<Value> {
        self.run_script(expression, true, false).await
    }

    /// Run the snapshot, with DevTools' command-line API in scope so it can
    /// read each element's listeners with `getEventListeners`.
    pub(crate) async fn evaluate_snapshot(&mut self, expression: &str) -> anyhow::Result<Value> {
        self.run_script(expression, false, true).await
    }

    async fn run_script(
        &mut self,
        expression: &str,
        await_promise: bool,
        command_line: bool,
    ) -> anyhow::Result<Value> {
        let mut params = json!({
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": await_promise,
        });
        if command_line {
            params["includeCommandLineAPI"] = json!(true);
        }
        let result = self
            .call("Runtime.evaluate", params)
            .await
            .map_err(|error| {
                if is_navigation(&error) {
                    StaleObservation::new(NAVIGATED).into()
                } else if tabs::is_gone(&error) && self.return_to_opener() {
                    StaleObservation::new("The tab closed; back on the tab that opened it").into()
                } else {
                    error
                }
            })?;
        if result.get("exceptionDetails").is_some() {
            return Err(StaleObservation::new("Document changed during evaluation").into());
        }
        Ok(result["result"]["value"].clone())
    }

    /// Let the last input's effects land before the next read. A no-op when
    /// nothing is pending, so timing it separately from `observe` is exact.
    pub(crate) async fn settle(&mut self) {
        if let Some(action) = self.after_input.take() {
            self.settle_page(Some(&action)).await;
            self.follow_popup().await;
        }
    }

    /// Read the page: settle any pending input, snapshot, then fingerprint.
    /// The snapshot also (re)installs the quiet clock, for a document that
    /// loaded before it was registered. Dialogs answered since the last read
    /// are added as `dialogs` and as lines before the page text, after the
    /// fingerprint is taken: a dismissed confirm is news for the model but
    /// not a change to the page, so it cannot hide a stall.
    pub(crate) async fn observe(&mut self) -> anyhow::Result<Value> {
        self.settle().await;
        let snapshot = format!("{TRACK_JS}\n({CONTEXT_JS})({SNAPSHOT_JS})");
        for attempt in 0..OBSERVE_ATTEMPTS {
            match self.evaluate_snapshot(&snapshot).await {
                Ok(Value::Null) => {
                    return Err(StaleObservation::new("Document is navigating").into());
                }
                Ok(mut observation) => {
                    let read = match self.frame_text {
                        true => self.read_frames().await,
                        false => Vec::new(),
                    };
                    // Taken before the frames' text joins the page text: a
                    // frame's text is not the page's progress (see
                    // `fingerprint_with_frames`).
                    let fingerprint = fingerprint_with_frames(&observation, &read);
                    frames::add_frames(&mut observation, read);
                    observation["fingerprint"] = json!(fingerprint);
                    add_dialogs(&mut observation, self.connection.take_dialogs());
                    self.add_opened_tab(&mut observation);
                    return Ok(observation);
                }
                Err(error) if error.is::<StaleObservation>() && attempt + 1 < OBSERVE_ATTEMPTS => {
                    // Most often a navigation: wait for the new document.
                    self.settle_page(None).await;
                }
                Err(error) => return Err(error),
            }
        }
        Err(StaleObservation::new("Page did not settle").into())
    }

    /// Has the page kept the meaning the decision was made against?
    pub(crate) async fn fresh(
        &mut self,
        observation: &Value,
        action: Option<&Value>,
    ) -> anyhow::Result<bool> {
        if let Some(action) = action
            && matches!(action["kind"].as_str(), Some("click") | Some("select"))
        {
            let Some(node) = action["node"].as_i64() else {
                return Ok(false);
            };
            let current = self
                .evaluate(&format!(
                    "(() => {{ const c=window.__jevFast; \
                     return c?.pageKey ? [c.pageKey(),c.guard(c.nodes.get({node}))] : null; }})()"
                ))
                .await?;
            let expected = json!([
                observation["page_key"],
                observation["guards"]
                    .get(node.to_string())
                    .cloned()
                    .unwrap_or(Value::Null),
            ]);
            return Ok(current == expected);
        }
        let marker = self
            .evaluate_snapshot(&format!(
                "(() => {{ const state={SNAPSHOT_JS}; return state?.marker ?? null; }})()"
            ))
            .await?;
        Ok(marker == observation["marker"])
    }
}

#[async_trait]
impl JevBrowser for Page {
    async fn observe(&mut self) -> anyhow::Result<Value> {
        Page::observe(self).await
    }

    async fn fresh(&mut self, observation: &Value, action: Option<&Value>) -> anyhow::Result<bool> {
        Page::fresh(self, observation, action).await
    }

    async fn act(
        &mut self,
        action: &Value,
        observation: &Value,
        text: Option<&str>,
        wait: Duration,
    ) -> anyhow::Result<JevActOutcome> {
        Page::act(self, action, observation, text, wait).await
    }

    async fn close(&mut self) -> anyhow::Result<()> {
        Page::close(self).await
    }

    async fn refuse_cookie_banner(&mut self) -> anyhow::Result<Option<String>> {
        Page::refuse_cookie_banner(self).await
    }

    async fn describe(&mut self) -> anyhow::Result<Option<JevPageFacts>> {
        let facts = self.evaluate(DESCRIBE_JS).await?;
        Ok(serde_json::from_value(facts).ok())
    }
}

/// The main document's HTTP status and visible headings.
const DESCRIBE_JS: &str = include_str!("assets/describe.js");

/// Page text cap, as `snapshot.js` applies it.
pub(crate) const TEXT_CHARS: usize = 6000;

/// Put answered dialogs on an observation: as `dialogs`, and as one line
/// each before the page text, where the model reads page content.
pub(crate) fn add_dialogs(observation: &mut Value, dialogs: Vec<JevDialog>) {
    if dialogs.is_empty() {
        return;
    }
    let mut lines = dialogs
        .iter()
        .map(|dialog| {
            let answer = if dialog.accepted {
                "accepted"
            } else {
                "dismissed"
            };
            format!(
                "[{} dialog, {answer} by Jev] {}",
                dialog.kind, dialog.message
            )
        })
        .collect::<Vec<_>>();
    if let Some(text) = observation["text"].as_str().filter(|text| !text.is_empty()) {
        lines.push(text.to_string());
    }
    let text = lines
        .join("\n")
        .chars()
        .take(TEXT_CHARS)
        .collect::<String>();
    observation["text"] = json!(text);
    observation["dialogs"] = json!(dialogs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialogs_come_before_the_page_text_and_leave_the_fingerprint() {
        let mut page = json!({"text": "Drafts\nQ3 notes", "fingerprint": "f"});
        add_dialogs(&mut page, Vec::new());
        assert_eq!(
            page,
            json!({"text": "Drafts\nQ3 notes", "fingerprint": "f"})
        );

        let dialogs = vec![
            JevDialog {
                kind: "confirm".into(),
                message: "Delete Q3 notes?".into(),
                accepted: false,
            },
            JevDialog {
                kind: "alert".into(),
                message: "Saved".into(),
                accepted: true,
            },
        ];
        add_dialogs(&mut page, dialogs);
        assert_eq!(
            page["text"],
            "[confirm dialog, dismissed by Jev] Delete Q3 notes?\n\
             [alert dialog, accepted by Jev] Saved\nDrafts\nQ3 notes"
        );
        assert_eq!(page["dialogs"][0]["type"], "confirm");
        assert_eq!(page["dialogs"][1]["accepted"], true);
        assert_eq!(page["fingerprint"], "f");

        let mut long = json!({"text": "x".repeat(TEXT_CHARS)});
        add_dialogs(
            &mut long,
            vec![JevDialog {
                kind: "alert".into(),
                message: "Hi".into(),
                accepted: true,
            }],
        );
        let text = long["text"].as_str().unwrap();
        assert_eq!(text.chars().count(), TEXT_CHARS);
        assert!(text.starts_with("[alert dialog, accepted by Jev] Hi\n"));
    }

    #[test]
    fn in_page_scripts_keep_their_load_bearing_calls() {
        // Guards against an edit that would silently change observation.
        assert!(SNAPSHOT_JS.contains("window.__jevFast"));
        assert!(SNAPSHOT_JS.contains("checkVisibility"));
        // The press re-check drops the guard's last entry as the text around
        // the control, so that entry must stay last.
        assert!(SNAPSHOT_JS.contains("cut(scope?.innerText||'',6000).toWellFormed()];"));
        assert!(act::ACT_JS.contains("tree.hit(x,y)"));
        assert!(SNAPSHOT_JS.contains("document.elementFromPoint(x,y)"));
        assert!(SNAPSHOT_JS.contains("shadowRoot?.elementFromPoint"));
        assert!(act::FRAME_JS.contains("requestAnimationFrame"));
        assert!(fill::FILL_JS.contains("tree.active"));
        assert!(SNAPSHOT_JS.contains("shadowRoot?.activeElement"));
        assert!(CONTEXT_JS.contains("DOCUMENT_POSITION_FOLLOWING"));
        assert!(settle::SETTLE_JS.contains("requestAnimationFrame"));
        assert!(settle::TRACK_JS.contains("MutationObserver"));
        assert!(consent::REFUSE_JS.contains("__jevConsentChecked"));
        assert!(consent::REFUSE_JS.contains("autoconsentStandalone"));
    }
}
