//! A second opinion from the text model (after fastbrowse's `consult`).
//!
//! Jev's heads are often unsure of steps that are right: across the live
//! corpus about one acting step in six has a call confidence under 0.55,
//! nearly all of them in runs that pass. So the consult does not veto, and
//! does not end a run: before a step that acts, when the call confidence is
//! under [`crate::engine::CONSULT_BELOW`] (or about to repeat the click
//! just made, which the repeat guard would otherwise stop), one call asks
//! the text model to name the control the step should act on. Jev's choice
//! stands when it names that one, names nothing, or the call fails; another
//! control it names is acted on instead. Either way the step passes every
//! gate Jev's own would: the irreversible gate holds a swapped control to
//! no answer (so as irreversible as it may be) and to no confidence, and a
//! repeat the consult endorses is let through only for a control that names
//! no commitment, adds nothing and posts no form (see [`repeatable`]: a
//! twin, or "Add to cart" again, still stops). Nor is a
//! cookie or consent choice ever offered to it.
//!
//! By default the threshold is zero, so only the unsure repeat is asked
//! about: that repeat stopped `mock_infinite_scroll` at its second "Load
//! more" (0.66), and the swaps an unsure-step consult made at 0.55 cost the
//! live corpus more than they won (see `CONSULT_BELOW`).
//!
//! A consult is not a step: it spends no action, model-call or stall budget
//! and records no history entry. Its call counts as a text call, with its
//! usage. At most [`MAX_CONSULTS`] a run, and once per page and choice.

use std::collections::HashSet;

use serde_json::{Value, json};

use super::Agent;
use super::repeat::{CONTEXT_CHARS, LABEL_CHARS};
use crate::engine::{JevConsultAnswer, JevDecision, JevEngineConfig, JevSuppressedKind};
use crate::irreversible::names_commitment;
use crate::session::cut;
use crate::usage::JevBilled;

/// Consults a run may make.
pub(super) const MAX_CONSULTS: usize = 8;
/// Page text sent with a consult.
const TEXT_CHARS: usize = 3000;
/// Controls listed in a consult, on-screen ones first.
const MAX_CONTROLS: usize = 80;
/// Recent steps listed in a consult.
const RECENT_STEPS: usize = 6;

/// What the consult remembers across the run.
#[derive(Default)]
pub(super) struct Consults {
    pub(super) made: usize,
    /// (page fingerprint, Jev's choice) already asked about.
    asked: HashSet<(String, String)>,
    /// Light recoveries spent (see `recovery`).
    pub(super) recoveries: usize,
}

/// Why the loop asks, for the request and the recorded event.
pub(super) enum Ask<'a> {
    /// Jev is unsure of a step that acts, or it repeats the click just made.
    Unsure {
        action: &'a Value,
        decision: &'a JevDecision,
        repeat: bool,
    },
    /// The run stopped making progress; `reason` says how.
    Stalled { reason: &'a str },
}

/// The consult's verdict on the step about to be taken.
pub(super) struct Verdict {
    pub(super) action: Value,
    pub(super) decision: JevDecision,
    /// The consult named the control acted on.
    pub(super) endorsed: bool,
    /// The control acted on is not the one Jev chose.
    pub(super) swapped: bool,
}

/// A step of one of these kinds acts on the page.
pub(super) fn acts(action: &Value) -> bool {
    matches!(
        action["kind"].as_str(),
        Some("click" | "fill" | "select" | "enter")
    )
}

/// A control a consult may name: anything but a wait, a document read, or
/// a cookie or consent choice. Banners are the banner refusal's alone (and
/// with it off, nobody's): live, a light recovery pressed "Accept all" and
/// a consult "No thanks" on pages the task said to leave the banner on.
fn nameable(action: &Value, consent_page: bool) -> bool {
    !matches!(
        action["kind"].as_str(),
        Some("wait" | "read_document") | None
    ) && !(consent_page && consent_choice(action["label"].as_str().unwrap_or_default()))
}

