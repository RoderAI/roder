//! A keyless real-DOM harness: fixture pages, a throwaway headless Chrome,
//! and the real [`Page`] driven by scripted decisions.
//!
//! Nothing else exercises `snapshot.js`, `act.js`, `settle.js` or the CDP
//! input path against a real document. When no Chrome binary is installed,
//! tests here pass without running, and the run says so once on the
//! terminal; with `JEV_REQUIRE_CHROME=1` or `CI` set they fail instead, and
//! a `JEV_CHROME_BINARY` that is not an executable file always fails them
//! (see [`browser::binaries`]).
//!
//! To add a scenario, drop a page into `tests/fixtures/pages/`, open it with
//! [`Harness::open`], and either drive the page directly (use
//! [`timed_step`] to also measure it) or hand it to [`Harness::run`] with a
//! [`scripted::PlanDecider`]. For an end-to-end task graded on its outcome,
//! add it to the eval corpus instead (see [`evals`]).

mod browser;
mod proxy;
mod scripted;
mod sessions;
mod site;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use serde::Serialize;
use serde_json::Value;

use crate::cdp::Connection;
use crate::engine::{
    JevActOutcome, JevEngine, JevEngineConfig, JevRunResult, JevTextValueResolver,
};
use crate::page::Page;
use browser::TestChrome;
use scripted::PlanDecider;
use site::FixtureSite;

pub(crate) struct Harness {
    pub(crate) site: FixtureSite,
    chrome: Arc<TestChrome>,
}

impl Harness {
    /// `None` when Chrome is not installed and not required; any other
    /// failure panics, since a harness that half-starts would hide real
    /// breakage.
    pub(crate) async fn start() -> Option<Self> {
        let chrome = TestChrome::launch().await.expect("start headless Chrome")?;
        let site = FixtureSite::start().await.expect("start the fixture site");
        Some(Self {
            site,
            chrome: Arc::new(chrome),
        })
    }

    /// Like [`Harness::start`], on a Chrome with a real window.
    pub(crate) async fn start_headed() -> Option<Self> {
        let chrome = TestChrome::launch_headed()
            .await
            .expect("start windowed Chrome")?;
        let site = FixtureSite::start().await.expect("start the fixture site");
        Some(Self {
            site,
            chrome: Arc::new(chrome),
        })
    }

    /// A fresh fixture site on the same Chrome, so one run's recorded POSTs
    /// are its own without paying for another browser.
    pub(crate) async fn with_new_site(&self) -> Self {
        Self {
            site: FixtureSite::start().await.expect("start the fixture site"),
            chrome: self.chrome.clone(),
        }
    }

    /// This Chrome's DevTools HTTP address.
    pub(crate) fn endpoint(&self) -> &str {
        &self.chrome.endpoint
    }

    /// How many page tabs this Chrome has open, to check that none leaked.
    pub(crate) async fn page_targets(&self) -> usize {
        let mut connection = self.connect().await.expect("connect to Chrome");
        let targets = connection
            .call("Target.getTargets", serde_json::json!({}), None)
            .await
            .expect("list targets");
        targets["targetInfos"].as_array().map_or(0, |targets| {
            targets
                .iter()
                .filter(|target| target["type"] == "page")
                .count()
        })
    }

