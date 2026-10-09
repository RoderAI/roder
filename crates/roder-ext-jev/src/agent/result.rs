//! The run's result, in the shape the tool reports.

use serde_json::{Value, json};

use super::Agent;
use crate::engine::{
    JevActionRecord, JevControl, JevFrameText, JevOmitted, JevPageFacts, JevRunResult, JevStatus,
};
use crate::space::{ActionSpace, action_space};
use crate::usage::{JevCallUsage, JevUsage};

impl Agent {
    /// The result payload, in the shape the tool reports.
    pub(crate) fn result(&self, stopped_because: Option<String>) -> JevRunResult {
        let text = self.observation["text"].as_str().unwrap_or_default();
        let visible_text = text.chars().take(6000).collect::<String>();
        let actions = self
            .history
            .iter()
            .map(|entry| JevActionRecord {
                step: entry["step"].as_u64().unwrap_or_default() as usize,
                action: entry["action"].as_str().unwrap_or_default().to_string(),
                kind: entry["kind"].as_str().unwrap_or_default().to_string(),
                context: entry["context"].as_str().map(str::to_string),
                text: entry["text"].as_str().map(str::to_string),
                url: entry["url"].as_str().unwrap_or_default().to_string(),
                page_changed: entry["page_changed"].as_bool(),
                covered: entry["covered"].as_bool().unwrap_or_default(),
                covered_by: entry["covered_by"].as_str().map(str::to_string),
                refused: entry["refused"].as_str().map(str::to_string),
                uncovered: entry["uncovered"].as_str().map(str::to_string),
                dialogs: serde_json::from_value(entry["dialogs"].clone()).unwrap_or_default(),
                opened_tab: entry["opened_tab"] == json!(true),
                effect: entry["effect"].as_str().map(str::to_string),
                elapsed_ms: entry["elapsed_ms"].as_u64().unwrap_or_default(),
                choice: entry["choice"].as_str().unwrap_or_default().to_string(),
                probability: entry["probability"].clone(),
                confidence: entry["confidence"].as_f64().unwrap_or_default(),
                target_confidence: entry["target_confidence"].as_f64(),
                decision_latency_ms: entry["latency_ms"].as_u64().unwrap_or_default(),
                text_helper: entry["text_helper"].as_str().map(str::to_string),
                text_latency_ms: entry["text_latency_ms"].as_u64().unwrap_or_default(),
                operation: entry["operation"].as_str().unwrap_or_default().to_string(),
                target: entry["target"].as_str().map(str::to_string),
                usage: entry["usage"].clone(),
                executed_ms: entry["executed_ms"].as_u64().unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        let observed = action_space(
            self.observation["actions"]
                .as_array()
                .map_or(&[][..], Vec::as_slice),
        );
        JevRunResult {
            status: self.status,
            url: self.observation["url"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            title: self.observation["title"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            visible_text,
            actions,
            elapsed_ms: self.elapsed_ms,
            observed_elements: observed.elements.len(),
            omitted: omitted(&self.observation, &observed),
            model_calls: self.decisions,
            text_calls: self.text_calls.len(),
            usage: JevUsage {
                decision: JevCallUsage::sum(
                    self.decision_calls
                        .iter()
                        .map(|call| &call.usage)
                        .chain(&self.unusable_decisions),
                ),
                text: JevCallUsage::sum(self.text_calls.iter().map(|call| &call["usage"])),
            },
            decisions: self.decision_calls.clone(),
            // A reason can quote the page, which may show a typed secret.
            stopped_because: stopped_because.map(|reason| self.secrets.scrub(&reason)),
            stop_cause: (self.status != JevStatus::Done)
                .then_some(self.cause)
                .flatten(),
            suppressed_click: self.suppressed_click.clone(),
            untrusted: true,
            controls: controls(&self.observation),
            page: self.page_facts(),
            typed_secrets: self.secrets.clone(),
        }
    }

    /// The browser's facts about the final page, with the text of the frames
    /// its last observation read.
    fn page_facts(&self) -> Option<JevPageFacts> {
        let frames =
            serde_json::from_value::<Vec<JevFrameText>>(self.observation["frames"].clone())
                .unwrap_or_default();
        let mut facts = self.facts.clone().unwrap_or_default();
        facts.frames = frames;
        (facts != JevPageFacts::default()).then_some(facts)
    }
}

/// What `observation` held that Jev was not offered: the controls its
/// snapshot left out (`omitted_actions`, counted per control) and the
/// targets `space` had no room for, which a choice cannot take.
pub(crate) fn omitted(observation: &Value, space: &ActionSpace) -> JevOmitted {
    JevOmitted {
        controls: observation["omitted_actions"].as_u64().unwrap_or_default() as usize,
        options: space.skipped,
    }
}

/// A control's value is cut to this many characters.
const VALUE_CHARS: usize = 60;
/// A select lists at most this many options.
const OPTIONS: usize = 12;

/// The controls an observation offers, one per element in observed order:
/// a field once (not also its "Open" click), a select once with its options.
/// A secret field's value is never included; the observation holds none.
pub(crate) fn controls(observation: &Value) -> Vec<JevControl> {
    let mut controls: Vec<(Value, JevControl)> = Vec::new();
    for action in observation["actions"].as_array().into_iter().flatten() {
        let kind = action["kind"].as_str().unwrap_or_default();
        if !matches!(kind, "click" | "fill" | "select") {
            continue;
        }
        let node = action["node"].clone();
        let label = action["label"].as_str().unwrap_or_default();
        if let Some((_, known)) = controls.iter_mut().find(|(seen, _)| *seen == node) {
            if kind == "select"
                && known.options.len() < OPTIONS
                && let Some((_, option)) = label.split_once(" → ")
            {
                known.options.push(option.to_string());
            }
            continue;
        }
        let secret = crate::secret::is_secret(action);
        // A checkbox, radio or switch reports its state, not its HTML value
        // ("on" unless the page sets one).
        let checked = match (kind, action["checked"].as_str()) {
            ("click", Some("true")) => Some(true),
            ("click", Some("false")) => Some(false),
            _ => None,
        };
        let value = match kind {
            "select" => action["current_value"].as_str(),
            _ if secret || checked.is_some() => None,
            _ => action["value"].as_str(),
        }
        .filter(|value| !value.trim().is_empty())
        .map(|value| cut(value, VALUE_CHARS));
        let (label, options) = match (kind, label.split_once(" → ")) {
            ("select", Some((field, option))) => (field, vec![option.to_string()]),
            _ => (label, Vec::new()),
        };
        controls.push((
            node,
            JevControl {
                label: label.to_string(),
                kind: kind.to_string(),
                role: action["role"].as_str().map(str::to_string),
                context: action["context"].as_str().map(str::to_string),
                section: action["section"].as_str().map(str::to_string),
                value,
                checked,
                options,
                offscreen: action["offscreen"] == json!(true),
            },
        ));
    }
    controls.into_iter().map(|(_, control)| control).collect()
}

fn cut(text: &str, chars: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(chars) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}
