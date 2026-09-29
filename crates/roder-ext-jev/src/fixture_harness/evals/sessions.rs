//! Session tasks: several `jev_browse` calls on one thread, graded per call
//! on the page reached and on the tab it was reached in.
//!
//! Tasks are data, in `tests/fixtures/evals/sessions.json`. Each call names
//! a fixture page (loaded as the call's url) or none (url `""`: go on from
//! the page the session's tab shows), optionally the `tab` it asks for
//! (`current` when absent), a goal, the keyless tier's plan and
//! what to expect: the status, the page, `tab_note` (`new`, `continued`,
//! `navigated`, `reopened`), `same_tab` (acted in the tab the last call
//! ended in) and `tabs_open`. A task can also expect form posts over all
//! its calls. Both tiers run the calls through the same `JevSessions::call`
//! the tool makes, the keyless one with each call's plan and the live one
//! with the hosted model.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

use super::grade::ExpectedPost;
use super::{Step, StepDecider, TaskValues};
use crate::engine::JevDecisionClient;
use crate::fixture_harness::Harness;
use crate::fixture_harness::sessions::{call_with, targets, test_sessions};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionTask {
    pub(crate) id: String,
    pub(crate) covers: String,
    #[serde(default)]
    pub(crate) values: BTreeMap<String, String>,
    /// Exactly these form submissions over all the calls, in order.
    #[serde(default)]
    pub(crate) posts: Vec<ExpectedPost>,
    pub(crate) calls: Vec<SessionCall>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionCall {
    /// A fixture page to load; absent goes on from the tab's page.
    pub(crate) page: Option<String>,
    /// The `tab` argument: `current` when absent.
    pub(crate) tab: Option<String>,
    pub(crate) goal: String,
    pub(crate) plan: Vec<Step>,
    pub(crate) expect: CallExpect,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallExpect {
    pub(crate) status: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) url_ends_with: Option<String>,
    #[serde(default)]
    pub(crate) text_contains: Vec<String>,
    pub(crate) tab_note: Option<String>,
    /// The call acted in the tab the previous call ended in.
    pub(crate) same_tab: Option<bool>,
    pub(crate) tabs_open: Option<u64>,
    /// Text a frame Jev read but cannot act in must show.
    #[serde(default)]
    pub(crate) frame_text_contains: Vec<String>,
    /// Text the result the caller reads must hold.
    #[serde(default)]
    pub(crate) digest_contains: Vec<String>,
    /// Steps, as `"<kind> <label>"`, the call must not take.
    #[serde(default)]
    pub(crate) forbid_actions: Vec<String>,
}

/// `{today}` in an expected text is today's date as a booking page shows
/// it: "Mon, Sep 28, 2026".
pub(crate) fn expand(expected: &str) -> String {
    expected.replace(
        "{today}",
        &chrono::Local::now().format("%a, %b %-d, %Y").to_string(),
    )
}

impl CallExpect {
    fn grade(&self, n: usize, result: &Value, same_tab: bool) -> Vec<String> {
        let mut failures = Vec::new();
        let mut check = |ok: bool, what: String| {
            if !ok {
                failures.push(format!("call {n}: {what}"));
            }
        };
        if let Some(status) = &self.status {
            check(
                result["status"] == json!(status),
                format!("status {} != {status}", result["status"]),
            );
        }
        if let Some(title) = &self.title {
            check(
                result["title"] == json!(title),
                format!("title {} != {title:?}", result["title"]),
            );
        }
        let url = result["url"].as_str().unwrap_or_default();
        if let Some(end) = &self.url_ends_with {
            check(
                url.ends_with(end.as_str()),
                format!("url {url} does not end with {end}"),
            );
        }
        let text = result["visible_text"].as_str().unwrap_or_default();
        for wanted in self.text_contains.iter().map(|wanted| expand(wanted)) {
            check(
                text.contains(wanted.as_str()),
                format!("text lacks {wanted:?}"),
            );
        }
        let frames = result["page"]["frames"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|frame| frame["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for wanted in self.frame_text_contains.iter().map(|wanted| expand(wanted)) {
            check(
                frames.contains(wanted.as_str()),
                format!("frame text {frames:?} lacks {wanted:?}"),
            );
        }
        if !self.digest_contains.is_empty() {
            let digest = crate::report::tool_text(&mut result.clone());
            for wanted in self.digest_contains.iter().map(|wanted| expand(wanted)) {
                check(
                    digest.contains(wanted.as_str()),
                    format!("result text lacks {wanted:?}"),
                );
            }
        }
        let trace = result["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|action| {
                format!(
                    "{} {}",
                    action["kind"].as_str().unwrap_or_default(),
                    action["action"].as_str().unwrap_or_default()
                )
            })
            .collect::<Vec<_>>();
        for forbidden in &self.forbid_actions {
            check(!trace.contains(forbidden), format!("took {forbidden:?}"));
        }
        let session = &result["session"];
        if let Some(note) = &self.tab_note {
            check(
                session["tab_note"] == json!(note),
                format!("tab_note {} != {note}", session["tab_note"]),
            );
        }
        if let Some(same) = self.same_tab {
            check(same_tab == same, format!("same_tab {same_tab} != {same}"));
        }
        if let Some(open) = self.tabs_open {
            check(
                session["tabs_open"] == json!(open),
                format!("tabs_open {} != {open}", session["tabs_open"]),
            );
        }
        failures
    }
}

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/evals/sessions.json")
}

