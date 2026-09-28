//! The run's result, in the shape the tool reports.

use serde_json::json;

use super::Agent;
use crate::engine::{JevActionRecord, JevRunResult};
use crate::space::action_space;
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
                text: entry["text"].as_str().map(str::to_string),
                url: entry["url"].as_str().unwrap_or_default().to_string(),
                page_changed: entry["page_changed"].as_bool(),
                covered: entry["covered"].as_bool().unwrap_or_default(),
                refused: entry["refused"].as_str().map(str::to_string),
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
            untrusted: true,
        }
    }
}
