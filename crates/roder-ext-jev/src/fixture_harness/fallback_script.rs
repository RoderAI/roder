//! A keyless stand-in for the fallback model: it plays a fixed plan of tool
//! calls, then its final message, one per reply.
//!
//! Refs (`e1`, `e2`, …) depend on the page, so a step names its target by
//! label and the stand-in resolves it against the page the last look read:
//! `ref_of` (and `from_ref_of`, `to_ref_of`) becomes that element's ref,
//! and `at` (`from_at`, `to_at`) a point within its box (`{"label": …,
//! "fx": …, "fy": …}`, the centre by default). A label `tag:span` names the
//! first listed element with that tag. Each reply reports 1,000 input and
//! 50 output tokens, so the result's accounting can be checked.

use std::sync::Mutex;

use anyhow::{Context, bail};
use async_trait::async_trait;
use roder_api::inference::{TokenUsage, ToolCallCompleted};
use roder_api::transcript::TranscriptItem;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::fallback::model::{FallbackModel, Reply, Turn};

/// One step of a scripted fallback: a tool call, or the final message.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FallbackStep {
    /// A direct tool's short name (`look`, `click`, …).
    tool: Option<String>,
    #[serde(default)]
    args: Map<String, Value>,
    /// The final message, such as `DONE: …`.
    say: Option<String>,
}

impl FallbackStep {
    pub(crate) fn check(&self) -> anyhow::Result<()> {
        match (&self.tool, &self.say) {
            (Some(_), None) | (None, Some(_)) => Ok(()),
            _ => bail!("a fallback step is a tool call or a final message: {self:?}"),
        }
    }
}

pub(crate) struct ScriptedFallback {
    plan: Vec<FallbackStep>,
    played: Mutex<usize>,
    /// Whether it is shown the pictures tools return.
    images: bool,
    /// The click tool's parameters as the first request offered them.
    pub(crate) click_parameters: Mutex<Vec<String>>,
    /// The names of the tools the first request offered.
    offered: Mutex<Vec<String>>,
    /// The standing instructions the first request carried.
    instructions: Mutex<String>,
    /// The first message the fallback was given: the goal, what Jev did and
    /// the page.
    opening: Mutex<String>,
    /// The tool results the newest request carried, as the model read them.
    results: Mutex<Vec<String>>,
}

impl ScriptedFallback {
    pub(crate) fn new(plan: Vec<FallbackStep>) -> Self {
        Self {
            plan,
            played: Mutex::new(0),
            images: true,
            click_parameters: Mutex::new(Vec::new()),
            offered: Mutex::new(Vec::new()),
            instructions: Mutex::new(String::new()),
            opening: Mutex::new(String::new()),
            results: Mutex::new(Vec::new()),
        }
    }

    /// A model whose engine does not show it the pictures a tool returns.
    pub(crate) fn without_images(mut self) -> Self {
        self.images = false;
        self
    }

    pub(crate) fn from_json(plan: Value) -> Self {
        Self::new(serde_json::from_value(plan).expect("a fallback plan"))
    }

    /// How many replies it gave.
    pub(crate) fn played(&self) -> usize {
        *self.played.lock().unwrap()
    }

    /// The first message it was given, as the fallback model read it.
    pub(crate) fn opening(&self) -> String {
        self.opening.lock().unwrap().clone()
    }

    /// The tool results the last request carried, as the model read them.
    pub(crate) fn tool_results(&self) -> Vec<String> {
        self.results.lock().unwrap().clone()
    }

    /// The names of the tools the first request offered.
    pub(crate) fn offered_tools(&self) -> Vec<String> {
        self.offered.lock().unwrap().clone()
    }

    /// The standing instructions the first request carried.
    pub(crate) fn instructions(&self) -> String {
        self.instructions.lock().unwrap().clone()
    }
}

#[async_trait]
impl FallbackModel for ScriptedFallback {
    fn label(&self) -> String {
        "scripted/fallback (none)".into()
    }

    fn sees_tool_result_images(&self) -> bool {
        self.images
    }

