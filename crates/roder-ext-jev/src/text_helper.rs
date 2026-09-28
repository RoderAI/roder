//! The helper that writes one field value.
//!
//! A port of upstream Jev's `model.field_context` and `model.field_text`. The
//! request shape, including which reasoning field each provider gets, and the
//! strict `{"text": ...}` reply contract are reproduced exactly; Roder supplies
//! the endpoint and key from its own providers (see `text_model`). The field
//! context is Jev's: it adds today's date, and the ISO format a native date
//! or time input takes.

use std::time::Instant;

use anyhow::{Context, bail};
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use serde_json::{Map, Value, json};

use crate::engine::{JevStatus, JevStop, JevTextValue, JevTextValueResolver};
use crate::http::{JsonPoster, PostFailure, RetryPolicy};
use crate::prompts::TEXT_VALUE;
use crate::python_json;
use crate::text_model::TextModel;
use crate::usage::JevBilled;

const MAX_VALUE_CHARS: usize = 2000;

const NO_VALUE: &str = "Text helper returned no valid field value; nothing typed.";

/// The helper's `{"text": null}`, or a secret the goal does not hold: the
/// goal does not give this field's value. Names the field (page text).
fn missing_value(context: &Value) -> anyhow::Error {
    let label = context["field"]["label"].as_str().unwrap_or_default();
    JevStop::new(
        JevStatus::NeedsInput,
        format!("The goal gives no value for the field {label:?}; nothing typed."),
    )
    .into()
}

/// A password or one-time code the helper wrote must appear in the goal
/// word for word: a code is rarely known in advance, and a secret must never
/// be guessed. Any other field's value may be inferred.
fn grounded(context: &Value, value: &str) -> bool {
    let secret = matches!(
        context["field"]["input_type"].as_str(),
        Some("password" | "one-time-code")
    );
    !secret
        || context["goal"]
            .as_str()
            .is_some_and(|goal| goal.contains(value.trim()))
}

/// Today with its weekday and the next two months, each named by its
/// distance, in fastbrowse's exact words (MIT, page.py `today`): told the
/// date alone, its field writer booked "the next Monday" a week late, and
/// told the coming months as a bare list, it took the last for "next month".
pub(crate) fn today(date: NaiveDate) -> String {
    let next_month = |date: NaiveDate| {
        let (year, month) = match date.month() {
            12 => (date.year() + 1, 1),
            month => (date.year(), month + 1),
        };
        NaiveDate::from_ymd_opt(year, month, 1).expect("the first of a month")
    };
    let first = next_month(date);
    let second = next_month(first);
    format!(
        "{}. In one month: {}; in two months: {}",
        date.format("%Y-%m-%d (%A)"),
        first.format("%B %Y"),
        second.format("%B %Y")
    )
}

/// The ISO shape a native date or time input takes, by its type.
fn iso_format(input_type: &str) -> Option<&'static str> {
    Some(match input_type {
        "date" => "YYYY-MM-DD (ISO 8601, such as 2026-09-25)",
        "datetime-local" => "YYYY-MM-DDThh:mm (ISO 8601, such as 2026-09-25T14:30)",
        "month" => "YYYY-MM (ISO 8601, such as 2026-09)",
        "week" => "YYYY-Www (ISO 8601, such as 2026-W39)",
        "time" => "hh:mm (24-hour, such as 14:30)",
        _ => return None,
    })
}

/// Everything the helper is told about the field it is filling. `today` is
/// passed in so the context is deterministic under test.
pub(crate) fn field_context(
    goal: &str,
    action: &Value,
    page: &Value,
    history: &[Value],
    today_is: NaiveDate,
) -> Value {
    let mut field = Map::new();
    for key in ["label", "role", "value"] {
        field.insert(key.into(), action.get(key).cloned().unwrap_or(Value::Null));
    }
    if let Some(input_type) = action["input_type"].as_str() {
        field.insert("input_type".into(), json!(input_type));
        if let Some(format) = iso_format(input_type) {
            field.insert("format".into(), json!(format));
        }
    }
    // The card, row, section or column the field sits in, as the decision
    // model sees it: what tells a second "Name" field from the first.
    if let Some(context) = action.get("context") {
        field.insert("context".into(), context.clone());
    }
    let text = page["text"].as_str().unwrap_or_default();
    let text = text.chars().take(6000).collect::<String>();
    let recent = history
        .iter()
        .skip(history.len().saturating_sub(6))
        .map(|entry| {
            let mut recorded = Map::new();
            for key in ["action", "text"] {
                recorded.insert(key.into(), entry.get(key).cloned().unwrap_or(Value::Null));
            }
            Value::Object(recorded)
        })
        .collect::<Vec<_>>();
    let mut context = Map::new();
    context.insert("goal".into(), json!(goal));
    context.insert("date".into(), json!(today(today_is)));
    context.insert("field".into(), Value::Object(field));
    let others = other_fields(action, page);
    if !others.is_empty() {
        context.insert("other_fields".into(), Value::Array(others));
    }
    context.insert("page".into(), json!({"title": page["title"], "text": text}));
    context.insert("recent_actions".into(), Value::Array(recent));
    Value::Object(context)
}

/// How many other fields the helper is told about, and how much of each value.
const OTHER_FIELDS: usize = 20;
const OTHER_VALUE_CHARS: usize = 1000;