/// Whether a page's text speaks of cookies or consent.
fn consent_page(observation: &Value) -> bool {
    let text = observation["text"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    ["cookie", "consent", "privacy choices"]
        .iter()
        .any(|word| text.contains(word))
}

/// Whether a label reads as a cookie-banner choice.
pub(super) fn consent_choice(label: &str) -> bool {
    let label = label.to_lowercase();
    let words = label
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    const WORDS: &[&str] = &[
        "accept",
        "reject",
        "agree",
        "allow",
        "consent",
        "cookie",
        "cookies",
        "necessary",
        "decline",
        "deny",
        "essential",
        "preferences",
    ];
    words.iter().any(|word| WORDS.contains(word))
        || ["no thanks", "got it", "manage options"]
            .iter()
            .any(|phrase| label.contains(phrase))
}

/// Whether an unsure repeat of `action` may be asked about, and go ahead
/// when the consult endorses it: a control whose label names no commitment
/// and adds nothing (a second "Add to cart" is the costly repeat the guard
/// exists for, and "add" is no commitment word), and that posts no form.
pub(super) fn repeatable(action: &Value) -> bool {
    const ADDS: &[&str] = &[
        "add",
        "cart",
        "basket",
        "bag",
        "trolley",
        "increase",
        "increment",
        "vote",
        "upvote",
        "like",
        "follow",
        "claim",
        "redeem",
    ];
    let label = action["label"].as_str().unwrap_or_default();
    let adds = label
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| ADDS.contains(&word));
    !names_commitment(label) && !adds && action["submits"]["method"] != json!("post")
}

/// Jev's operation name for an observed action's kind.
pub(super) fn operation_of(action: &Value) -> String {
    match action["kind"].as_str() {
        Some("fill") => "TYPE_TEXT",
        Some("enter") => "PRESS_ENTER",
        Some("select") => "SELECT",
        Some("scroll") => "SCROLL",
        Some("key") => "PRESS_KEY",
        _ => "CLICK",
    }
    .to_string()
}

/// Apply `JEV_CONSULT`, `JEV_CONSULT_BELOW` and `JEV_LIGHT_RECOVERIES` to a
/// config that would get `consultant`: `JEV_CONSULT=0` (or `false`, `no`,
/// `off`) leaves it without one.
pub(crate) fn configure(
    mut config: JevEngineConfig,
    consultant: Option<std::sync::Arc<dyn crate::engine::JevConsultant>>,
) -> anyhow::Result<JevEngineConfig> {
    let env = |key: &str| std::env::var(key).ok().filter(|v| !v.trim().is_empty());
    let on = match env("JEV_CONSULT").map(|raw| raw.trim().to_ascii_lowercase()) {
        None => true,
        Some(raw) => match raw.as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => anyhow::bail!("JEV_CONSULT must be 1 or 0 (or true or false), not {raw:?}"),
        },
    };
    let Some(consultant) = consultant.filter(|_| on) else {
        return Ok(config);
    };
    config = config.with_consultant(consultant);
    if let Some(below) = env("JEV_CONSULT_BELOW") {
        let below = below.trim().parse::<f64>().map_err(|_| {
            anyhow::anyhow!("JEV_CONSULT_BELOW must be a number from 0 to 1, not {below:?}")
        })?;
        config = config.with_consult_below(below);
    }
    if let Some(count) = env("JEV_LIGHT_RECOVERIES") {
        let count = count.trim().parse::<usize>().map_err(|_| {
            anyhow::anyhow!("JEV_LIGHT_RECOVERIES must be a whole number, not {count:?}")
        })?;
        config = config.with_light_recoveries(count);
    }
    Ok(config)
}

