//! Several fields of one form from one text call (after fastbrowse's
//! `fill_form`, MIT).
//!
//! When the decision model first chooses to type into an empty field of a
//! form that has other empty text fields, the resolver is asked once for
//! all of them ([`JevTextValueResolver::resolve_form`]). The chosen field's
//! value is typed now; the others are kept, and a later step that chooses
//! one of those fields types its kept value without another text call, as
//! long as the field is still the same node in the same document, still
//! reads the same and is still empty. Every field is still chosen by its
//! own decision, so the gate, the loop caps and the budgets see each step
//! as before; only the text call is shared.
//!
//! A form with a password or one-time-code field is never batched, so no
//! secret field reaches the text model this way. A form is asked once per
//! document: a field the reply left out, a field that changed, and a form
//! whose batch failed all fall back to one call per field.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::engine::{JevStatus, JevStop, JevTextValue, JevTextValueResolver};
use crate::secret;
use crate::text_helper::form::form_context;
use crate::usage::JevBilled;

/// At most this many fields are listed in one form call.
const MAX_FIELDS: usize = 12;

/// A form's empty text fields this many or more are worth one call.
const MIN_FIELDS: usize = 2;

/// The values written for fields not yet typed, and the forms already asked.
#[derive(Default)]
pub(super) struct FormFill {
    /// By the field's `ref` (its document and node).
    kept: HashMap<String, Kept>,
    /// Forms (document and form id) already asked in this run.
    asked: HashSet<(String, String)>,
}

struct Kept {
    /// What the field read as when its value was written.
    identity: Value,
    value: String,
    model: String,
}

/// The document a `ref` (`j<time origin>-<node>`) belongs to.
fn document(action: &Value) -> Option<&str> {
    action["ref"]
        .as_str()
        .and_then(|reference| reference.rsplit_once('-'))
        .map(|(document, _)| document)
}

/// The form a field belongs to, in its document.
fn form_key(action: &Value) -> Option<(String, String)> {
    let form = &action["form"]["id"];
    if form.is_null() {
        return None;
    }
    Some((document(action)?.to_string(), form.to_string()))
}

/// What a kept value is tied to: everything the helper was told about the
/// field but its (empty) value.
fn identity(action: &Value) -> Value {
    json!([
        action["label"],
        action["role"],
        action["input_type"],
        action["context"],
        action["form"]["id"],
    ])
}

fn empty(action: &Value) -> bool {
    action["value"].as_str().is_none_or(str::is_empty)
}

/// A field a form call may write: an empty text field (a combobox, whose
/// value is picked from suggestions, is left to its own steps).
fn writable(action: &Value) -> bool {
    action["kind"] == "fill"
        && action["role"] != "combobox"
        && empty(action)
        && action["ref"].is_string()
}

/// The fields to ask for with `chosen`, `chosen` first, or `None` when its
/// form is not worth a form call: it is not in a form, a field of the form
/// is a secret, or fewer than two of its text fields are empty.
pub(super) fn form_fields<'a>(chosen: &'a Value, observation: &'a Value) -> Option<Vec<&'a Value>> {
    let key = form_key(chosen)?;
    if !writable(chosen) || secret::is_secret(chosen) {
        return None;
    }
    let actions = observation["actions"].as_array()?;
    let same_form = actions
        .iter()
        .filter(|action| form_key(action).as_ref() == Some(&key))
        .collect::<Vec<_>>();
    if same_form.iter().any(|action| secret::is_secret(action)) {
        return None;
    }
    let mut fields = vec![chosen];
    for action in same_form {
        if fields.len() == MAX_FIELDS {
            break;
        }
        if writable(action) && !fields.iter().any(|field| field["ref"] == action["ref"]) {
            fields.push(action);
        }
    }
    (fields.len() >= MIN_FIELDS).then_some(fields)
}