/// The page's other text fields and dropdowns, each once, with what they hold
/// now: the text to copy from a textarea, or which field of a form this one
/// is. Password and one-time-code fields, whose content is never read, are
/// not listed.
fn other_fields(action: &Value, page: &Value) -> Vec<Value> {
    let mut seen = vec![action["node"].clone()];
    let mut fields = Vec::new();
    for other in page["actions"].as_array().map_or(&[][..], Vec::as_slice) {
        let kind = other["kind"].as_str().unwrap_or_default();
        if !matches!(kind, "fill" | "select")
            || seen.contains(&other["node"])
            || crate::secret::is_secret(other)
        {
            continue;
        }
        seen.push(other["node"].clone());
        let label = other["label"].as_str().unwrap_or_default();
        let value = other
            .get("current_value")
            .or_else(|| other.get("value"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut entry = Map::new();
        entry.insert(
            "label".into(),
            json!(label.split(" → ").next().unwrap_or(label)),
        );
        if let Some(input_type) = other.get("input_type") {
            entry.insert("input_type".into(), input_type.clone());
        }
        if let Some(context) = other.get("context") {
            entry.insert("context".into(), context.clone());
        }
        entry.insert(
            "value".into(),
            json!(value.chars().take(OTHER_VALUE_CHARS).collect::<String>()),
        );
        fields.push(Value::Object(entry));
        if fields.len() == OTHER_FIELDS {
            break;
        }
    }
    fields
}

/// Upstream selects the reasoning field by base URL: DeepSeek's `thinking`,
/// else `reasoning.effort`; `reasoning.enabled=false` replaces both where the
/// endpoint takes it (OpenRouter, see `text_model`).
fn reasoning(base_url: &str, reasoning_none: bool) -> (String, Value) {
    if reasoning_none {
        return ("reasoning".into(), json!({"enabled": false}));
    }
    if base_url.contains("api.deepseek.com/") {
        ("thinking".into(), json!({"type": "disabled"}))
    } else {
        ("reasoning".into(), json!({"effort": "low"}))
    }
}

/// The exact body upstream posts to the text helper.
pub(crate) fn request_body(context: &Value, model: &str, base_url: &str, none: bool) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("max_tokens".into(), json!(1024));
    body.insert("response_format".into(), json!({"type": "json_object"}));
    let (key, value) = reasoning(base_url, none);
    body.insert(key, value);
    body.insert(
        "messages".into(),
        json!([
            {"role": "system", "content": TEXT_VALUE},
            {"role": "user", "content": python_json::dumps(context)},
        ]),
    );
    Value::Object(body)
}

/// Upstream trims the trailing slash before appending the path.
fn endpoint(base_url: &str) -> String {
    format!("{}/chat/completions", base_url.trim_end_matches('/'))
}

/// Accept only `{"text": "..."}` with a usable value, as upstream does. A
/// null or blank value is the helper saying the goal lacks it (`Ok(None)`),
/// which ends the run as `needs_input`; any other bad reply is an `error`.
pub(crate) fn parse_value(content: &str) -> anyhow::Result<Option<String>> {
    let parsed: Value = serde_json::from_str(content).map_err(|_| anyhow::anyhow!(NO_VALUE))?;
    let object = parsed.as_object().context(NO_VALUE)?;
    if object.len() != 1 {
        bail!(NO_VALUE);
    }
    match object.get("text") {
        Some(Value::Null) => {}
        Some(Value::String(value)) if value.trim().is_empty() => {}
        Some(Value::String(value)) if value.chars().count() <= MAX_VALUE_CHARS => {
            return Ok(Some(value.clone()));
        }
        _ => bail!(NO_VALUE),
    }
    Ok(None)
}

/// Roder's configured text model, over one pooled client for the whole run.
pub(crate) struct TextHelper {
    model: TextModel,
    http: JsonPoster,
}

impl TextHelper {
    pub(crate) fn new(model: TextModel) -> Self {
        Self::with_policy(model, RetryPolicy::text_helper())
    }

    fn with_policy(model: TextModel, policy: RetryPolicy) -> Self {
        Self {
            model,
            http: JsonPoster::new(policy),
        }
    }

    /// Ask the text model for the value of one field.
    async fn field_text(&self, context: &Value) -> anyhow::Result<JevTextValue> {
        let text = &self.model;
        let body = request_body(context, &text.model, &text.base_url, text.reasoning_none);
        let started = Instant::now();
        let result = self
            .http
            .post(&endpoint(&text.base_url), &text.api_key, &body)
            .await
            .map_err(|failure| {
                let message = match &failure {
                    PostFailure::Connection => {
                        "Text helper connection failed; nothing typed.".to_string()
                    }
                    PostFailure::Status(status, _) => {
                        format!("Text helper returned HTTP {status}; nothing typed.")
                    }
                    PostFailure::InvalidBody => NO_VALUE.to_string(),
                };
                failure.stop(message)
            })?;
        let usage = result.get("usage").cloned().unwrap_or_else(|| json!({}));
        // The provider has answered, so the call is billed even when it gave
        // no usable value (a null, a blank, or a malformed reply): that
        // usage is kept.
        let value = result["choices"][0]["message"]["content"]
            .as_str()
            .context(NO_VALUE)
            .and_then(parse_value)
            .and_then(|value| match value {
                Some(value) if grounded(context, &value) => Ok(value),
                _ => Err(missing_value(context)),
            })
            .map_err(|error| anyhow::Error::from(JevBilled::new(usage.clone(), error)))?;
        Ok(JevTextValue {
            value,
            model: text.model.clone(),
            latency_ms: started.elapsed().as_millis() as u64,
            usage,
        })
    }
}

#[async_trait]
impl JevTextValueResolver for TextHelper {
    async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue> {
        self.field_text(field_context).await
    }
}

#[cfg(test)]
mod tests;
