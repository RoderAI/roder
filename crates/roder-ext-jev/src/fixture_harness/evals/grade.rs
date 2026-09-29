//! Graders over what a run leaves behind: its status, the final page, the
//! posted forms, the DOM, and the executed trace.

use std::collections::BTreeMap;

use anyhow::ensure;
use serde::Deserialize;
use serde_json::{Value, json};

use super::Outcome;

/// Every field is optional; an absent one is not checked.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Expect {
    /// The final status as the result serialises it: `done`, `blocked`,
    /// `budget_exceeded`, `timed_out`, `needs_input`, `unavailable`,
    /// `error`, `needs_confirmation` or `access_denied`; or a list of
    /// statuses any of which passes, where the task's intent allows more
    /// than one honest ending.
    pub(crate) status: Option<Statuses>,
    /// Whether the run ended with a `stopped_because` reason.
    pub(crate) stopped: Option<bool>,
    pub(crate) stopped_because_contains: Option<String>,
    pub(crate) url_ends_with: Option<String>,
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) text_contains: Vec<String>,
    /// Exactly these form submissions, in order; `[]` means none.
    pub(crate) posts: Option<Vec<ExpectedPost>>,
    /// A JavaScript expression read from the final document, to its value.
    #[serde(default)]
    pub(crate) dom: BTreeMap<String, Value>,
    /// The executed trace as `"<kind> <label>"`.
    pub(crate) actions: Option<Vec<String>>,
    /// Steps, as `"<kind> <label>"`, the trace holds somewhere, for a step
    /// whose place in it depends on timing (a banner refused in the page's
    /// own time).
    #[serde(default)]
    pub(crate) actions_include: Vec<String>,
    /// Executed actions, when listing them would be noise.
    pub(crate) acts: Option<usize>,
    pub(crate) model_calls: Option<usize>,
    /// Per executed action, whether a covering element refused it.
    pub(crate) covered: Option<Vec<bool>>,
    /// Per executed action, whether the page changed after it.
    pub(crate) page_changed: Option<Vec<bool>>,
}

/// One status, or several any of which passes.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum Statuses {
    One(String),
    Any(Vec<String>),
}