impl Agent {
    /// Before acting on `action`: ask the consultant when Jev is unsure of
    /// it, or it is an unsure repeat the guard would stop; the action to
    /// take, possibly another observed one.
    pub(super) async fn consult_before_acting(
        &mut self,
        action: Value,
        decision: JevDecision,
    ) -> Verdict {
        let unchanged = |action, decision| Verdict {
            action,
            decision,
            endorsed: false,
            swapped: false,
        };
        // A step the action budget will stop anyway is not worth a call.
        let budget_left =
            self.history.iter().filter(super::chosen).count() < self.config.max_actions;
        if self.config.consult.consultant.is_none()
            || self.consults.made >= MAX_CONSULTS
            || !budget_left
            || !acts(&action)
        {
            return unchanged(action, decision);
        }
        let unsure = decision.call_confidence() < self.config.consult.below;
        let repeat = self
            .unsure_repeat(&action, &decision)
            .is_some_and(|held| held.kind == JevSuppressedKind::SameControl)
            && repeatable(&action);
        if !unsure && !repeat {
            return unchanged(action, decision);
        }
        let key = (
            self.observation["fingerprint"].to_string(),
            decision.choice.clone(),
        );
        if !self.consults.asked.insert(key) {
            return unchanged(action, decision);
        }
        let ask = Ask::Unsure {
            action: &action,
            decision: &decision,
            repeat,
        };
        let Some(answer) = self.ask(ask).await else {
            return unchanged(action, decision);
        };
        let named = answer.control.as_deref().and_then(|id| self.observed(id));
        let Some(named) = named.filter(|named| self.nameable(named)) else {
            self.record_consult(&answer, "kept", &action);
            return unchanged(action, decision);
        };
        if named["id"] == action["id"] {
            self.record_consult(&answer, "endorsed", &action);
            return Verdict {
                action,
                decision,
                endorsed: true,
                swapped: false,
            };
        }
        self.record_consult(&answer, "swapped", &named);
        let decision = directed(&named, decision.usage.clone());
        Verdict {
            action: named,
            decision,
            endorsed: true,
            swapped: true,
        }
    }

    /// Whether a consult may name `action` on the page as it is.
    pub(super) fn nameable(&self, action: &Value) -> bool {
        nameable(action, consent_page(&self.observation))
    }

    /// The observed action with this id.
    pub(super) fn observed(&self, id: &str) -> Option<Value> {
        self.observation["actions"]
            .as_array()?
            .iter()
            .find(|action| action["id"].as_str() == Some(id))
            .cloned()
    }

    /// One consultant call; its usage is a text call. `None` is no advice:
    /// no consultant, or the call failed.
    pub(super) async fn ask(&mut self, ask: Ask<'_>) -> Option<JevConsultAnswer> {
        let consultant = self.config.consult.consultant.clone()?;
        let purpose = match ask {
            Ask::Unsure { .. } => "consult",
            Ask::Stalled { .. } => "recover",
        };
        let request = self.consult_request(&ask);
        self.consults.made += 1;
        match consultant.consult(&request).await {
            Ok(answer) => {
                self.text_calls.push(json!({
                    "purpose": purpose,
                    "model": answer.model,
                    "latency_ms": answer.latency_ms,
                    "usage": answer.usage,
                }));
                Some(answer)
            }
            Err(error) => {
                if let Some(usage) = JevBilled::usage_of(&error) {
                    self.text_calls
                        .push(json!({"purpose": purpose, "usage": usage}));
                }
                let reason = self.secrets.scrub(&error.to_string());
                self.recovery
                    .record(&format!("{purpose}_failed"), &reason, self.elapsed_ms);
                None
            }
        }
    }

    /// Record what a consult said about the step, page text scrubbed.
    fn record_consult(&mut self, answer: &JevConsultAnswer, outcome: &str, action: &Value) {
        let label = self.tidy(&action["label"], LABEL_CHARS).unwrap_or_default();
        let why = self
            .tidy(&json!(answer.diagnosis), CONTEXT_CHARS)
            .unwrap_or_default();
        self.recovery.record(
            &format!("consult_{outcome}"),
            &format!("{label:?}: {why}"),
            self.elapsed_ms,
        );
    }