/// Whether an executed action leaves kept values as written: typing,
/// picking an option, scrolling, waiting, clicking into a text field (the
/// decision model often focuses a field before typing in it), or ticking a
/// box, choosing a radio or a tab. Anything else (a button or link click,
/// Enter, a key) may submit the form or show what it rejected, which a value
/// written before it never saw, so the fields left are asked for alone with
/// the page as it is then.
fn keeps_values(action: &Value, observation: &Value) -> bool {
    match action["kind"].as_str() {
        Some("fill" | "select" | "scroll" | "wait") => true,
        Some("click") => {
            matches!(
                action["role"].as_str(),
                Some(
                    "checkbox"
                        | "radio"
                        | "switch"
                        | "tab"
                        | "menuitemcheckbox"
                        | "menuitemradio"
                        | "textbox"
                        | "searchbox"
                )
            ) || observation["actions"].as_array().is_some_and(|actions| {
                action["ref"].is_string()
                    && actions
                        .iter()
                        .any(|other| other["kind"] == "fill" && other["ref"] == action["ref"])
            })
        }
        _ => false,
    }
}

impl FormFill {
    /// Drop the values kept so far once `action`, chosen from
    /// `observation`, has run, unless it leaves them as written (see
    /// [`keeps_values`]).
    pub(super) fn acted(&mut self, action: &Value, observation: &Value) {
        if !keeps_values(action, observation) {
            self.kept.clear();
        }
    }

    /// The value kept for `action`, once: only while the field is the same
    /// node, reads as it did and is still empty.
    pub(super) fn take(&mut self, action: &Value) -> Option<JevTextValue> {
        let reference = action["ref"].as_str()?;
        let kept = self.kept.remove(reference)?;
        (empty(action) && kept.identity == identity(action)).then(|| JevTextValue {
            value: kept.value,
            model: kept.model,
            latency_ms: 0,
            usage: json!({}),
        })
    }

    /// Ask `text` once for the empty fields of `chosen`'s form. The chosen
    /// field's value, when the reply gives one; the others are kept. `None`
    /// sends the chosen field to the single-field path.
    pub(super) async fn write(
        &mut self,
        text: &dyn JevTextValueResolver,
        chosen: &Value,
        context: FormRequest<'_>,
        text_calls: &mut Vec<Value>,
    ) -> anyhow::Result<Option<JevTextValue>> {
        let Some(fields) = form_fields(chosen, context.observation) else {
            return Ok(None);
        };
        let key = form_key(chosen).expect("a form");
        if !self.asked.insert(key) {
            return Ok(None);
        }
        let form = form_context(
            context.goal,
            &fields,
            context.observation,
            context.history,
            chrono::Local::now().date_naive(),
        );
        let labels = fields
            .iter()
            .map(|field| field["label"].clone())
            .collect::<Vec<_>>();
        let written = match text.resolve_form(&form).await {
            Ok(Some(written)) => written,
            Ok(None) => return Ok(None),
            Err(error) => {
                if let Some(usage) = JevBilled::usage_of(&error) {
                    text_calls.push(json!({"usage": usage, "fields": labels, "values": null}));
                }
                // A provider that stayed down would fail the single call too.
                if JevStop::status_of(&error) == JevStatus::Unavailable {
                    return Err(error);
                }
                return Ok(None);
            }
        };
        text_calls.push(json!({
            "model": written.model,
            "latency_ms": written.latency_ms,
            "usage": written.usage,
            "field": chosen["label"],
            "fields": labels,
            "values": fields.iter().map(|field| {
                field["id"].as_str().and_then(|id| written.values.get(id)).cloned()
            }).collect::<Vec<_>>(),
        }));
        let mut now = None;
        for field in &fields {
            let Some(value) = field["id"].as_str().and_then(|id| written.values.get(id)) else {
                continue;
            };
            if field["ref"] == chosen["ref"] {
                now = Some(JevTextValue {
                    value: value.clone(),
                    model: written.model.clone(),
                    latency_ms: written.latency_ms,
                    usage: written.usage.clone(),
                });
            } else if let Some(reference) = field["ref"].as_str() {
                self.kept.insert(
                    reference.to_string(),
                    Kept {
                        identity: identity(field),
                        value: value.clone(),
                        model: written.model.clone(),
                    },
                );
            }
        }
        Ok(now)
    }
}

/// What a form call is built from besides the fields.
pub(super) struct FormRequest<'a> {
    pub goal: &'a str,
    pub observation: &'a Value,
    pub history: &'a [Value],
}

#[cfg(test)]
mod tests;