    async fn reply(&self, turn: Turn<'_>) -> anyhow::Result<Reply> {
        let index = {
            let mut played = self.played.lock().unwrap();
            *played += 1;
            *played - 1
        };
        if index == 0
            && let Some(TranscriptItem::UserMessage(first)) = turn.transcript.first()
        {
            *self.opening.lock().unwrap() = first.text.clone();
        }
        *self.results.lock().unwrap() = turn
            .transcript
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::ToolResult(result) => Some(result.result.clone()),
                _ => None,
            })
            .collect();
        if index == 0 {
            *self.offered.lock().unwrap() =
                turn.tools.iter().map(|tool| tool.name.clone()).collect();
            *self.instructions.lock().unwrap() = turn.instructions.to_string();
        }
        if index == 0
            && let Some(click) = turn.tools.iter().find(|tool| tool.name == "jev_tab_click")
        {
            *self.click_parameters.lock().unwrap() = click.parameters["properties"]
                .as_object()
                .map(|properties| properties.keys().cloned().collect())
                .unwrap_or_default();
        }
        let usage = Some(TokenUsage::new(1000, 50, 1050));
        let Some(step) = self.plan.get(index) else {
            return Ok(Reply {
                text: "DONE: the plan ran out".into(),
                usage,
                ..Reply::default()
            });
        };
        if let Some(say) = &step.say {
            return Ok(Reply {
                text: say.clone(),
                usage,
                ..Reply::default()
            });
        }
        let tool = step.tool.as_deref().unwrap_or_default();
        let page = turn.last_page.cloned().unwrap_or(Value::Null);
        let args = resolve(&step.args, &page)
            .with_context(|| format!("scripted fallback step {} ({tool})", index + 1))?;
        Ok(Reply {
            text: String::new(),
            calls: vec![ToolCallCompleted {
                id: format!("call-{}", index + 1),
                name: format!("jev_tab_{tool}"),
                arguments: Value::Object(args).to_string(),
            }],
            usage,
            items: Vec::new(),
        })
    }
}

/// A step's arguments with labels resolved against the page.
fn resolve(args: &Map<String, Value>, page: &Value) -> anyhow::Result<Map<String, Value>> {
    let mut out = Map::new();
    for (key, value) in args {
        if let Some(prefix) = ["ref_of", "from_ref_of", "to_ref_of"]
            .contains(&key.as_str())
            .then(|| key.trim_end_matches("ref_of"))
        {
            let label = value.as_str().context("ref_of takes a label")?;
            let element = element(page, label)?;
            out.insert(format!("{prefix}ref"), element["ref"].clone());
        } else if let Some(prefix) = ["at", "from_at", "to_at"]
            .contains(&key.as_str())
            .then(|| key.trim_end_matches("at"))
        {
            let label = value["label"].as_str().context("at takes a label")?;
            let element = element(page, label)?;
            let at = |axis: &str, size: &str, fraction: &str| {
                element[axis].as_f64().unwrap_or(0.0)
                    + element[size].as_f64().unwrap_or(0.0)
                        * value[fraction].as_f64().unwrap_or(0.5)
            };
            out.insert(format!("{prefix}x"), json!(at("x", "w", "fx")));
            out.insert(format!("{prefix}y"), json!(at("y", "h", "fy")));
        } else {
            out.insert(key.clone(), value.clone());
        }
    }
    Ok(out)
}

fn element<'a>(page: &'a Value, label: &str) -> anyhow::Result<&'a Value> {
    let elements = page["elements"].as_array().context("no page read yet")?;
    let found = match label.strip_prefix("tag:") {
        Some(tag) => elements.iter().find(|element| element["tag"] == tag),
        None => elements
            .iter()
            .find(|element| element["label"] == label)
            .or_else(|| {
                elements.iter().find(|element| {
                    element["label"]
                        .as_str()
                        .is_some_and(|text| text.contains(label))
                })
            }),
    };
    found.with_context(|| format!("no element {label:?} on {}", page["url"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_resolve_to_refs_and_points() {
        let page = json!({"url": "http://x/", "elements": [
            {"ref": "e1", "tag": "button", "label": "Products", "x": 10, "y": 20, "w": 100, "h": 40},
            {"ref": "e2", "tag": "span", "label": "", "x": 300, "y": 0, "w": 20, "h": 20},
        ]});
        let args = |raw: Value| resolve(raw.as_object().unwrap(), &page).unwrap();
        assert_eq!(args(json!({"ref_of": "Products"}))["ref"], "e1");
        assert_eq!(args(json!({"to_ref_of": "Prod"}))["to_ref"], "e1");
        let point = args(json!({"at": {"label": "Products", "fx": 0.25, "fy": 0.5}}));
        assert_eq!(
            (point["x"].clone(), point["y"].clone()),
            (json!(35.0), json!(40.0))
        );
        assert_eq!(
            args(json!({"from_at": {"label": "tag:span"}}))["from_x"],
            json!(310.0)
        );
        assert_eq!(args(json!({"key": "Escape", "repeat": 2}))["repeat"], 2);
        assert!(resolve(json!({"ref_of": "Nope"}).as_object().unwrap(), &page).is_err());
    }
}
