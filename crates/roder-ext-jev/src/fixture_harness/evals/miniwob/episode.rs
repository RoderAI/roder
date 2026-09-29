//! One MiniWoB++ episode: set the task up, run the real engine on it, and
//! read the task's own reward.
//!
//! Setup follows `core.js` at the pinned commit, as BrowserGym does: seed
//! `Math.random`, give the episode a long clock, start it without the START
//! cover, and stop the countdown. The human display (reward HUD, click canvas
//! and cover) is hidden rather than removed: `core.endEpisode` writes the
//! episode count into the HUD after latching the reward, and throws inside
//! the task's handler when that element is gone. `core.startEpisode` becomes
//! a no-op, so the ended episode's page stays up instead of the cover, and
//! `core.endEpisode` ignores anything after the first reward (its timer is
//! cleared), so the first reward is the episode's.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::engine::{
    JevActOutcome, JevBrowser, JevDecision, JevDecisionClient, JevEngine, JevEngineConfig,
    JevRunResult, JevStatus, JevStop, JevTextValueResolver,
};
use crate::fallback::model::FallbackModel;
use crate::fallback::{EndCheck, FallbackOutcome, Limits, Rules};
use crate::fixture_harness::Harness;
use crate::page::Page;

/// Why the harness, not Jev, ended a run.
pub(super) const EPISODE_ENDED: &str = "miniwob: the task ended the episode";
pub(super) const STEP_CAP: &str = "miniwob: per-episode decision cap reached";

const SETUP_TIMEOUT: Duration = Duration::from_secs(15);

/// Episode start: the task's `window.onload` has built its widgets and made
/// the cover `core.startEpisode` needs.
const LOADED_JS: &str = "document.readyState==='complete' && typeof core==='object' && \
     typeof Math.seedrandom==='function' && core.cover_div!==null";

const READY_JS: &str = "typeof WOB_TASK_READY==='undefined' || WOB_TASK_READY===true";

const DONE_JS: &str = "WOB_DONE_GLOBAL===true";

/// Read in `close`, after the last action has settled.
const GRADE_JS: &str = "({done: WOB_DONE_GLOBAL, raw_reward: WOB_RAW_REWARD_GLOBAL, \
     reward: WOB_REWARD_GLOBAL, reason: WOB_REWARD_REASON == null ? null : String(WOB_REWARD_REASON)})";

fn setup_js(seed: u64) -> String {
    format!(
        "(() => {{
  Math.seedrandom('{seed}');
  core.EPISODE_MAX_TIME = 1000000;
  core.startEpisode = function () {{}};
  core.startEpisodeReal();
  core.clearTimer();
  for (const id of ['reward-display', 'click-canvas', 'sync-task-cover']) {{
    const el = document.getElementById(id);
    if (el) el.style.setProperty('display', 'none', 'important');
  }}
  // The natural-language email tasks return an object with the utterance outside test mode.
  const utterance = core.getUtterance();
  return typeof utterance === 'string' ? utterance : utterance && utterance.utterance;
}})()"
    )
}

/// What the task's globals say once the run is over.
#[derive(Debug, Clone, Default)]
pub(super) struct Grade {
    pub(super) done: bool,
    pub(super) raw_reward: f64,
    pub(super) reward: f64,
    pub(super) reason: Option<String>,
}

impl Grade {
    fn read(value: &Value) -> Self {
        Self {
            done: value["done"].as_bool().unwrap_or(false),
            raw_reward: value["raw_reward"].as_f64().unwrap_or(0.0),
            reward: value["reward"].as_f64().unwrap_or(0.0),
            reason: value["reason"].as_str().map(str::to_string),
        }
    }
}

/// Shared between the page wrapper and the decider.
#[derive(Debug, Default)]
struct EpisodeState {
    /// The task has ended the episode; nothing can change its reward now.
    done: bool,
    /// The first observation's actions, as `kind label`.
    start_actions: Option<Vec<String>>,
    grade: Option<Result<Grade, String>>,
}

/// The real page, reading the episode's `done` flag after every observation
/// and its reward just before it closes.
struct MiniwobPage {
    page: Page,
    state: Arc<Mutex<EpisodeState>>,
}