impl Statuses {
    fn all(&self) -> &[String] {
        match self {
            Self::One(status) => std::slice::from_ref(status),
            Self::Any(statuses) => statuses,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExpectedPost {
    pub(crate) path: String,
    /// Decoded form fields that must match; others are ignored.
    #[serde(default)]
    pub(crate) fields: BTreeMap<String, String>,
    /// Fields holding a secret: a mismatch names them without quoting
    /// either value, so a failure row never carries the secret.
    #[serde(default)]
    pub(crate) secret_fields: Vec<String>,
}

impl Expect {
    pub(crate) fn check(&self) -> anyhow::Result<()> {
        for status in self.status.iter().flat_map(Statuses::all) {
            ensure!(
                [
                    "done",
                    "blocked",
                    "budget_exceeded",
                    "timed_out",
                    "needs_input",
                    "unavailable",
                    "error",
                    "needs_confirmation",
                    "access_denied",
                ]
                .contains(&status.as_str()),
                "unknown status {status:?}"
            );
        }
        ensure!(
            self.status
                .as_ref()
                .is_none_or(|statuses| !statuses.all().is_empty()),
            "an empty status list passes nothing"
        );
        ensure!(
            !(self.stopped == Some(false) && self.stopped_because_contains.is_some()),
            "stopped: false contradicts stopped_because_contains"
        );
        Ok(())
    }

    /// Expressions to read from the final document.
    pub(crate) fn probes(&self) -> impl Iterator<Item = &String> {
        self.dom.keys()
    }

    /// Every way the outcome misses this expectation; empty means a pass.
    pub(crate) fn grade(&self, outcome: &Outcome) -> Vec<String> {
        let result = &outcome.result;
        let mut failures = Vec::new();
        let mut check = |ok: bool, failure: String| {
            if !ok {
                failures.push(failure);
            }
        };
        if let Some(statuses) = &self.status {
            let actual = serde_json::to_value(result.status).unwrap_or(Value::Null);
            let failure = match statuses {
                Statuses::One(status) => format!("status {actual} != {status:?}"),
                Statuses::Any(statuses) => format!("status {actual} not in {statuses:?}"),
            };
            check(
                statuses.all().iter().any(|status| actual == json!(status)),
                failure,
            );
        }
        if let Some(stopped) = self.stopped {
            check(
                result.stopped_because.is_some() == stopped,
                format!("stopped_because {:?}", result.stopped_because),
            );
        }
        if let Some(needle) = &self.stopped_because_contains {
            check(
                result
                    .stopped_because
                    .as_deref()
                    .is_some_and(|reason| reason.contains(needle.as_str())),
                format!(
                    "stopped_because {:?} lacks {needle:?}",
                    result.stopped_because
                ),
            );
        }
        if let Some(suffix) = &self.url_ends_with {
            check(
                result.url.ends_with(suffix.as_str()),
                format!("url {} does not end with {suffix}", result.url),
            );
        }
        if let Some(title) = &self.title {
            check(
                &result.title == title,
                format!("title {:?} != {title:?}", result.title),
            );
        }
        for needle in self
            .text_contains
            .iter()
            .map(|needle| super::sessions::expand(needle))
        {
            check(
                result.visible_text.contains(needle.as_str()),
                format!("visible text lacks {needle:?}"),
            );
        }
        if let Some(expected) = &self.posts {
            // Paths only: a body may hold a secret.
            check(
                outcome.posts.len() == expected.len(),
                format!(
                    "{} posts, expected {}: {:?}",
                    outcome.posts.len(),
                    expected.len(),
                    outcome
                        .posts
                        .iter()
                        .map(|post| &post.path)
                        .collect::<Vec<_>>()
                ),
            );
            for (post, want) in outcome.posts.iter().zip(expected) {
                check(
                    post.path == want.path,
                    format!("posted to {}, expected {}", post.path, want.path),
                );
                for (field, value) in &want.fields {
                    let actual = post.field(field);
                    let failure = match want.secret_fields.contains(field) {
                        true => format!("posted {field} is not the expected secret"),
                        false => format!("posted {field}={actual:?}, expected {value:?}"),
                    };
                    check(actual.as_deref() == Some(value.as_str()), failure);
                }
            }
        }
        for (expression, want) in &self.dom {
            match outcome.probed.dom.get(expression) {
                Some(Ok(actual)) => check(
                    actual == want,
                    format!("dom `{expression}` = {actual}, expected {want}"),
                ),
                Some(Err(error)) => check(false, format!("dom `{expression}`: {error}")),
                None => check(false, format!("dom `{expression}` was not read")),
            }
        }
        if let Some(actions) = &self.actions {
            let actual = result
                .actions
                .iter()
                .map(|action| format!("{} {}", action.kind, action.action))
                .collect::<Vec<_>>();
            check(
                &actual == actions,
                format!("actions {actual:?} != {actions:?}"),
            );
        }
        let trace = result
            .actions
            .iter()
            .map(|action| format!("{} {}", action.kind, action.action))
            .collect::<Vec<_>>();
        for step in &self.actions_include {
            check(
                trace.contains(step),
                format!("actions {trace:?} lack {step:?}"),
            );
        }
        if let Some(acts) = self.acts {
            check(
                result.actions.len() == acts,
                format!("{} actions, expected {acts}", result.actions.len()),
            );
        }
        if let Some(calls) = self.model_calls {
            check(
                result.model_calls == calls,
                format!("{} model calls, expected {calls}", result.model_calls),
            );
        }
        if let Some(covered) = &self.covered {
            let actual = result
                .actions
                .iter()
                .map(|action| action.covered)
                .collect::<Vec<_>>();
            check(
                &actual == covered,
                format!("covered {actual:?} != {covered:?}"),
            );
        }
        if let Some(changed) = &self.page_changed {
            let actual = result
                .actions
                .iter()
                .map(|action| action.page_changed.unwrap_or(false))
                .collect::<Vec<_>>();
            check(
                &actual == changed,
                format!("page_changed {actual:?} != {changed:?}"),
            );
        }
        failures
    }
}
