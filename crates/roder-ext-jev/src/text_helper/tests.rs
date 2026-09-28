use super::*;
use crate::http::tests::{MockServer, Reply, fast_policy};

fn fixtures() -> (Value, Value, Value) {
    (
        serde_json::from_str(include_str!("../../tests/fixtures/field_context.json")).unwrap(),
        serde_json::from_str(include_str!(
            "../../tests/fixtures/field_text_requests.json"
        ))
        .unwrap(),
        serde_json::from_str::<Value>(include_str!("../../tests/fixtures/fingerprint.json"))
            .unwrap()["page"]
            .clone(),
    )
}

#[test]
fn field_context_matches_upstream() {
    let (context_fixture, _, page) = fixtures();
    let choose: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/choose_request.json")).unwrap();
    let history = choose["history"].as_array().unwrap().clone();
    let ours = field_context(
        context_fixture["goal"].as_str().unwrap(),
        &context_fixture["action"],
        &page,
        &history,
        fixture_day(),
    );
    assert_eq!(ours, context_fixture["context"]);
    assert_eq!(
        serde_json::to_string(&ours).unwrap(),
        serde_json::to_string(&context_fixture["context"]).unwrap()
    );
}

/// The day the field-context fixture was recorded on.
fn fixture_day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 27).unwrap()
}

#[test]
fn today_names_the_weekday_and_the_next_two_months() {
    assert_eq!(
        today(fixture_day()),
        "2026-09-27 (Sunday). In one month: October 2026; in two months: November 2026"
    );
    // Across a year end, and from the last day of a long month.
    assert_eq!(
        today(NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()),
        "2026-11-30 (Monday). In one month: December 2026; in two months: January 2027"
    );
    assert_eq!(
        today(NaiveDate::from_ymd_opt(2027, 1, 31).unwrap()),
        "2027-01-31 (Sunday). In one month: February 2027; in two months: March 2027"
    );
}

#[test]
fn the_helper_is_told_the_fields_context_and_the_other_fields() {
    let page = json!({"title": "Copy", "text": "Copy the note", "actions": [
        {"id": "e1", "node": 1, "kind": "fill", "role": "textbox", "label": "Note",
         "value": "Meet at nine", "input_type": "textarea"},
        {"id": "e2", "node": 1, "kind": "click", "role": "textbox", "label": "Open Note",
         "value": "Meet at nine"},
        {"id": "e3", "node": 2, "kind": "fill", "role": "textbox", "label": "Answer",
         "value": "", "context": "Reply"},
        {"id": "e4", "node": 3, "kind": "select", "role": "combobox", "label": "Size → L",
         "value": "l", "current_value": "M"},
        {"id": "e5", "node": 3, "kind": "select", "role": "combobox", "label": "Size → S",
         "value": "s", "current_value": "M"},
        {"id": "e6", "node": 4, "kind": "click", "role": "button", "label": "Submit",
         "value": ""},
    ]});
    let answer = page["actions"][2].clone();
    let context = field_context("Copy the note", &answer, &page, &[], fixture_day());
    assert_eq!(context["field"]["context"], "Reply");
    assert_eq!(
        context["other_fields"],
        json!([
            {"label": "Note", "input_type": "textarea", "value": "Meet at nine"},
            {"label": "Size", "value": "M"},
        ])
    );
    assert_eq!(
        context.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "goal",
            "date",
            "field",
            "other_fields",
            "page",
            "recent_actions"
        ]
    );
    // A page with no other field lists none.
    let alone = json!({"title": "t", "text": "", "actions": [answer.clone()]});
    let context = field_context("g", &answer, &alone, &[], fixture_day());
    assert!(context.get("other_fields").is_none());
}