    /// Wait up to 2 s for this Chrome to have `count` page tabs, and return
    /// how many it has. Chrome drops a closed tab from its list shortly
    /// after `Target.closeTarget` answers.
    pub(crate) async fn settled_page_targets(&self, count: usize) -> usize {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let open = self.page_targets().await;
            if open == count || tokio::time::Instant::now() >= deadline {
                return open;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Open a fixture page (name plus optional query) in its own tab.
    pub(crate) async fn open(&self, page: &str) -> anyhow::Result<Page> {
        self.open_url(&self.site.url(page)).await
    }

    /// Open a fixture page with DuckDuckGo's autoconsent injected, as a
    /// run with cookie-banner refusal on (the default) opens it.
    pub(crate) async fn open_refusing(&self, page: &str) -> anyhow::Result<Page> {
        self.open_url_with(&self.site.url(page), true).await
    }

    /// Open any URL in its own tab.
    pub(crate) async fn open_url(&self, url: &str) -> anyhow::Result<Page> {
        self.open_url_with(url, false).await
    }

    /// Open any URL in its own tab, with autoconsent injected when asked.
    pub(crate) async fn open_url_with(&self, url: &str, autoconsent: bool) -> anyhow::Result<Page> {
        let connection = self.connect().await?;
        Page::open(connection, url, autoconsent).await
    }

    /// Connect to this Chrome. With dozens of test Chromes starting at once
    /// (`--test-threads=32`), the DevTools endpoint can miss its first
    /// request, so try it a few times.
    pub(crate) async fn connect(&self) -> anyhow::Result<Connection> {
        let mut attempt = 1;
        loop {
            match Connection::connect(&self.chrome.endpoint).await {
                Err(_) if attempt < 3 => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                result => return result,
            }
        }
    }

    /// Close a tab by its target id, for a page whose owner failed before it
    /// could close the page itself.
    pub(crate) async fn close_target(&self, target_id: &str) -> anyhow::Result<()> {
        let mut connection = self.connect().await?;
        connection
            .call(
                "Target.closeTarget",
                serde_json::json!({"targetId": target_id}),
                None,
            )
            .await
            .map(|_| ())
    }

    /// Run the real agent loop on a fixture page with a scripted plan.
    pub(crate) async fn run(
        &self,
        page: &str,
        decider: Arc<PlanDecider>,
        text: Option<Arc<dyn JevTextValueResolver>>,
        timeout: Duration,
    ) -> JevRunResult {
        let page = self.open(page).await.expect("open the fixture page");
        // Page-layer loops: no banner refusal, which the eval corpus covers.
        let mut config = JevEngineConfig::new("scripted fixture goal", decider)
            .with_wait(Duration::from_millis(100))
            .with_cookie_banner_refusal(false);
        if let Some(text) = text {
            config = config.with_text_resolver(text);
        }
        let mut engine = JevEngine::start(Box::new(page), config)
            .await
            .expect("first observation");
        let result = engine.run(timeout).await;
        engine.close().await.ok();
        result
    }
}

/// Skip a test body when no Chrome is installed and none is required; the
/// harness fails the test for an unusable `JEV_CHROME_BINARY` or a required
/// Chrome that is missing.
macro_rules! harness_or_skip {
    () => {
        match $crate::fixture_harness::Harness::start().await {
            Some(harness) => harness,
            None => {
                eprintln!("skipping: no Chrome binary found (set JEV_CHROME_BINARY)");
                return;
            }
        }
    };
}

/// Where one executed step spent its time.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct StepTiming {
    /// The fixture page and the action label, for the JSONL record.
    pub(crate) step: String,
    /// Freshness check, hit test and CDP input.
    pub(crate) act_ms: f64,
    /// `settle.js` waiting for the page to react.
    pub(crate) settle_ms: f64,
    /// The snapshot and fingerprint that follow.
    pub(crate) observe_ms: f64,
}

/// Execute one observed action, then settle and re-observe, timing each
/// phase. Returns the new observation.
pub(crate) async fn timed_step(
    page: &mut Page,
    observation: &Value,
    action: &Value,
    text: Option<&str>,
) -> anyhow::Result<(Value, StepTiming)> {
    let started = Instant::now();
    page.act(action, observation, text, Duration::from_millis(100))
        .await?;
    let acted = Instant::now();
    page.settle().await;
    let settled = Instant::now();
    let next = page.observe().await?;
    let timing = StepTiming {
        step: format!(
            "{} {}",
            observation["url"].as_str().unwrap_or_default(),
            action["label"].as_str().unwrap_or_default()
        ),
        act_ms: millis(acted - started),
        settle_ms: millis(settled - acted),
        observe_ms: millis(settled.elapsed()),
    };
    Ok((next, timing))
}

/// Act on the observed action with this kind and exact label, then settle
/// and read the page again.
pub(crate) async fn act_on(
    page: &mut Page,
    observation: &Value,
    kind: &str,
    label: &str,
    text: Option<&str>,
) -> anyhow::Result<(JevActOutcome, Value)> {
    let action = scripted::find(observation, kind, label)
        .with_context(|| format!("{kind} {label:?} is not observed: {observation:#}"))?
        .clone();
    let outcome = page
        .act(&action, observation, text, Duration::from_millis(100))
        .await?;
    Ok((outcome, page.observe().await?))
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// `$CARGO_TARGET_DIR`, or the workspace's `target/`.
pub(crate) fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"))
}

/// Write rows as JSONL to `target/jev-fixtures/<name>.jsonl`, replacing the
/// previous run's file, and return its path.
pub(crate) fn write_jsonl<T: Serialize>(name: &str, rows: &[T]) -> anyhow::Result<PathBuf> {
    let dir = target_dir().join("jev-fixtures");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(format!("{name}.jsonl"));
    let mut out = String::new();
    for row in rows {
        out.push_str(&serde_json::to_string(row)?);
        out.push('\n');
    }
    std::fs::write(&path, out).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

mod act_edge_tests;
mod autoconsent_tests;
mod booking_tests;
mod clickable_tests;
mod composed_tests;
mod consent_tests;
mod dialog_tests;
mod digest_tests;
mod evals;
mod field_tests;
mod fill_tests;
mod frame_text_tests;
mod hidden_tab_tests;
mod input_cost_tests;
mod key_tests;
mod launch_tests;
mod observe_edge_tests;
mod pointer_tests;
mod reach_tests;
mod region_tests;
mod secret_tests;
mod session_edge_tests;
mod session_life_tests;
mod session_tests;
mod settle_tests;
mod setup_tests;
mod snapshot_cost_tests;
mod snapshot_tests;
mod tab_tests;
mod tests;
mod twin_tests;
