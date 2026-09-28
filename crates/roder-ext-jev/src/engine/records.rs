//! What a run leaves behind: its steps, its decisions and its result.

use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value};

use super::{JevDialog, JevStatus};
use crate::usage::JevUsage;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JevActionRecord {
    pub step: usize,
    pub action: String,
    pub kind: String,
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
    /// Dialogs the page opened after this action, and how Jev answered.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dialogs: Vec<JevDialog>,
    /// This action opened a new tab, which the run then followed: it is the
    /// page the steps after it act on.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub opened_tab: bool,
    /// What the action visibly did: the address it went to, the controls
    /// whose value or state changed, and those it showed or removed.
    #[serde(skip)]
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
    pub untrusted: bool,
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
            untrusted: true,
        }
    }
}
