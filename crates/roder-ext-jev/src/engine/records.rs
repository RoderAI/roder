//! What a run leaves behind: its steps, its decisions and its result.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{JevDialog, JevStatus, JevStopCause};
use crate::secret::Secrets;
use crate::usage::JevUsage;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JevActionRecord {
    pub step: usize,
    pub action: String,
    pub kind: String,
    /// The card, row or section the control sat in, when the page named one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// What a fill typed; `[secret]` for a password or one-time-code field.
    pub text: Option<String>,
    pub url: String,
    pub page_changed: Option<bool>,
    /// Another element covered the target, so nothing was clicked or typed.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub covered: bool,
    /// Why the page did not keep what the action asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
    /// The target was covered and Jev dismissed what covered it first:
    /// how (Escape, the layer's close control, a press outside it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uncovered: Option<String>,
    /// Dialogs the page opened after this action, and how Jev answered.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dialogs: Vec<JevDialog>,
    /// This action opened a new tab, which the run then followed: it is the
    /// page the steps after it act on.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub opened_tab: bool,
    /// What the action visibly did: the address it went to, the controls
    /// whose value or state changed, and those it showed or removed. Page
    /// text, as untrusted as the rest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    pub elapsed_ms: u64,
    /// Detailed planner telemetry is available to embedded hosts without
    /// expanding the compact `jev_browse` tool result.
    #[serde(skip)]
    pub choice: String,
    #[serde(skip)]
    pub probability: Value,
    /// The operation head's confidence.
    #[serde(skip)]
    pub confidence: f64,
    #[serde(skip)]
    pub target_confidence: Option<f64>,
    #[serde(skip)]
    pub decision_latency_ms: u64,
    #[serde(skip)]
    pub text_helper: Option<String>,
    #[serde(skip)]
    pub text_latency_ms: u64,
    #[serde(skip)]
    pub operation: String,
    #[serde(skip)]
    pub target: Option<String>,
    #[serde(skip)]
    pub usage: Value,
    #[serde(skip)]
    pub executed_ms: u64,
}

#[cfg(test)]
impl JevActionRecord {
    /// A step that clicked or typed into `label`, for tests.
    pub(crate) fn for_tests(kind: &str, label: &str) -> Self {
        Self {
            step: 1,
            action: label.into(),
            kind: kind.into(),
            context: None,
            text: None,
            url: String::new(),
            page_changed: Some(false),
            covered: false,
            refused: None,
            uncovered: None,
            dialogs: Vec::new(),
            opened_tab: false,
            effect: None,
            elapsed_ms: 0,
            choice: "e1".into(),
            probability: Value::Null,
            confidence: 1.0,
            target_confidence: None,
            decision_latency_ms: 0,
            text_helper: None,
            text_latency_ms: 0,
            operation: "CLICK".into(),
            target: None,
            usage: Value::Null,
            executed_ms: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JevDecisionRecord {
    pub choice: String,
    pub operation: String,
    pub target: Option<String>,
    pub confidence: f64,
    pub target_confidence: Option<f64>,
    pub probabilities: Map<String, Value>,
    pub latency_ms: u64,
    pub usage: Value,
    pub model: Option<String>,
    /// The irreversible-action gate's answer for the chosen action.
    pub irreversible: Option<f64>,
}

/// Typed outcome from one bounded JEV run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JevRunResult {
    pub status: JevStatus,
    pub url: String,
    pub title: String,
    pub visible_text: String,
    pub actions: Vec<JevActionRecord>,
    pub elapsed_ms: u64,
    pub observed_elements: usize,
    pub model_calls: usize,
    pub text_calls: usize,
    /// Summed decision and text-helper tokens, `"unknown"` where a provider
    /// did not report them.
    pub usage: JevUsage,
    #[serde(skip)]
    pub decisions: Vec<JevDecisionRecord>,
    pub stopped_because: Option<String>,
    /// What ended a run that stopped short of its goal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_cause: Option<JevStopCause>,
    pub untrusted: bool,
    /// The controls the final page offered Jev, in the order it observed
    /// them (on screen first). Page text, as untrusted as the rest.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<JevControl>,
    /// What else the final page showed: its HTTP status, headings and the
    /// text of frames Jev reads but cannot act in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<JevPageFacts>,
    /// Every secret typed so far, this run's and the ones it started with,
    /// for the next run on the same tab (see `JevEngineConfig::with_secrets`).
    #[serde(skip)]
    pub(crate) typed_secrets: Secrets,
}

impl JevRunResult {
    /// A run that ended before the loop started: nothing observed, nothing
    /// done, and the start URL as the only page it knows.
    pub(crate) fn before_start(
        status: JevStatus,
        url: &str,
        elapsed: Duration,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            status,
            url: url.into(),
            title: String::new(),
            visible_text: String::new(),
            actions: Vec::new(),
            elapsed_ms: elapsed.as_millis() as u64,
            observed_elements: 0,
            model_calls: 0,
            text_calls: 0,
            usage: JevUsage::none(),
            decisions: Vec::new(),
            stopped_because: Some(reason.into()),
            stop_cause: (status == JevStatus::Blocked).then_some(JevStopCause::NotLoaded),
            untrusted: true,
            controls: Vec::new(),
            page: None,
            typed_secrets: Secrets::default(),
        }
    }
}

/// One control the final page offered, as the caller reads it. A secret
/// field's value is never included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JevControl {
    pub label: String,
    /// `click`, `fill` or `select`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// The card, row or section that tells it apart from controls that read
    /// the same.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// The nearest heading before it on the page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// A field's text, a select's chosen option, or a toggle's state, cut to
    /// 60 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// A select's options, at most 12.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub offscreen: bool,
}

/// What a page shows besides its text and controls, as far as the browser
/// can tell ([`super::JevBrowser::describe`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JevPageFacts {
    /// The main document's HTTP status, when the browser reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// Visible headings, at most 12 of 80 characters each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headings: Vec<String>,
    /// Text of the frames Jev reads but does not act in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<JevFrameText>,
}

/// The text of one frame drawn over or in the page whose document Jev cannot
/// reach from it (another origin): read only, never offered as actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JevFrameText {
    pub origin: String,
    pub text: String,
}
