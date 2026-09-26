//! The helper that writes one field value.
//!
//! A port of upstream Jev's `model.field_context` and `model.field_text`. The
//! request shape, including which reasoning field each provider gets, and the
//! strict `{"text": ...}` reply contract are reproduced exactly; Roder supplies
//! the endpoint and key from its own providers (see `text_model`).

use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};

use crate::prompts::TEXT_VALUE;
use crate::python_json;
use crate::text_model::TextModel;

const TIMEOUT: Duration = Duration::from_secs(25);
const MAX_VALUE_CHARS: usize = 2000;

/// What the helper wrote, and what it cost.
#[derive(Debug, Clone)]
pub(crate) struct WrittenText {
    pub(crate) value: String,
    pub(crate) model: String,
    pub(crate) latency_ms: u64,
    pub(crate) usage: Value,
}

/// Everything the helper is told about the field it is filling.
pub(crate) fn field_context(goal: &str, action: &Value, page: &Value, history: &[Value]) -> Value {
    let mut field = Map::new();
    for key in ["label", "role", "value"] {
        field.insert(key.into(), action.get(key).cloned().unwrap_or(Value::Null));
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
    json!({
        "goal": goal,
        "field": Value::Object(field),
        "page": {"title": page["title"], "text": text},
        "recent_actions": recent,
    })
}

/// Upstream selects the reasoning field by base URL, and `TEXT_MODEL_REASONING=none`
/// overrides both shapes.
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

/// Accept only `{"text": "..."}` with a usable value, as upstream does.
pub(crate) fn parse_value(content: &str) -> anyhow::Result<String> {
    let parsed: Value = serde_json::from_str(content).map_err(|_| {
        anyhow::anyhow!("Text helper returned no valid field value; nothing typed.")
    })?;
    let object = parsed
        .as_object()
        .context("Text helper returned no valid field value; nothing typed.")?;
    let only_text = object.len() == 1 && object.contains_key("text");
    let value = object
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !only_text || value.trim().is_empty() || value.chars().count() > MAX_VALUE_CHARS {
        bail!("Text helper returned no valid field value; nothing typed.");
    }
    Ok(value.to_string())
}

/// Ask Roder's configured text model for the value of one field.
pub(crate) async fn field_text(context: &Value, text: &TextModel) -> anyhow::Result<WrittenText> {
    let body = request_body(context, &text.model, &text.base_url, text.reasoning_none);
    let started = Instant::now();
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .context("build the text-helper client")?;
    let response = client
        .post(endpoint(&text.base_url))
        .bearer_auth(&text.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Text helper connection failed; nothing typed."))?;
    let status = response.status().as_u16();
    if status >= 400 {
        bail!("Text helper returned HTTP {status}; nothing typed.");
    }
    let result: Value = response
        .json()
        .await
        .context("decode the text-helper response")?;
    let content = result["choices"][0]["message"]["content"]
        .as_str()
        .context("Text helper returned no valid field value; nothing typed.")?;
    Ok(WrittenText {
        value: parse_value(content)?,
        model: text.model.clone(),
        latency_ms: started.elapsed().as_millis() as u64,
        usage: result.get("usage").cloned().unwrap_or_else(|| json!({})),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> (Value, Value, Value) {
        (
            serde_json::from_str(include_str!("../tests/fixtures/field_context.json")).unwrap(),
            serde_json::from_str(include_str!("../tests/fixtures/field_text_requests.json"))
                .unwrap(),
            serde_json::from_str::<Value>(include_str!("../tests/fixtures/fingerprint.json"))
                .unwrap()["page"]
                .clone(),
        )
    }

    #[test]
    fn field_context_matches_upstream() {
        let (context_fixture, _, page) = fixtures();
        let choose: Value =
            serde_json::from_str(include_str!("../tests/fixtures/choose_request.json")).unwrap();
        let history = choose["history"].as_array().unwrap().clone();
        let ours = field_context(
            context_fixture["goal"].as_str().unwrap(),
            &context_fixture["action"],
            &page,
            &history,
        );
        assert_eq!(ours, context_fixture["context"]);
        assert_eq!(
            serde_json::to_string(&ours).unwrap(),
            serde_json::to_string(&context_fixture["context"]).unwrap()
        );
    }

    #[test]
    fn request_bodies_match_upstream_for_every_provider_shape() {
        let (context_fixture, requests, _) = fixtures();
        let context = &context_fixture["context"];
        for (name, expected) in requests.as_object().unwrap() {
            let (base, none) = match name.as_str() {
                "deepseek_default" => ("https://api.deepseek.com/v1", false),
                "deepseek_none" => ("https://api.deepseek.com/v1", true),
                "openrouter_default" => ("https://openrouter.ai/api/v1", false),
                "openrouter_none" => ("https://openrouter.ai/api/v1", true),
                "other_default" => ("https://api.example.test/v1", false),
                "other_none" => ("https://api.example.test/v1", true),
                other => panic!("unexpected fixture {other}"),
            };
            let ours = request_body(context, "fixture-writer", base, none);
            assert_eq!(
                serde_json::to_string(&ours).unwrap(),
                serde_json::to_string(&expected["body"]).unwrap(),
                "body for {name}"
            );
            assert_eq!(endpoint(base), expected["url"].as_str().unwrap());
        }
    }

    #[test]
    fn only_a_lone_usable_text_key_is_accepted() {
        assert_eq!(parse_value(r#"{"text": "Zurich"}"#).unwrap(), "Zurich");
        // Upstream rejects each of these.
        assert!(parse_value(r#"{"text": null}"#).is_err());
        assert!(parse_value(r#"{"text": "   "}"#).is_err());
        assert!(parse_value(r#"{"text": ""}"#).is_err());
        assert!(parse_value(r#"{"text": 12}"#).is_err());
        assert!(parse_value(r#"{"text": "ok", "note": "extra"}"#).is_err());
        assert!(parse_value(r#"{"value": "ok"}"#).is_err());
        assert!(parse_value("not json").is_err());
        let long = format!(r#"{{"text": "{}"}}"#, "x".repeat(2001));
        assert!(parse_value(&long).is_err());
        let limit = format!(r#"{{"text": "{}"}}"#, "x".repeat(2000));
        assert!(parse_value(&limit).is_ok());
    }

    #[test]
    fn endpoint_trims_a_trailing_slash() {
        assert_eq!(
            endpoint("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }
}