#[async_trait]
impl JevBrowser for MiniwobPage {
    async fn observe(&mut self) -> anyhow::Result<Value> {
        let observation = self.page.observe().await?;
        // For diagnosing a task: every observation's actions and text.
        if super::super::live::env("JEV_MINIWOB_DUMP").as_deref() == Some("1") {
            eprintln!(
                "--- observation\n{}\ntext: {}",
                observation["actions"]
                    .as_array()
                    .map(|actions| actions
                        .iter()
                        .map(|a| {
                            let mut a = a.clone();
                            if let Some(a) = a.as_object_mut() {
                                a.remove("rect");
                            }
                            a.to_string()
                        })
                        .collect::<Vec<_>>()
                        .join("\n"))
                    .unwrap_or_default(),
                observation["text"]
            );
        }
        let done = self.page.evaluate(DONE_JS).await.ok() == Some(json!(true));
        let mut state = self.state.lock().unwrap();
        state.done |= done;
        state
            .start_actions
            .get_or_insert_with(|| labels(&observation));
        Ok(observation)
    }

    async fn fresh(&mut self, observation: &Value, action: Option<&Value>) -> anyhow::Result<bool> {
        self.page.fresh(observation, action).await
    }

    async fn describe(&mut self) -> anyhow::Result<Option<crate::engine::JevPageFacts>> {
        self.page.describe().await
    }

    async fn act(
        &mut self,
        action: &Value,
        observation: &Value,
        text: Option<&str>,
        wait: Duration,
    ) -> anyhow::Result<JevActOutcome> {
        self.page.act(action, observation, text, wait).await
    }

    async fn refuse_cookie_banner(&mut self) -> anyhow::Result<Option<String>> {
        self.page.refuse_cookie_banner().await
    }

    async fn close(&mut self) -> anyhow::Result<()> {
        self.page.settle().await;
        let grade = self
            .page
            .evaluate(GRADE_JS)
            .await
            .map(|value| Grade::read(&value))
            .map_err(|error| format!("{error:#}"));
        self.state.lock().unwrap().grade = Some(grade);
        self.page.close().await
    }
}

/// The hosted decider, stopped by the harness once the task has ended the
/// episode (as BrowserGym ends it) or the per-episode decision cap is spent.
struct EpisodeDecider {
    inner: Arc<dyn JevDecisionClient>,
    state: Arc<Mutex<EpisodeState>>,
    calls: AtomicUsize,
    cap: usize,
}

#[async_trait]
impl JevDecisionClient for EpisodeDecider {
    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        if self.state.lock().unwrap().done {
            return Err(JevStop::new(JevStatus::Blocked, EPISODE_ENDED).into());
        }
        if self.calls.fetch_add(1, Ordering::Relaxed) >= self.cap {
            return Err(JevStop::new(JevStatus::BudgetExceeded, STEP_CAP).into());
        }
        self.inner.choose(observation, goal, history).await
    }
}

pub(super) struct EpisodeSpec<'a> {
    pub(super) url: String,
    pub(super) seed: u64,
    pub(super) max_steps: usize,
    pub(super) timeout: Duration,
    pub(super) decision: &'a Arc<dyn JevDecisionClient>,
    pub(super) text: &'a Arc<dyn JevTextValueResolver>,
    /// With `JEV_EVAL_FALLBACK=model`: the fallback after an episode Jev
    /// could not finish, on the same tab, until the task ends the episode.
    pub(super) fallback: Option<&'a Arc<dyn FallbackModel>>,
}

pub(super) struct Episode {
    pub(super) goal: String,
    /// The first observation's actions, as `kind label`.
    pub(super) start_actions: Vec<String>,
    pub(super) result: JevRunResult,
    pub(super) grade: Result<Grade, String>,
    /// The grade when Jev stopped, before a fallback went on.
    pub(super) jev_grade: Option<Grade>,
    pub(super) fallback: Option<FallbackOutcome>,
    pub(super) wall_ms: u64,
}

/// Stops the fallback once the task has ended the episode, as BrowserGym
/// ends it: nothing can change the reward after that.
struct EpisodeEnd {
    endpoint: String,
    target: String,
}

#[async_trait]
impl EndCheck for EpisodeEnd {
    async fn ended(&self) -> bool {
        let read = super::super::probe::read(&self.endpoint, &self.target, &[DONE_JS.into()]).await;
        matches!(read.dom.get(DONE_JS), Some(Ok(value)) if *value == json!(true))
    }
}