    /// The request a consultant reads: the goal, the page, its controls,
    /// the recent steps and the problem. Page text was scrubbed when it was
    /// read, and typed secrets are scrubbed from the goal too.
    fn consult_request(&self, ask: &Ask<'_>) -> Value {
        let text = self.observation["text"].as_str().unwrap_or_default();
        let mut controls = self.observation["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|action| self.nameable(action))
            .collect::<Vec<_>>();
        // On-screen controls first; the sort is stable.
        controls.sort_by_key(|action| action["offscreen"] == json!(true));
        let controls = controls
            .into_iter()
            .take(MAX_CONTROLS)
            .map(control)
            .collect::<Vec<_>>();
        let mut steps = self
            .history
            .iter()
            .rev()
            .filter(super::chosen)
            .take(RECENT_STEPS)
            .map(step)
            .collect::<Vec<_>>();
        steps.reverse();
        let mut request = json!({
            "goal": self.secrets.scrub(&self.decision_goal()),
            "page": {
                "url": self.observation["url"],
                "title": self.observation["title"],
                "text": cut(text, TEXT_CHARS),
            },
            "controls": controls,
            "recent_steps": steps,
            "today": chrono::Local::now().date_naive().to_string(),
        });
        match ask {
            Ask::Unsure {
                action,
                decision,
                repeat,
            } => {
                request["purpose"] = json!("consult");
                request["jev_proposal"] = json!({
                    "control": action["id"],
                    "label": action["label"],
                    "confidence": (decision.call_confidence() * 100.0).round() / 100.0,
                });
                request["problem"] = json!(match repeat {
                    true =>
                        "Jev is unsure, and its proposal repeats the click the last step \
                             made, which changed the page. Repeat it only when the task needs \
                             it again (more to load, another page); never to redo a purchase, \
                             send, delete or other commitment already made.",
                    false => "Jev is unsure of its proposal for the next step.",
                });
            }
            Ask::Stalled { reason } => {
                request["purpose"] = json!("recover");
                request["problem"] = json!(format!(
                    "Jev has stopped making progress: {reason}. Name one control that moves \
                     the task on another way than the recent steps tried."
                ));
            }
        }
        request
    }
}

/// A control as a consult lists it.
fn control(action: &Value) -> Value {
    let mut listed = json!({"id": action["id"], "kind": action["kind"]});
    let text = |value: &Value, chars| value.as_str().map(|text| cut(text, chars));
    if let Some(label) = text(&action["label"], LABEL_CHARS) {
        listed["label"] = json!(label);
    }
    let place = action.get("context").or_else(|| action.get("section"));
    if let Some(place) = place.and_then(|place| text(place, CONTEXT_CHARS)) {
        listed["context"] = json!(place);
    }
    if let Some(value) = text(&action["value"], LABEL_CHARS).filter(|v| !v.is_empty()) {
        listed["value"] = json!(value);
    }
    for key in ["checked", "selected", "expanded", "offscreen"] {
        if action[key].is_boolean() {
            listed[key] = action[key].clone();
        }
    }
    listed
}

/// A history step as a consult lists it; typed secrets are `[secret]`.
fn step(entry: &Value) -> Value {
    json!({
        "action": entry["action"],
        "kind": entry["kind"],
        "text": entry["text"],
        "page_changed": entry["page_changed"],
        "covered": entry["covered"],
        "effect": entry["effect"],
    })
}

/// A decision for an observed action Jev did not choose: no answer to the
/// gate's questions and no confidence, so every gate holds it to the most
/// it may be.
pub(super) fn directed(action: &Value, usage: Value) -> JevDecision {
    let id = action["id"].as_str().unwrap_or_default().to_string();
    JevDecision {
        choice: id,
        operation: operation_of(action),
        target: None,
        confidence: 0.0,
        target_confidence: None,
        probabilities: Default::default(),
        latency_ms: 0,
        usage,
        model: None,
        irreversible: None,
        downloads_forbidden: None,
    }
}

#[cfg(test)]
mod tests;
