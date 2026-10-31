//! What the text helper is told about the field it fills: the goal, today's
//! date with the days after it, the field itself, the page's other fields,
//! the page and the recent actions.

use chrono::{Datelike, NaiveDate};
use serde_json::{Map, Value, json};

/// Today with its weekday, the next two months, each named by its distance,
/// and the seven days after it, each with its weekday (after fastbrowse,
/// MIT, page.py `today` and `next_days`): told the date alone, its field
/// writer booked "the next Monday" a week late; told the coming months as a
/// bare list, it took the last for "next month"; and told the weekday but
/// left to count, it still booked a relative day a week late. A relative
/// day is looked up in the list instead.
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
    let next_days = date
        .iter_days()
        .skip(1)
        .take(7)
        .map(|day| day.format("%A %Y-%m-%d").to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{}. In one month: {}; in two months: {}. Next seven days: {next_days}",
        date.format("%Y-%m-%d (%A)"),
        first.format("%B %Y"),
        second.format("%B %Y")
    )
}

/// The ISO shape a native date or time input takes, by its type.
pub(super) fn iso_format(input_type: &str) -> Option<&'static str> {
    Some(match input_type {
        "date" => "YYYY-MM-DD (ISO 8601, such as 2026-09-25)",
        "datetime-local" => "YYYY-MM-DDThh:mm (ISO 8601, such as 2026-09-25T14:30)",
        "month" => "YYYY-MM (ISO 8601, such as 2026-09)",
        "week" => "YYYY-Www (ISO 8601, such as 2026-W39)",
        "time" => "hh:mm (24-hour, such as 14:30)",
        _ => return None,
    })
}

/// What the helper is told about one field it fills: its label, role,
/// value, type with the ISO format a native date or time input takes, and
/// the card, row, section or column it sits in, as the decision model sees
/// it: what tells a second "Name" field from the first.
pub(crate) fn field_entry(action: &Value) -> Map<String, Value> {
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
    if let Some(context) = action.get("context") {
        field.insert("context".into(), context.clone());
    }
    field
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
    let field = field_entry(action);
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
    let others = other_fields(&[action], page);
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

/// The page's text fields and dropdowns other than `filled`, each once, with
/// what they hold now: the text to copy from a textarea, or which field of a
/// form this one is. Password and one-time-code fields, whose content is
/// never read, are not listed.
pub(crate) fn other_fields(filled: &[&Value], page: &Value) -> Vec<Value> {
    let mut seen = filled
        .iter()
        .map(|action| action["node"].clone())
        .collect::<Vec<_>>();
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
