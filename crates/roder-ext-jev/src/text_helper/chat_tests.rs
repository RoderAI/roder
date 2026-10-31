//! The chat-completions transport: OpenRouter's provider routing, the
//! retry for a reply cut off at its output cap, and a fenced reply.

use super::*;
use crate::http::tests::{MockServer, Reply, fast_policy};

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

/// OpenRouter's provider routing goes to OpenRouter alone, by host: not to
/// another endpoint, nor to a proxy that only names openrouter.ai.
#[test]
fn provider_routing_is_sent_only_to_openrouter() {
    let context = json!({"goal": "g"});
    let routing = |base: &str| {
        request_body(&context, "m", base, None)
            .get("provider")
            .cloned()
    };
    let expected = Some(json!({"sort": "latency", "require_parameters": true}));
    assert_eq!(routing("https://openrouter.ai/api/v1"), expected);
    assert_eq!(routing("https://OpenRouter.ai/api/v1/"), expected);
    for other in [
        "https://api.example.test/v1",
        "https://api.deepseek.com/v1",
        "https://proxy.example.test/openrouter.ai/v1",
        "https://openrouter.ai.example.test/v1",
        "not a url",
    ] {
        assert_eq!(routing(other), None, "{other}");
    }
}

fn completion(content: &str, finish_reason: &str, usage: Value) -> Reply {
    Reply::ok(Box::leak(
        json!({"choices": [{"message": {"content": content}, "finish_reason": finish_reason}],
               "usage": usage})
        .to_string()
        .into_boxed_str(),
    ))
}

/// A reply cut off at the output cap is asked again once with four times
/// the cap; both calls are billed and counted.
#[tokio::test]
async fn a_reply_cut_off_at_the_cap_is_asked_again_with_four_times_the_room() {
    let server = MockServer::start(vec![
        completion(
            "{\"text\": \"Zur",
            "length",
            json!({"prompt_tokens": 40, "completion_tokens": 1024, "cost": 0.001}),
        ),
        completion(
            "{\"text\": \"Zurich\"}",
            "stop",
            json!({"prompt_tokens": 40, "completion_tokens": 9, "cost": 0.0005}),
        ),
    ])
    .await;

    let written = helper(&server.url)
        .resolve(&json!({"goal": "g"}))
        .await
        .unwrap();

    assert_eq!(written.value, "Zurich");
    let caps = server
        .requests()
        .iter()
        .map(|(_, body)| body["max_tokens"].clone())
        .collect::<Vec<_>>();
    assert_eq!(caps, [json!(1024), json!(4096)]);
    let usage = JevCallUsage::sum([&written.usage]);
    assert_eq!(usage.calls, 2);
    assert_eq!(usage.input_tokens, crate::usage::JevTokenCount::Known(80));
    assert_eq!(
        usage.output_tokens,
        crate::usage::JevTokenCount::Known(1033)
    );
    assert!((usage.cost.unwrap() - 0.0015).abs() < 1e-12);
}

/// A reply cut off twice is no value: the run ends as an error, nothing is
/// typed, and both billed calls ride on the error. A third is never sent.
#[tokio::test]
async fn a_reply_cut_off_twice_is_no_value() {
    let cut = || completion("{\"text\": \"Zur", "length", json!({"prompt_tokens": 40}));
    let server = MockServer::start(vec![cut(), cut(), cut()]).await;

    let error = helper(&server.url).resolve(&json!({})).await.unwrap_err();

    assert_eq!(JevStop::status_of(&error), JevStatus::Error);
    assert_eq!(error.to_string(), NO_VALUE);
    assert_eq!(server.hits(), 2);
    let billed = crate::usage::JevBilled::usage_of(&error).unwrap();
    assert_eq!(billed["calls"], 2);
    assert_eq!(billed["input_tokens"], 80);
}

/// The retry failing keeps the cut-off reply's usage on its error; a reply
/// that finished is never asked again.
#[tokio::test]
async fn a_failed_retry_keeps_the_first_calls_usage() {
    let server = MockServer::start(vec![
        completion("{\"text\": \"Zur", "length", json!({"prompt_tokens": 40})),
        Reply::status(402),
    ])
    .await;
    let error = helper(&server.url).resolve(&json!({})).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Text helper returned HTTP 402; nothing typed."
    );
    assert_eq!(
        crate::usage::JevBilled::usage_of(&error),
        Some(&json!({"prompt_tokens": 40}))
    );

    let finished = MockServer::start(vec![completion(
        "{\"text\": \"Zurich\"}",
        "stop",
        json!({"prompt_tokens": 40}),
    )])
    .await;
    let written = helper(&finished.url).resolve(&json!({})).await.unwrap();
    assert_eq!(written.usage, json!({"prompt_tokens": 40}));
    assert_eq!(finished.hits(), 1);
}

/// A code fence around or after the object is not part of the reply: the
/// writer's `{"text": "London"}` followed by a stray "```" is typed. What is
/// inside must still be exactly one usable `text`.
#[test]
fn a_fenced_reply_is_read_without_its_fence() {
    for fenced in [
        "{\"text\": \"London\"}\n```",
        "```json\n{\"text\": \"London\"}\n```",
        "```\n{\"text\": \"London\"}```",
        "  {\"text\": \"London\"}  ",
    ] {
        assert_eq!(
            parse_value(fenced).unwrap().as_deref(),
            Some("London"),
            "{fenced:?}"
        );
    }
    assert_eq!(parse_value("```json\n{\"text\": null}\n```").unwrap(), None);
    for bad in [
        "```json\n{\"value\": \"London\"}\n```",
        "London\n```",
        "``` {\"text\": \"London\"} ``` ```",
    ] {
        assert_eq!(
            parse_value(bad).unwrap_err().to_string(),
            NO_VALUE,
            "{bad:?}"
        );
    }
}

/// OpenRouter answers 404 when no endpoint of the model honours every
/// parameter (a non-reasoning model is sent `reasoning` too). That call was
/// not billed: it is asked again without `require_parameters`, which stays
/// off for the rest of the run, keeping the latency sort.
#[tokio::test]
async fn a_model_no_endpoint_fully_supports_is_asked_again_without_require_parameters() {
    let ok = || {
        completion(
            "{\"text\": \"Zurich\"}",
            "stop",
            json!({"prompt_tokens": 40}),
        )
    };
    let server = MockServer::start(vec![Reply::status(404), ok(), ok()]).await;
    let writer = helper(&server.url);
    let url = endpoint(&server.url);
    let body = request_body(
        &json!({"goal": "g"}),
        "m",
        "https://openrouter.ai/api/v1",
        None,
    );

    for _ in 0..2 {
        let (content, usage) = writer.chat(&url, "sk", body.clone()).await.unwrap();
        assert_eq!(content.unwrap(), "{\"text\": \"Zurich\"}");
        assert_eq!(usage, json!({"prompt_tokens": 40}));
    }

    let routing = server
        .requests()
        .iter()
        .map(|(_, body)| body["provider"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        routing,
        [
            json!({"sort": "latency", "require_parameters": true}),
            json!({"sort": "latency"}),
            json!({"sort": "latency"}),
        ]
    );

    // Any other endpoint's 404 is the run's error, asked once.
    let missing = MockServer::start(vec![Reply::status(404), ok()]).await;
    let error = helper(&missing.url).resolve(&json!({})).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Text helper returned HTTP 404; nothing typed."
    );
    assert_eq!(missing.hits(), 1);
}