#[test]
fn a_native_date_field_is_told_its_iso_format() {
    let page = json!({"title": "Book", "text": "Departure"});
    let action = json!({"label": "Departure", "role": "textbox", "value": "",
        "input_type": "date"});
    let context = field_context("Leave on 12 October", &action, &page, &[], fixture_day());
    assert_eq!(context["field"]["input_type"], "date");
    assert_eq!(
        context["field"]["format"],
        "YYYY-MM-DD (ISO 8601, such as 2026-09-25)"
    );
    for input_type in ["datetime-local", "month", "week", "time"] {
        assert!(iso_format(input_type).is_some(), "{input_type}");
    }
    // Any other field is described as before.
    let plain = json!({"label": "Name", "role": "textbox", "value": ""});
    let context = field_context("g", &plain, &page, &[], fixture_day());
    assert_eq!(
        context["field"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["label", "role", "value"]
    );
}

#[test]
fn request_bodies_match_upstream_for_every_provider_shape() {
    let (context_fixture, requests, _) = fixtures();
    let context = &context_fixture["context"];
    for (name, expected) in requests.as_object().unwrap() {
        // Upstream's default is no level; its `TEXT_MODEL_REASONING=none`
        // is `JEV_TEXT_MODEL_REASONING=none`.
        let none = Some(Effort::None);
        let (base, effort) = match name.as_str() {
            "deepseek_default" => ("https://api.deepseek.com/v1", None),
            "deepseek_none" => ("https://api.deepseek.com/v1", none),
            "openrouter_default" => ("https://openrouter.ai/api/v1", None),
            "openrouter_none" => ("https://openrouter.ai/api/v1", none),
            "other_default" => ("https://api.example.test/v1", None),
            "other_none" => ("https://api.example.test/v1", none),
            other => panic!("unexpected fixture {other}"),
        };
        let ours = request_body(context, "fixture-writer", base, effort);
        assert_eq!(
            serde_json::to_string(&ours).unwrap(),
            serde_json::to_string(&expected["body"]).unwrap(),
            "body for {name}"
        );
        assert_eq!(endpoint(base), expected["url"].as_str().unwrap());
    }
}

/// Levels upstream never sends: named by `JEV_TEXT_MODEL_REASONING`.
#[test]
fn a_named_level_is_sent_as_the_endpoint_takes_it() {
    let context = json!({"goal": "g"});
    let body = |base: &str, effort| request_body(&context, "m", base, Some(effort));
    let other = body("https://api.example.test/v1", Effort::Medium);
    assert_eq!(other["reasoning"], json!({"effort": "medium"}));
    // DeepSeek has no levels: any but none turns its thinking on.
    let deepseek = body("https://api.deepseek.com/v1", Effort::High);
    assert_eq!(deepseek["thinking"], json!({"type": "enabled"}));
    assert!(deepseek.get("reasoning").is_none());
}

#[test]
fn only_a_lone_usable_text_key_is_accepted() {
    assert_eq!(
        parse_value(r#"{"text": "Zurich"}"#).unwrap().as_deref(),
        Some("Zurich")
    );
    // Upstream rejects each of these: the first three as a missing value.
    assert_eq!(parse_value(r#"{"text": null}"#).unwrap(), None);
    assert_eq!(parse_value(r#"{"text": "   "}"#).unwrap(), None);
    assert_eq!(parse_value(r#"{"text": ""}"#).unwrap(), None);
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
fn a_bad_reply_is_an_error() {
    let status = |content: &str| JevStop::status_of(&parse_value(content).unwrap_err());
    for bad in [
        "not json",
        r#"{"text": 12}"#,
        r#"{"text": null, "note": "extra"}"#,
        r#"{"value": "ok"}"#,
    ] {
        assert_eq!(status(bad), JevStatus::Error, "{bad}");
        assert_eq!(parse_value(bad).unwrap_err().to_string(), NO_VALUE);
    }
}

#[test]
fn endpoint_trims_a_trailing_slash() {
    assert_eq!(
        endpoint("https://api.deepseek.com/v1/"),
        "https://api.deepseek.com/v1/chat/completions"
    );
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

#[tokio::test]
async fn text_helper_retries_an_overloaded_provider_then_types() {
    let server = MockServer::start(vec![
        Reply::status(503),
        Reply::ok(r#"{"choices":[{"message":{"content":"{\"text\": \"Zurich\"}"}}],"usage":{"total_tokens":7}}"#),
    ])
    .await;
    let base = server.url.trim_end_matches("/v1/systemone").to_string() + "/v1";

    let written = helper(&base).resolve(&json!({"goal": "g"})).await.unwrap();

    assert_eq!(written.value, "Zurich");
    assert_eq!(written.model, "fixture-writer");
    assert_eq!(written.usage["total_tokens"], json!(7));
    assert_eq!(server.hits(), 2);
    assert_eq!(server.requests()[0].0, "Bearer sk-writer");
}

#[tokio::test]
async fn text_helper_keeps_the_nothing_typed_messages() {
    let refused = MockServer::start(vec![Reply::status(402)]).await;
    let error = helper(&refused.url).resolve(&json!({})).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Text helper returned HTTP 402; nothing typed."
    );
    assert_eq!(JevStop::status_of(&error), JevStatus::Error);
    assert_eq!(refused.hits(), 1);

    let overloaded = MockServer::start(vec![Reply::status(429)]).await;
    let error = helper(&overloaded.url)
        .resolve(&json!({}))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Text helper returned HTTP 429; nothing typed."
    );
    // Still overloaded after every retry: the provider is unavailable.
    assert_eq!(JevStop::status_of(&error), JevStatus::Unavailable);
    assert_eq!(overloaded.hits(), 6);
}

/// A reply without a usable value was still billed: its usage rides on the
/// error, which still ends the run as before.
#[tokio::test]
async fn an_unusable_reply_keeps_its_usage() {
    let missing = MockServer::start(vec![Reply::ok(
        r#"{"choices":[{"message":{"content":"{\"text\": null}"}}],"usage":{"prompt_tokens":30,"completion_tokens":3}}"#,
    )])
    .await;
    let error = helper(&missing.url)
        .resolve(&json!({"field": {"label": "City"}}))
        .await
        .unwrap_err();
    assert_eq!(JevStop::status_of(&error), JevStatus::NeedsInput);
    assert_eq!(
        error.to_string(),
        "The goal gives no value for the field \"City\"; nothing typed."
    );
    assert_eq!(
        crate::usage::JevBilled::usage_of(&error),
        Some(&json!({"prompt_tokens": 30, "completion_tokens": 3}))
    );

    let garbled = MockServer::start(vec![Reply::ok(
        r#"{"choices":[{"message":{"content":"Zurich"}}],"usage":{"prompt_tokens":9}}"#,
    )])
    .await;
    let error = helper(&garbled.url).resolve(&json!({})).await.unwrap_err();
    assert_eq!(JevStop::status_of(&error), JevStatus::Error);
    assert_eq!(error.to_string(), NO_VALUE);
    assert_eq!(
        crate::usage::JevBilled::usage_of(&error),
        Some(&json!({"prompt_tokens": 9}))
    );
}

fn reply(text: &str) -> Reply {
    // The mock serves static bodies; a test leaks a few bytes per reply.
    Reply::ok(Box::leak(
        format!(
            r#"{{"choices":[{{"message":{{"content":{}}}}}],"usage":{{"prompt_tokens":5}}}}"#,
            serde_json::to_string(&json!({"text": text}).to_string()).unwrap()
        )
        .into_boxed_str(),
    ))
}

/// A password or code the helper writes must be in the goal word for word:
/// one it made up ends the run `needs_input`, naming the field, with nothing
/// typed; one the goal holds is typed.
#[tokio::test]
async fn a_secret_the_goal_does_not_hold_is_never_typed() {
    let context = |goal: &str, input_type: &str| {
        json!({"goal": goal, "field": {"label": "Code", "role": "textbox", "value": "",
               "input_type": input_type}})
    };
    let guessed = MockServer::start(vec![reply("123456")]).await;
    let error = helper(&guessed.url)
        .resolve(&context("Verify the account", "one-time-code"))
        .await
        .unwrap_err();
    assert_eq!(JevStop::status_of(&error), JevStatus::NeedsInput);
    assert_eq!(
        error.to_string(),
        "The goal gives no value for the field \"Code\"; nothing typed."
    );
    assert!(!format!("{error:#}").contains("123456"));
    assert!(crate::usage::JevBilled::usage_of(&error).is_some());

    let given = MockServer::start(vec![reply("482913")]).await;
    let written = helper(&given.url)
        .resolve(&context("Verify with code 482913", "one-time-code"))
        .await
        .unwrap();
    assert_eq!(written.value, "482913");

    let password = MockServer::start(vec![reply("hunter2")]).await;
    let error = helper(&password.url)
        .resolve(&context("Sign in as ada", "password"))
        .await
        .unwrap_err();
    assert_eq!(JevStop::status_of(&error), JevStatus::NeedsInput);

    // Any other field may still be inferred from the goal.
    let inferred = MockServer::start(vec![reply("Zurich")]).await;
    let written = helper(&inferred.url)
        .resolve(&context("Fly to the largest Swiss city", "text"))
        .await
        .unwrap();
    assert_eq!(written.value, "Zurich");
}

#[test]
fn secret_fields_are_not_listed_among_the_other_fields() {
    let page = json!({"title": "Sign in", "text": "", "actions": [
        {"node": 1, "kind": "fill", "label": "Username", "value": "ada", "input_type": "text"},
        {"node": 2, "kind": "fill", "label": "Password", "value": "", "input_type": "password",
         "filled": true},
        {"node": 3, "kind": "fill", "label": "Code", "value": "", "input_type": "one-time-code",
         "filled": false},
    ]});
    let context = field_context("Sign in", &page["actions"][2], &page, &[], fixture_day());
    assert_eq!(
        context["other_fields"],
        json!([{"label": "Username", "input_type": "text", "value": "ada"}])
    );
    assert_eq!(context["field"]["input_type"], json!("one-time-code"));
}