pub(crate) fn load() -> anyhow::Result<Vec<SessionTask>> {
    let raw = std::fs::read_to_string(path()).context("read sessions.json")?;
    let tasks: Vec<SessionTask> = serde_json::from_str(&raw).context("parse sessions.json")?;
    let pages = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages");
    for task in &tasks {
        ensure!(
            task.calls.len() >= 2,
            "{}: a session task needs two calls",
            task.id
        );
        ensure!(
            task.calls[0].page.is_some(),
            "{}: the first call needs a page",
            task.id
        );
        for call in &task.calls {
            if let Some(page) = &call.page {
                let name = page.split('?').next().unwrap_or_default();
                ensure!(pages.join(name).is_file(), "{}: no page {name}", task.id);
            }
            for step in &call.plan {
                step.check()
                    .with_context(|| format!("{}: bad plan step", task.id))?;
            }
        }
    }
    Ok(tasks)
}

/// Run every call of `task` on one thread, deciding each with `decide`,
/// and return its failures.
pub(crate) async fn run(
    harness: &Harness,
    task: &SessionTask,
    decide: impl Fn(&SessionCall) -> Arc<dyn JevDecisionClient>,
) -> Vec<String> {
    let sessions = test_sessions();
    let text = Arc::new(TaskValues::new(&task.values));
    let mut failures = Vec::new();
    let mut last_tab: Option<String> = None;
    for (index, call) in task.calls.iter().enumerate() {
        let url = call
            .page
            .as_deref()
            .map(|page| harness.site.url(page))
            .unwrap_or_default();
        let tab = call.tab.as_deref().unwrap_or("current");
        let args = json!({"goal": call.goal, "url": url, "tab": tab, "timeout_seconds": 60});
        let result = match call_with(
            harness,
            &sessions,
            &task.id,
            args,
            decide(call),
            Some(text.clone()),
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                failures.push(format!("call {}: {error:#}", index + 1));
                break;
            }
        };
        let tab = targets(&sessions, &task.id).last().cloned();
        let same_tab = last_tab.is_some() && tab == last_tab;
        failures.extend(call.expect.grade(index + 1, &result, same_tab));
        last_tab = tab;
    }
    let posts = harness
        .site
        .wait_for_posts(task.posts.len(), Duration::from_secs(2))
        .await;
    if posts.len() != task.posts.len() {
        failures.push(format!(
            "{} posts, expected {}",
            posts.len(),
            task.posts.len()
        ));
    }
    for (post, expected) in posts.iter().zip(&task.posts) {
        if post.path != expected.path {
            failures.push(format!("post to {} != {}", post.path, expected.path));
        }
        for (field, value) in &expected.fields {
            if post.field(field).as_deref() != Some(value.as_str()) {
                failures.push(format!("post field {field} != {value:?}"));
            }
        }
    }
    let _ = sessions
        .close(
            &task.id,
            tokio::time::Instant::now() + Duration::from_secs(5),
        )
        .await;
    failures
}

#[test]
fn the_session_corpus_is_well_formed() {
    assert!(load().unwrap().len() >= 4);
}

#[tokio::test]
async fn keyless_session_corpus_passes() {
    let tasks = load().unwrap();
    let base = harness_or_skip!();
    let mut failed = Vec::new();
    for task in &tasks {
        let harness = base.with_new_site().await;
        let failures = run(&harness, task, |call| {
            Arc::new(StepDecider::new(&call.plan))
        })
        .await;
        eprintln!(
            "{:<28} {} ({})",
            task.id,
            if failures.is_empty() { "pass" } else { "FAIL" },
            task.covers
        );
        if !failures.is_empty() {
            failed.push(format!("{}: {}", task.id, failures.join("; ")));
        }
    }
    assert!(
        failed.is_empty(),
        "failed session tasks:\n{}",
        failed.join("\n")
    );
}

#[tokio::test]
#[ignore = "live: needs TYPESAFE_API_KEY (or JEV_API_KEY), Chrome, and spends model calls"]
async fn live_session_corpus() {
    let tasks = load().unwrap();
    let Some(key) = super::live::live_key() else {
        eprintln!("skipping: set TYPESAFE_API_KEY (or JEV_API_KEY) to run the live tier");
        return;
    };
    let model = super::live::env("JEV_MODEL").unwrap_or_else(|| "jev-latest".into());
    let Some(base) = Harness::start().await else {
        eprintln!("skipping: no Chrome binary found (set JEV_CHROME_BINARY)");
        return;
    };
    let mut passed = 0;
    for task in &tasks {
        let harness = base.with_new_site().await;
        let failures = run(&harness, task, |_| {
            Arc::new(crate::decide::JevTypeSafeDecisionClient::new(
                key.clone(),
                model.clone(),
            ))
        })
        .await;
        passed += usize::from(failures.is_empty());
        eprintln!(
            "{:<28} {} {}",
            task.id,
            if failures.is_empty() { "pass" } else { "FAIL" },
            failures.join("; ")
        );
    }
    eprintln!("{passed}/{} session tasks passed", tasks.len());
}
