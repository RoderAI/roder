//! Several fields of one form from one text-helper call (after fastbrowse's
//! form writer, `fill_form`): the context lists the form's empty text
//! fields, the one being filled first, and the reply gives each a value or
//! null. Each value passes the checks a single field's does.

use std::collections::BTreeMap;
use std::time::Instant;

use anyhow::bail;
use chrono::NaiveDate;
use serde_json::{Map, Value, json};

use super::{
    Ask, MAX_VALUE_CHARS, TextHelper, field_context, field_entry, grounded, other_fields, unfenced,
};
use crate::engine::JevFormValues;
use crate::text_model::TextModel;
use crate::usage::JevBilled;

const NO_VALUES: &str = "Text helper returned no valid form values; nothing typed.";

/// What the helper is told when it fills `fields` (the field being filled
/// first): the single field's context, with `fields`, each with its action
/// id, in place of `field`, and the page's other fields besides them.
pub(crate) fn form_context(
    goal: &str,
    fields: &[&Value],
    page: &Value,
    history: &[Value],
    today_is: NaiveDate,
) -> Value {
    let chosen = fields.first().copied().unwrap_or(&Value::Null);
    let single = field_context(goal, chosen, page, history, today_is);
    let listed = fields
        .iter()
        .map(|action| {
            let mut entry = Map::new();
            entry.insert("id".into(), action["id"].clone());
            entry.extend(field_entry(action));
            Value::Object(entry)
        })
        .collect::<Vec<_>>();
    let mut context = Map::new();
    for key in ["goal", "date"] {
        context.insert(key.into(), single[key].clone());
    }
    context.insert("fields".into(), Value::Array(listed));
    let others = other_fields(fields, page);
    if !others.is_empty() {
        context.insert("other_fields".into(), Value::Array(others));
    }
    for key in ["page", "recent_actions"] {
        context.insert(key.into(), single[key].clone());
    }
    Value::Object(context)
}

/// Accept only `{"fields": [{"id": ..., "text": ...}, ...]}` naming each
/// listed id at most once, once any code fence is taken off; the values
/// given, by id. A null or blank text is no value; a field the reply leaves
/// out has none either. An unknown or repeated id, or a value longer than a
/// single field's may be, makes the whole reply unusable.
pub(crate) fn parse_values(
    content: &str,
    ids: &[&str],
) -> anyhow::Result<BTreeMap<String, String>> {
    let parsed: Value =
        serde_json::from_str(unfenced(content)).map_err(|_| anyhow::anyhow!(NO_VALUES))?;
    let Some(object) = parsed.as_object().filter(|object| object.len() == 1) else {
        bail!(NO_VALUES);
    };
    let Some(entries) = object.get("fields").and_then(Value::as_array) else {
        bail!(NO_VALUES);
    };
    let mut seen = Vec::new();
    let mut values = BTreeMap::new();
    for entry in entries {
        let Some(id) = entry["id"].as_str().filter(|id| ids.contains(id)) else {
            bail!(NO_VALUES);
        };
        if seen.contains(&id) {
            bail!(NO_VALUES);
        }
        seen.push(id);
        match &entry["text"] {
            Value::Null => {}
            Value::String(value) if value.trim().is_empty() => {}
            Value::String(value) if value.chars().count() <= MAX_VALUE_CHARS => {
                values.insert(id.to_string(), value.clone());
            }
            _ => bail!(NO_VALUES),
        }
    }
    Ok(values)
}

/// Whether a form call's value for an email, phone or web-address field is
/// written in the goal (ignoring case). Asked for the whole contact form of
/// `missing_value`, whose goal gives no email, the default writer wrote
/// "email@example.com" in 1 of 3 live runs, where alone it answered null
/// every time; such a field is asked for alone instead.
fn stated(field: &Value, value: &str, goal: &str) -> bool {
    !matches!(field["input_type"].as_str(), Some("email" | "tel" | "url"))
        || goal.to_lowercase().contains(&value.trim().to_lowercase())
}

impl TextHelper {
    /// Ask `model` for the values of the fields `context` lists. A password
    /// or one-time code is never listed; were one listed, it would keep a
    /// value only when the goal holds it word for word, as a single field.
    /// An email, phone or web address the goal does not hold is dropped,
    /// so its field is asked for alone (see [`stated`]).
    pub(super) async fn ask_form(
        &self,
        model: &TextModel,
        context: &Value,
    ) -> anyhow::Result<JevFormValues> {
        let started = Instant::now();
        let (content, usage) = self.post(model, Ask::Form, context).await?;
        let fields = context["fields"].as_array().map_or(&[][..], Vec::as_slice);
        let ids = fields
            .iter()
            .filter_map(|field| field["id"].as_str())
            .collect::<Vec<_>>();
        let mut values = content
            .and_then(|content| parse_values(&content, &ids))
            .map_err(|error| anyhow::Error::from(JevBilled::new(usage.clone(), error)))?;
        let goal = context["goal"].as_str().unwrap_or_default();
        values.retain(|id, value| {
            let field = fields.iter().find(|field| field["id"] == json!(id));
            let field = field.cloned().unwrap_or(Value::Null);
            grounded(&json!({"goal": goal, "field": field}), value) && stated(&field, value, goal)
        });
        Ok(JevFormValues {
            values,
            model: model.model.clone(),
            latency_ms: started.elapsed().as_millis() as u64,
            usage,
        })
    }
}

#[cfg(test)]
mod tests;