/// Open and set up the task, then run the engine on it.
pub(super) async fn run(harness: &Harness, spec: &EpisodeSpec<'_>) -> anyhow::Result<Episode> {
    let started = Instant::now();
    // As the tool opens a page: with cookie-banner refusal on, the default.
    let mut page = harness.open_url_with(&spec.url, true).await?;
    let goal = match tokio::time::timeout(SETUP_TIMEOUT, set_up(&mut page, spec.seed)).await {
        Ok(Ok(goal)) => goal,
        Ok(Err(error)) => {
            page.close().await.ok();
            return Err(error);
        }
        Err(_) => {
            page.close().await.ok();
            bail!("setup did not finish within {SETUP_TIMEOUT:?}");
        }
    };
    let target_id = page.target_id().to_string();
    let state = Arc::new(Mutex::new(EpisodeState::default()));
    let browser = MiniwobPage {
        page,
        state: state.clone(),
    };
    let decider = Arc::new(EpisodeDecider {
        inner: spec.decision.clone(),
        state: state.clone(),
        calls: AtomicUsize::new(0),
        cap: spec.max_steps,
    });
    // The production wait (800 ms for a `wait` action), not the fixture tiers'.
    let config = JevEngineConfig::new(goal.clone(), decider).with_text_resolver(spec.text.clone());
    let mut engine = match JevEngine::start(Box::new(browser), config).await {
        Ok(engine) => engine,
        Err(error) => {
            harness.close_target(&target_id).await.ok();
            return Err(error).context("first observation");
        }
    };
    let result = engine.run(spec.timeout).await;
    let mut jev_grade = None;
    let mut fallback = None;
    if let (Some(model), Some(why)) = (spec.fallback, crate::fallback::trigger::trigger(&result)) {
        let read =
            super::super::probe::read(harness.endpoint(), &target_id, &[GRADE_JS.into()]).await;
        let grade = match read.dom.get(GRADE_JS) {
            Some(Ok(value)) => Grade::read(value),
            _ => Grade::default(),
        };
        let ended = grade.done;
        jev_grade = Some(grade);
        if !ended {
            let tab = roder_ext_chrome::direct::DirectTab::Target {
                endpoint: harness.endpoint().to_string(),
                target_id: target_id.clone(),
            };
            let end = EpisodeEnd {
                endpoint: harness.endpoint().to_string(),
                target: target_id.clone(),
            };
            let rules = Rules {
                scope: crate::scope::JevOriginScope::any(),
                gate: false,
                authorized: false,
                banners: true,
                secrets: result.typed_secrets.clone(),
            };
            let limits = Limits {
                max_steps: super::super::fallback_steps(),
                max_tokens: crate::fallback::settings::DEFAULT_TOKENS,
                deadline: tokio::time::Instant::now() + spec.timeout,
            };
            let brief = crate::fallback::Brief {
                goal: &goal,
                trigger: why,
                jev: &result,
            };
            let (outcome, _) =
                crate::fallback::fall_back(&tab, model.as_ref(), brief, rules, limits, Some(&end))
                    .await;
            fallback = Some(outcome);
        }
    }
    engine.close().await.ok();
    let wall_ms = started.elapsed().as_millis() as u64;
    let mut state = state.lock().unwrap();
    let grade = state
        .grade
        .take()
        .unwrap_or_else(|| Err("the page closed before it was graded".into()));
    Ok(Episode {
        goal,
        start_actions: state.start_actions.take().unwrap_or_default(),
        result,
        grade,
        jev_grade,
        fallback,
        wall_ms,
    })
}

/// An observation's actions as `kind label`, controls included.
fn labels(observation: &Value) -> Vec<String> {
    observation["actions"]
        .as_array()
        .map(|actions| {
            actions
                .iter()
                .map(|action| {
                    format!(
                        "{} {}",
                        action["kind"].as_str().unwrap_or_default(),
                        action["label"].as_str().unwrap_or_default()
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn set_up(page: &mut Page, seed: u64) -> anyhow::Result<String> {
    poll(page, LOADED_JS, Duration::from_secs(10))
        .await
        .context("the task page did not load core.js and start its episode")?;
    let goal = page.evaluate(&setup_js(seed)).await.context("setup")?;
    let goal = goal
        .as_str()
        .filter(|goal| !goal.trim().is_empty())
        .context("the task has no utterance")?
        .to_string();
    // No task under miniwob/ sets it, but core.js offers the flag.
    poll(page, READY_JS, Duration::from_secs(3))
        .await
        .context("WOB_TASK_READY stayed false")?;
    Ok(goal)
}

async fn poll(page: &mut Page, expression: &str, limit: Duration) -> anyhow::Result<()> {
    let deadline = Instant::now() + limit;
    loop {
        if page.evaluate(expression).await.ok() == Some(json!(true)) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("`{expression}` was not true within {limit:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
