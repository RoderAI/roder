//! Native semantic evidence and image routing; raw answers remain unchanged.
use super::{EVIDENCE, request_body};
use crate::usage::JevCallUsage;
use crate::{JevBilled, JevDecisionTransport};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

const SECRET_FIELDS: &str = "Secret contents are withheld. A secret field with filled=true is populated, not empty. Preserve a populated secret after the value resolver filled it unless the goal requests replacement or current validation rejects it. Presence does not prove the secret is correct.";

/// Empty redaction placeholders must not describe a populated secret as empty.
fn prepare(request: &Value) -> Value {
    let mut request = request.clone();
    let fields = request["state"]["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|element| crate::secret::is_secret(element))
        .filter_map(|element| {
            Some((
                element.get("node")?.as_u64()?,
                element.get("filled")?.as_bool()?,
            ))
        })
        .collect::<Vec<_>>();
    fn visit(value: &mut Value, fields: &[(u64, bool)]) {
        match value {
            Value::Object(object) => {
                if let Some((_, filled)) = object
                    .get("node")
                    .and_then(Value::as_u64)
                    .and_then(|node| fields.iter().find(|(id, _)| *id == node))
                {
                    object.remove("value");
                    object.remove("current_value");
                    object.insert("filled".into(), json!(filled));
                    object.insert(
                        "value_visibility".into(),
                        json!("Secret contents withheld; filled reports presence only."),
                    );
                }
                for child in object.values_mut() {
                    visit(child, fields);
                }
            }
            Value::Array(array) => {
                for child in array {
                    visit(child, fields);
                }
            }
            _ => {}
        }
    }
    visit(&mut request, &fields);
    if !fields.is_empty() {
        for question in request["questions"]
            .as_object_mut()
            .into_iter()
            .flat_map(|q| q.values_mut())
        {
            question["instructions"]["secret_fields"] = json!(SECRET_FIELDS);
        }
    }
    if let Some(criterion) = request
        .get_mut("questions")
        .and_then(|q| q.get_mut("operation"))
        .and_then(|q| q.get_mut("criteria"))
        .and_then(|q| q.get_mut("BLOCKED"))
    {
        *criterion = json!(
            "At least one requirement of user_goal is unmet or unproven, and none of the offered operations can advance it. Do not choose BLOCKED if the requested goal is already satisfied; choose DONE instead. The absence of controls after success is not a blocker."
        );
    }
    request
}

pub(super) fn predicate_body(request: &Value) -> anyhow::Result<Value> {
    request_body(&prepare(request))
}

fn sum(first: &Value, second: &Value) -> Value {
    serde_json::to_value(JevCallUsage::sum([first, second])).unwrap()
}

fn combine(first: anyhow::Result<Value>, second: anyhow::Result<Value>) -> anyhow::Result<Value> {
    let usage = |reply: &anyhow::Result<Value>| match reply {
        Ok(response) => response.get("usage").cloned().unwrap_or(Value::Null),
        Err(error) => JevBilled::usage_of(error).cloned().unwrap_or_else(|| {
            if crate::JevStop::status_of(error) == crate::JevStatus::BudgetExceeded {
                // Reservation was denied before contacting the provider.
                json!({"calls":0,"input_tokens":0,"output_tokens":0})
            } else {
                Value::Null
            }
        }),
    };
    let usage = sum(&usage(&first), &usage(&second));
    match (first, second) {
        (Ok(mut first), Ok(second)) => {
            let merged = (|| -> anyhow::Result<()> {
                first["answers"]
                    .as_array_mut()
                    .context("Invalid text Decisions answers")?
                    .extend(
                        second["answers"]
                            .as_array()
                            .context("Invalid visual Decisions answers")?
                            .iter()
                            .cloned(),
                    );
                Ok(())
            })();
            merged.map_err(|error| JevBilled::unusable(usage.clone(), error))?;
            first["usage"] = usage;
            Ok(first)
        }
        (Err(error), _) | (_, Err(error)) => Err(JevBilled::new(usage, error).into()),
    }
}

pub(super) async fn evaluate(
    transport: &dyn JevDecisionTransport,
    request: &Value,
    scoped: bool,
) -> anyhow::Result<Value> {
    let request = prepare(request);
    let mut body = request_body(&request)?;
    let extra = EVIDENCE.try_with(Clone::clone).unwrap_or_default();
    let mut state = request["state"].clone();
    state["user_goal"] = request["questions"]["operation"]["instructions"]["goal"].clone();
    state["recent_actions"] = extra["history"].clone();
    for entry in state["recent_actions"].as_array_mut().into_iter().flatten() {
        entry.as_object_mut().unwrap().remove("text_added");
    }
    body["input"] = json!(state.to_string());
    let Some(screenshot) = extra["screenshot"].as_str() else {
        return transport.decide(&body).await;
    };
    ensure!(
        screenshot.starts_with("data:image/"),
        "Screenshot must be an inline image"
    );
    let mut visual = body.clone();
    state["candidate_rectangles"] = extra["rectangles"].clone();
    visual["input"] = json!([{"role":"user","content":[{"type":"input_text","text":state.to_string()},{"type":"input_image","image_url":screenshot}]}]);
    if !scoped || extra["clipped_field"] != true {
        return transport.decide(&visual).await;
    }
    // Decisions images are shared input. Route independent target questions to
    // a visual batch while operation and safety predicates remain text-only.
    let target = |q: &Value| {
        q["type"] == "choice" && q["name"].as_str().is_some_and(|n| n.ends_with("_target"))
    };
    body["questions"]
        .as_array_mut()
        .unwrap()
        .retain(|q| !target(q));
    visual["questions"].as_array_mut().unwrap().retain(target);
    if visual["questions"].as_array().unwrap().is_empty() {
        // A sole offered target is resolved locally by the native adapter.
        return transport.decide(&body).await;
    }
    let (text, visual) = tokio::join!(transport.decide(&body), transport.decide(&visual));
    combine(text, visual)
}

#[cfg(test)]
mod tests;
