//! Scripted fakes for driving the JEV loop deterministically.
//!
//! `ScriptedBrowser` replays queued observations and `fresh()` answers and
//! records every act; `ScriptedDecider` replays queued choices. Both hand out
//! a shared log so a test can inspect what the loop did after `run` returns.
//! The idea follows fastbrowse's `ScriptedJev`/`ScriptedLLM` test doubles
//! (MIT, tests/test_policy.py); the code is our own.

// Each integration test binary compiles this module and uses part of it.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use roder_ext_jev::{
    Covered, JevActOutcome, JevBrowser, JevDecision, JevDecisionClient, JevPageFacts, JevStatus,
    JevStop,
};
use serde_json::{Map, Value, json};

/// A page with one action per `(id, kind)` pair.
pub fn page(fingerprint: &str, actions: &[(&str, &str)]) -> Value {
    let actions = actions
        .iter()
        .enumerate()
        .map(|(node, (id, kind))| {
            json!({
                "id": id,
                "kind": kind,
                "label": format!("Label {id}"),
                "role": if *kind == "fill" { "textbox" } else { "button" },
                "value": "",
                "node": node + 1,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "url": "https://scripted.test/",
        "title": "Scripted",
        "text": format!("page {fingerprint}"),
        "fingerprint": fingerprint,
        "marker": ["scripted", fingerprint],
        "actions": actions,
    })
}

/// A page with one action per `(id, kind, label)`.
pub fn labelled_page(fingerprint: &str, actions: &[(&str, &str, &str)]) -> Value {
    let mut observation = page(
        fingerprint,
        &actions
            .iter()
            .map(|(id, kind, _)| (*id, *kind))
            .collect::<Vec<_>>(),
    );
    for (action, (_, _, label)) in observation["actions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(actions)
    {
        action["label"] = json!(label);
    }
    observation
}

/// `page` with dialogs answered since the previous observation, as a real
/// browser reports them: as `dialogs`, and before the page text.
pub fn page_with_dialogs(fingerprint: &str, actions: &[(&str, &str)], dialogs: Value) -> Value {
    let mut observation = page(fingerprint, actions);
    let lines = dialogs
        .as_array()
        .unwrap()
        .iter()
        .map(|dialog| format!("[{} dialog] {}", dialog["type"], dialog["message"]))
        .collect::<Vec<_>>();
    observation["text"] = json!(format!("{}\n{}", lines.join("\n"), observation["text"]));
    observation["dialogs"] = dialogs;
    observation
}

/// One `act` call as the browser saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct ActCall {
    pub id: String,
    pub text: Option<String>,
}

#[derive(Default)]
pub struct BrowserLog {
    pub observes: usize,
    /// The action id each `fresh()` call checked, `None` for a page-only check.
    pub fresh_calls: Vec<Option<String>>,
    /// Every act that started, including one that never finished or found
    /// its target covered.
    pub acts: Vec<ActCall>,
    /// How many times the loop asked the browser to refuse a cookie banner.
    pub banner_checks: usize,
    /// How many times the loop asked the browser to describe the page.
    pub describes: usize,
}

pub struct ScriptedBrowser {
    observations: VecDeque<Value>,
    last: Value,
    fresh: VecDeque<bool>,
    covered: VecDeque<bool>,
    refusals: VecDeque<Option<String>>,
    banners: VecDeque<Option<String>>,
    act_delay: Option<Duration>,
    /// What `describe` answers, in order; the last repeats.
    facts: VecDeque<JevPageFacts>,
    log: Arc<Mutex<BrowserLog>>,
}

impl ScriptedBrowser {
    /// Observations are served in order; the last one repeats once the
    /// queue is empty.
    pub fn new(observations: Vec<Value>) -> Self {
        let last = observations.last().cloned().expect("at least one page");
        Self {
            observations: observations.into(),
            last,
            fresh: VecDeque::new(),
            covered: VecDeque::new(),
            refusals: VecDeque::new(),
            banners: VecDeque::new(),
            act_delay: None,
            facts: VecDeque::new(),
            log: Arc::default(),
        }
    }

    /// Answers for successive `fresh()` calls; `true` once they run out.
    pub fn with_fresh(mut self, answers: &[bool]) -> Self {
        self.fresh = answers.iter().copied().collect();
        self
    }

    /// Whether successive acts find their target covered (and so return
    /// [`Covered`] without acting); `false` once they run out.
    pub fn with_covered(mut self, answers: &[bool]) -> Self {
        self.covered = answers.iter().copied().collect();
        self
    }

    /// Why successive acts that ran were refused by the page (`None` for
    /// one it kept); kept once they run out.
    pub fn with_refusals(mut self, refusals: &[Option<&str>]) -> Self {
        self.refusals = refusals
            .iter()
            .map(|refusal| refusal.map(str::to_string))
            .collect();
        self
    }

    /// The label of the banner button each successive cookie-banner check
    /// clicks (`None` for one that found none); none once they run out.
    pub fn with_banners(mut self, banners: &[Option<&str>]) -> Self {
        self.banners = banners
            .iter()
            .map(|banner| banner.map(str::to_string))
            .collect();
        self
    }

    /// Make every act sleep before returning.
    pub fn with_act_delay(mut self, delay: Duration) -> Self {
        self.act_delay = Some(delay);
        self
    }

    /// What successive `describe` calls answer, the last repeating;
    /// nothing when none are given.
    pub fn with_facts(mut self, facts: &[JevPageFacts]) -> Self {
        self.facts = facts.iter().cloned().collect();
        self
    }

    pub fn log(&self) -> Arc<Mutex<BrowserLog>> {
        Arc::clone(&self.log)
    }
}

#[async_trait]
impl JevBrowser for ScriptedBrowser {
    async fn observe(&mut self) -> anyhow::Result<Value> {
        self.log.lock().unwrap().observes += 1;
        if let Some(next) = self.observations.pop_front() {
            self.last = next;
        }
        Ok(self.last.clone())
    }

    async fn fresh(
        &mut self,
        _observation: &Value,
        action: Option<&Value>,
    ) -> anyhow::Result<bool> {
        let id = action.and_then(|action| action["id"].as_str().map(str::to_string));
        self.log.lock().unwrap().fresh_calls.push(id);
        Ok(self.fresh.pop_front().unwrap_or(true))
    }

    async fn act(
        &mut self,
        action: &Value,
        _observation: &Value,
        text: Option<&str>,
        _wait: Duration,
    ) -> anyhow::Result<JevActOutcome> {
        self.log.lock().unwrap().acts.push(ActCall {
            id: action["id"].as_str().unwrap_or_default().to_string(),
            text: text.map(str::to_string),
        });
        if self.covered.pop_front().unwrap_or(false) {
            return Err(Covered::new("covered in the script").into());
        }
        if let Some(delay) = self.act_delay {
            tokio::time::sleep(delay).await;
        }
        Ok(match self.refusals.pop_front().flatten() {
            Some(reason) => JevActOutcome::refused(reason),
            None => JevActOutcome::done(),
        })
    }

    async fn refuse_cookie_banner(&mut self) -> anyhow::Result<Option<String>> {
        self.log.lock().unwrap().banner_checks += 1;
        Ok(self.banners.pop_front().flatten())
    }

    async fn describe(&mut self) -> anyhow::Result<Option<JevPageFacts>> {
        self.log.lock().unwrap().describes += 1;
        Ok(match self.facts.len() {
            0 => None,
            1 => self.facts.front().cloned(),
            _ => self.facts.pop_front(),
        })
    }
}

#[derive(Default)]
pub struct DeciderLog {
    /// The fingerprint of the observation each decision was made on.
    pub seen: Vec<Value>,
    /// Whether each decision was asked with the irreversible-action gate on.
    pub gated: Vec<bool>,
}

pub struct ScriptedDecider {
    choices: Mutex<VecDeque<String>>,
    last: Mutex<String>,
    confidences: Mutex<VecDeque<f64>>,
    /// The gate's answer about each gated decision's choice, in order;
    /// `None` (no valid answer) once they run out.
    irreversible: Mutex<VecDeque<Option<f64>>>,
    /// Fail every decision from this one (counted from zero) on, as a
    /// [`JevStop`] with this status or, for `None`, a plain error.
    failure: Option<(usize, Option<JevStatus>)>,
    /// What each decision reports as its usage.
    usage: Value,
    log: Arc<Mutex<DeciderLog>>,
}

impl ScriptedDecider {
    /// Choices are returned in order; the last one repeats once the queue is
    /// empty. `DONE` and `BLOCKED` get their own operation name.
    pub fn new(choices: &[&str]) -> Self {
        let last = choices.last().expect("at least one choice").to_string();
        Self {
            choices: Mutex::new(choices.iter().map(|choice| choice.to_string()).collect()),
            last: Mutex::new(last),
            confidences: Mutex::default(),
            irreversible: Mutex::default(),
            failure: None,
            usage: json!({}),
            log: Arc::default(),
        }
    }

    /// Answer `after` decisions from the script, then fail every one after
    /// that: with a [`JevStop`] of `status`, or a plain error for `None`.
    pub fn failing_after(mut self, after: usize, status: Option<JevStatus>) -> Self {
        self.failure = Some((after, status));
        self
    }

    /// The operation confidence of each decision in order; 1.0 after that.
    pub fn with_confidences(self, confidences: &[f64]) -> Self {
        *self.confidences.lock().unwrap() = confidences.iter().copied().collect();
        self
    }

    /// What the gate's question about each gated decision's choice answers.
    pub fn with_irreversible(self, answers: &[Option<f64>]) -> Self {
        *self.irreversible.lock().unwrap() = answers.iter().copied().collect();
        self
    }

    /// Report `usage` on every decision.
    pub fn with_usage(mut self, usage: Value) -> Self {
        self.usage = usage;
        self
    }

    pub fn log(&self) -> Arc<Mutex<DeciderLog>> {
        Arc::clone(&self.log)
    }
}

#[async_trait]
impl JevDecisionClient for ScriptedDecider {
    async fn choose(
        &self,
        observation: &Value,
        _goal: &str,
        _history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.decide(observation, false)
    }

    async fn choose_gated(
        &self,
        observation: &Value,
        _goal: &str,
        _history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        let mut decision = self.decide(observation, true)?;
        decision.irreversible = self.irreversible.lock().unwrap().pop_front().flatten();
        Ok(decision)
    }
}

impl ScriptedDecider {
    fn decide(&self, observation: &Value, gated: bool) -> anyhow::Result<JevDecision> {
        let decided = {
            let mut log = self.log.lock().unwrap();
            log.seen.push(observation["fingerprint"].clone());
            log.gated.push(gated);
            log.seen.len() - 1
        };
        if let Some((after, status)) = self.failure
            && decided >= after
        {
            return Err(match status {
                Some(status) => JevStop::new(status, "scripted stop").into(),
                None => anyhow::anyhow!("scripted failure"),
            });
        }
        let choice = match self.choices.lock().unwrap().pop_front() {
            Some(choice) => {
                *self.last.lock().unwrap() = choice.clone();
                choice
            }
            None => self.last.lock().unwrap().clone(),
        };
        let operation = match choice.as_str() {
            "DONE" | "BLOCKED" => choice.clone(),
            enter if enter.starts_with("enter") => "PRESS_ENTER".to_string(),
            _ => "CLICK".to_string(),
        };
        let mut probabilities = Map::new();
        probabilities.insert(choice.clone(), json!(1.0));
        Ok(JevDecision {
            choice,
            operation,
            target: None,
            confidence: self.confidences.lock().unwrap().pop_front().unwrap_or(1.0),
            target_confidence: None,
            probabilities,
            latency_ms: 1,
            usage: self.usage.clone(),
            model: None,
            irreversible: None,
        })
    }
}
