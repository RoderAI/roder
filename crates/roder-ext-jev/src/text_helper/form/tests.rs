//! The form call: what it is told, what reply it accepts, and its bodies.

use serde_json::json;

use super::super::responses;
use super::super::{Ask, TextHelper, request_body};
use super::*;
use crate::engine::JevTextValueResolver;
use crate::http::tests::{MockServer, Reply as Answer, fast_policy};
use crate::prompts::FORM_VALUES;
use crate::text_model::{Effort, Transport};
use crate::usage::JevBilled;

fn page() -> Value {
    json!({
        "title": "Contact us",
        "text": "Contact us\nName\nEmail\nMessage",
        "actions": [
            {"id": "e1", "node": 4, "kind": "fill", "role": "textbox", "label": "Name", "value": ""},
            {"id": "e2", "node": 5, "kind": "fill", "role": "textbox", "label": "Email",
             "input_type": "email", "value": ""},
            {"id": "e3", "node": 6, "kind": "fill", "role": "textbox", "label": "Start",
             "input_type": "date", "value": "", "context": "Newsletter"},
            {"id": "e4", "node": 7, "kind": "fill", "role": "textbox", "label": "Search", "value": "lamps"},
        ],
    })
}

/// The fields are listed by id, the chosen one first, each as a single
/// field's context describes it; `other_fields` leaves them out.
#[test]
fn the_form_context_lists_the_fields_by_id_and_the_rest_as_other_fields() {
    let page = page();
    let actions = page["actions"].as_array().unwrap();
    let fields = [&actions[1], &actions[0], &actions[2]];
    let today = NaiveDate::from_ymd_opt(2026, 9, 27).unwrap();
    let context = form_context("Write to Ada", &fields, &page, &[], today);
    let keys = context.as_object().unwrap().keys().collect::<Vec<_>>();
    assert_eq!(
        keys,
        [
            "goal",
            "date",
            "fields",
            "other_fields",
            "page",
            "recent_actions"
        ]
    );
    assert_eq!(
        context["fields"],
        json!([
            {"id": "e2", "label": "Email", "role": "textbox", "value": "", "input_type": "email"},
            {"id": "e1", "label": "Name", "role": "textbox", "value": ""},
            {"id": "e3", "label": "Start", "role": "textbox", "value": "", "input_type": "date",
             "format": "YYYY-MM-DD (ISO 8601, such as 2026-09-25)", "context": "Newsletter"},
        ])
    );
    assert_eq!(
        context["other_fields"],
        json!([{"label": "Search", "value": "lamps"}])
    );
    assert_eq!(
        context["date"],
        field_context("", &json!({}), &page, &[], today)["date"]
    );
}

#[test]
fn a_reply_gives_values_by_id_and_null_or_blank_is_none() {
    let ids = ["e1", "e2", "e3"];
    let values = parse_values(
        r#"{"fields": [{"id": "e1", "text": "Ada"}, {"id": "e2", "text": null}, {"id": "e3", "text": "  "}]}"#,
        &ids,
    )
    .unwrap();
    assert_eq!(
        values,
        BTreeMap::from([("e1".to_string(), "Ada".to_string())])
    );
    // A field the reply leaves out has no value; a fence is taken off.
    let values = parse_values(
        "```json\n{\"fields\": [{\"id\": \"e3\", \"text\": \"2026-10-12\"}]}\n```",
        &ids,
    )
    .unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values["e3"], "2026-10-12");
}

#[test]
fn an_unknown_or_repeated_id_or_another_shape_is_no_reply() {
    let ids = ["e1", "e2"];
    let long = "x".repeat(MAX_VALUE_CHARS + 1);
    for reply in [
        r#"{"fields": [{"id": "e9", "text": "Ada"}]}"#.to_string(),
        r#"{"fields": [{"id": "e1", "text": "Ada"}, {"id": "e1", "text": "Bea"}]}"#.to_string(),
        r#"{"fields": [], "extra": 1}"#.to_string(),
        r#"{"text": "Ada"}"#.to_string(),
        r#"{"fields": [{"id": "e1", "text": 4}]}"#.to_string(),
        format!(r#"{{"fields": [{{"id": "e1", "text": "{long}"}}]}}"#),
        "not json".to_string(),
    ] {
        assert!(parse_values(&reply, &ids).is_err(), "{reply}");
    }
}

/// The form call has its own instructions and twice a field's output cap,
/// over chat completions and over the Codex sign-in alike.
#[test]
fn the_form_bodies_carry_the_form_instructions_and_schema() {
    let context = json!({"goal": "g", "fields": []});
    let body = request_body(
        Ask::Form,
        &context,
        "m",
        "https://openrouter.ai/api/v1",
        None,
    );
    assert_eq!(body["messages"][0]["content"], json!(FORM_VALUES));
    assert_eq!(body["max_tokens"], json!(2048));
    assert_eq!(body["response_format"], json!({"type": "json_object"}));
    let body = responses::request_body(Ask::Form, &context, "gpt", Effort::Low);
    assert_eq!(body["instructions"], json!(FORM_VALUES));
    assert_eq!(body["text"]["format"], responses::form_format());
    let item = &responses::form_format()["schema"]["properties"]["fields"]["items"];
    assert_eq!(item["required"], json!(["id", "text"]));
}

fn helper(base_url: &str) -> TextHelper {
    TextHelper::with_policy(
        TextModel {
            model: "fixture-writer".into(),
            source: "explicit",
            note: None,
            fallback: None,
            transport: Transport::Chat {
                base_url: base_url.into(),
                api_key: "sk-writer".into(),
                reasoning: None,
            },
        },
        fast_policy(),
    )
}

fn completion(content: &str) -> Answer {
    Answer::ok(Box::leak(
        json!({"choices": [{"message": {"content": content}, "finish_reason": "stop"}],
               "usage": {"prompt_tokens": 300, "completion_tokens": 30, "cost": 0.0002}})
        .to_string()
        .into_boxed_str(),
    ))
}

/// One call writes every field it can; a password listed by mistake keeps
/// a value only when the goal holds it word for word.
#[tokio::test]
async fn one_call_writes_the_values_of_the_listed_fields() {
    let server = MockServer::start(vec![completion(
        r#"{"fields": [{"id": "e1", "text": "Ada"}, {"id": "e2", "text": "ada@example.com"}, {"id": "e5", "text": "hunter2"}]}"#,
    )])
    .await;
    let context = json!({
        "goal": "Sign up as Ada (ada@example.com)",
        "fields": [
            {"id": "e1", "label": "Name"},
            {"id": "e2", "label": "Email", "input_type": "email"},
            {"id": "e5", "label": "Password", "input_type": "password"},
        ],
    });
    let written = helper(&server.url)
        .resolve_form(&context)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        written.values,
        BTreeMap::from([
            ("e1".to_string(), "Ada".to_string()),
            ("e2".to_string(), "ada@example.com".to_string()),
        ])
    );
    assert_eq!(written.model, "fixture-writer");
    assert_eq!(written.usage["prompt_tokens"], json!(300));
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].1["messages"][0]["content"], json!(FORM_VALUES));
}

/// An email, phone or web address the goal does not hold is dropped, so
/// its field is asked for alone; one the goal holds is kept.
#[tokio::test]
async fn an_address_the_goal_does_not_hold_is_dropped() {
    let server = MockServer::start(vec![completion(
        r#"{"fields": [{"id": "e1", "text": "Ada Lovelace"}, {"id": "e2", "text": "email@example.com"}, {"id": "e3", "text": "+44 20 7946 0000"}, {"id": "e4", "text": "Hello"}]}"#,
    )])
    .await;
    let context = json!({
        "goal": "Write to us as Ada Lovelace (phone +44 20 7946 0000) saying hello, from my usual email.",
        "fields": [
            {"id": "e1", "label": "Name"},
            {"id": "e2", "label": "Email", "input_type": "email"},
            {"id": "e3", "label": "Phone", "input_type": "tel"},
            {"id": "e4", "label": "Message", "input_type": "textarea"},
        ],
    });
    let written = helper(&server.url)
        .resolve_form(&context)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        written.values.keys().collect::<Vec<_>>(),
        ["e1", "e3", "e4"]
    );
}

/// An unusable reply was still billed: the error carries its usage.
#[tokio::test]
async fn an_unusable_form_reply_keeps_its_usage() {
    let server = MockServer::start(vec![completion(
        r#"{"fields": [{"id": "e7", "text": "x"}]}"#,
    )])
    .await;
    let context = json!({"goal": "g", "fields": [{"id": "e1", "label": "Name"}]});
    let error = helper(&server.url)
        .resolve_form(&context)
        .await
        .unwrap_err();
    assert_eq!(
        JevBilled::usage_of(&error).unwrap()["prompt_tokens"],
        json!(300)
    );
}
