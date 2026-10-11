//! The Responses path against a local mock of the Codex backend.

use std::sync::Mutex;

use super::responses::{CodexToken, request_body as responses_body, value_format};
use super::*;
use crate::http::tests::{MockServer, Reply, fast_policy};
use crate::usage::{JevCallUsage, JevTokenCount};

/// Hands out the scripted tokens in turn, the last one repeating.
struct FakeAuth(Mutex<Vec<Option<&'static str>>>);

impl FakeAuth {
    fn new(tokens: &[Option<&'static str>]) -> Arc<Self> {
        Arc::new(Self(Mutex::new(tokens.to_vec())))
    }
}

#[async_trait]
impl CodexAuth for FakeAuth {
    async fn token(&self) -> anyhow::Result<Option<CodexToken>> {
        let mut tokens = self.0.lock().unwrap();
        let token = if tokens.len() > 1 {
            tokens.remove(0)
        } else {
            tokens[0]
        };
        Ok(token.map(|access| CodexToken {
            access: access.into(),
            account_id: Some("acct-test".into()),
        }))
    }
}

struct BrokenAuth;

#[async_trait]
impl CodexAuth for BrokenAuth {
    async fn token(&self) -> anyhow::Result<Option<CodexToken>> {
        anyhow::bail!("codex token request failed: 400 {{\"refresh_token\":\"rt-leak\"}}")
    }
}

fn codex(server: &MockServer, auth: Arc<dyn CodexAuth>, effort: Effort) -> TextHelper {
    codex_with(server, auth, effort, None)
}

/// The Codex model with `fallback` behind it, as a default choice is.
fn codex_with(
    server: &MockServer,
    auth: Arc<dyn CodexAuth>,
    effort: Effort,
    fallback: Option<TextModel>,
) -> TextHelper {
    TextHelper {
        model: Mutex::new(TextModel {
            model: "gpt-6-sol".into(),
            source: "codex",
            note: None,
            fallback: fallback.map(Box::new),
            transport: Transport::Codex { effort },
        }),
        http: JsonPoster::new(fast_policy()),
        codex: auth,
        codex_base_url: server.url.trim_end_matches("/v1/systemone").to_string(),
    }
}

/// A chat-completions model on `server`, as resolution puts behind the
/// default Codex choice.
fn chat_fallback(server: &MockServer) -> TextModel {
    TextModel {
        model: "deepseek-chat".into(),
        source: "roder-provider",
        note: None,
        fallback: None,
        transport: Transport::Chat {
            base_url: server.url.trim_end_matches("/v1/systemone").to_string() + "/v1",
            api_key: "sk-fallback".into(),
            reasoning: None,
        },
    }
}

fn chat_reply(text: &str) -> Reply {
    let body = json!({"choices": [{"message": {"content": value(Some(text))}}],
        "usage": {"prompt_tokens": 9, "completion_tokens": 2}});
    Reply::ok(Box::leak(body.to_string().into_boxed_str()))
}

/// A streamed reply whose message says `text`, as the Codex backend sends it.
fn streamed(text: &str) -> Reply {
    let message = json!({"type": "message", "role": "assistant",
        "content": [{"type": "output_text", "text": text}]});
    let usage = json!({"attribution": {"items": {}}, "input_tokens": 812,
        "input_tokens_details": {"cached_tokens": 640},
        "output_tokens": 19, "output_tokens_details": {"reasoning_tokens": 11},
        "total_tokens": 831});
    let body = [
        json!({"type": "response.created", "response": {"status": "in_progress"}}),
        json!({"type": "response.output_text.delta", "delta": text}),
        json!({"type": "response.output_item.done", "item": message}),
        json!({"type": "response.completed",
               "response": {"status": "completed", "output": [message], "usage": usage}}),
    ]
    .iter()
    .map(|event| {
        format!(
            "event: {}\ndata: {event}\n\n",
            event["type"].as_str().unwrap()
        )
    })
    .collect::<String>();
    Reply::sse(Box::leak(body.into_boxed_str()))
}

fn value(text: Option<&str>) -> String {
    json!({"text": text}).to_string()
}

#[test]
fn the_request_is_the_responses_mapping_of_the_same_prompt() {
    let context = json!({"goal": "Fly to Zurich", "field": {"label": "City"}});
    let body = responses_body(&context, "gpt-6-sol", Effort::Low);
    assert_eq!(body["model"], "gpt-6-sol");
    assert_eq!(body["stream"], true);
    assert_eq!(body["store"], false);
    assert_eq!(body["instructions"], TEXT_VALUE);
    assert_eq!(body["reasoning"], json!({"effort": "low"}));
    assert_eq!(body["text"], json!({"format": value_format()}));
    assert_eq!(
        body["input"],
        json!([{"type": "message", "role": "user",
                "content": [{"type": "input_text", "text": python_json::dumps(&context)}]}])
    );
    let mut keys = body
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    assert_eq!(
        keys,
        [
            "input",
            "instructions",
            "model",
            "reasoning",
            "store",
            "stream",
            "text"
        ]
    );
    // The schema admits exactly one key, a string or null.
    let schema = &value_format()["schema"];
    assert_eq!(schema["required"], json!(["text"]));
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(
        responses_body(&context, "gpt-6-sol", Effort::None)["reasoning"],
        json!({"effort": "none"})
    );
}

#[tokio::test]
async fn a_streamed_reply_types_its_value_and_reports_its_usage() {
    let server = MockServer::start(vec![streamed(&value(Some("Zurich")))]).await;
    let helper = codex(&server, FakeAuth::new(&[Some("tok-live")]), Effort::Low);

    let written = helper
        .resolve(&json!({"goal": "Fly to Zurich", "field": {"label": "City"}}))
        .await
        .unwrap();

    assert_eq!(written.value, "Zurich");
    assert_eq!(written.model, "gpt-6-sol");
    let (authorization, body) = &server.requests()[0];
    assert_eq!(authorization, "Bearer tok-live");
    assert_eq!(body["reasoning"], json!({"effort": "low"}));
    let head = &server.heads()[0];
    assert!(head.starts_with("POST /responses HTTP/1.1"), "{head}");
    let lower = head.to_ascii_lowercase();
    assert!(lower.contains("chatgpt-account-id: acct-test"), "{head}");
    assert!(lower.contains("originator: roder"), "{head}");
    // The Responses counts sum like any other call's; reasoning tokens are
    // already inside output_tokens, cached ones inside input_tokens.
    assert!(written.usage.get("attribution").is_none());
    assert_eq!(
        written.usage["output_tokens_details"]["reasoning_tokens"],
        11
    );
    let usage = JevCallUsage::sum([&written.usage]);
    assert_eq!(usage.input_tokens, JevTokenCount::Known(812));
    assert_eq!(usage.output_tokens, JevTokenCount::Known(19));
}

/// The Codex backend streams without a content type; the stream is still
/// read as one.
#[tokio::test]
async fn a_stream_without_a_content_type_is_still_read() {
    let Reply::Status(_, _, body) = streamed(&value(Some("Bern"))) else {
        unreachable!()
    };
    let server =
        MockServer::start(vec![Reply::Status(200, vec![("content-type", "")], body)]).await;
    let written = codex(&server, FakeAuth::new(&[Some("t")]), Effort::Low)
        .resolve(&json!({"goal": "Bern", "field": {"label": "City"}}))
        .await
        .unwrap();
    assert_eq!(written.value, "Bern");
}

#[tokio::test]
async fn the_reply_is_held_to_the_same_contract() {
    let context = json!({"goal": "Fly somewhere", "field": {"label": "City"}});
    // A null is the goal lacking the value: needs_input, still billed.
    let null = MockServer::start(vec![streamed(&value(None))]).await;
    let error = codex(&null, FakeAuth::new(&[Some("t")]), Effort::Low)
        .resolve(&context)
        .await
        .unwrap_err();
    assert_eq!(JevStop::status_of(&error), JevStatus::NeedsInput);
    assert_eq!(
        crate::usage::JevBilled::usage_of(&error).unwrap()["output_tokens"],
        json!(19)
    );
    // Anything but a lone text key is an error, as on the other path.
    for bad in [r#"{"text":"a","note":"b"}"#, r#"{"value":"a"}"#, "Zurich"] {
        let server = MockServer::start(vec![streamed(bad)]).await;
        let error = codex(&server, FakeAuth::new(&[Some("t")]), Effort::Low)
            .resolve(&context)
            .await
            .unwrap_err();
        assert_eq!(JevStop::status_of(&error), JevStatus::Error, "{bad}");
        assert_eq!(error.to_string(), NO_VALUE, "{bad}");
    }
    // A reply cut short has no value.
    let incomplete = Reply::sse(
        "data: {\"type\":\"response.incomplete\",\"response\":{\"status\":\"incomplete\",\"output\":[],\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n\n",
    );
    let server = MockServer::start(vec![incomplete]).await;
    let error = codex(&server, FakeAuth::new(&[Some("t")]), Effort::Low)
        .resolve(&context)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), NO_VALUE);
}

/// A password the goal does not hold is never typed, whichever path wrote it.
#[tokio::test]
async fn a_secret_the_goal_does_not_hold_is_never_typed() {
    let server = MockServer::start(vec![streamed(&value(Some("hunter2")))]).await;
    let error = codex(&server, FakeAuth::new(&[Some("t")]), Effort::Low)
        .resolve(&json!({"goal": "Sign in as ada",
            "field": {"label": "Password", "input_type": "password"}}))
        .await
        .unwrap_err();
    assert_eq!(JevStop::status_of(&error), JevStatus::NeedsInput);
    assert!(!format!("{error:#}").contains("hunter2"));
}

#[tokio::test]
async fn a_refused_token_is_fetched_again_and_sent_once_more() {
    let server = MockServer::start(vec![Reply::status(401), streamed(&value(Some("Oslo")))]).await;
    let auth = FakeAuth::new(&[Some("tok-old"), Some("tok-new")]);
    let written = codex(&server, auth, Effort::Low)
        .resolve(&json!({"goal": "Oslo", "field": {"label": "City"}}))
        .await
        .unwrap();
    assert_eq!(written.value, "Oslo");
    let sent = server
        .requests()
        .into_iter()
        .map(|(authorization, _)| authorization)
        .collect::<Vec<_>>();
    assert_eq!(sent, ["Bearer tok-old", "Bearer tok-new"]);
}

#[tokio::test]
async fn the_same_token_refused_twice_ends_the_run_without_quoting_it() {
    let server = MockServer::start(vec![Reply::status(401)]).await;
    let error = codex(&server, FakeAuth::new(&[Some("tok-stale")]), Effort::Low)
        .resolve(&json!({"goal": "g"}))
        .await
        .unwrap_err();
    assert_eq!(JevStop::status_of(&error), JevStatus::Error);
    let message = format!("{error:#}");
    assert!(message.contains("HTTP 401"), "{message}");
    assert!(message.contains("roder auth login codex"), "{message}");
    assert!(!message.contains("tok-stale"), "{message}");
    // Not resent with a token it already knows is refused.
    assert_eq!(server.hits(), 1);

    // Refused again after a new token: no third attempt.
    let server = MockServer::start(vec![Reply::status(401)]).await;
    let auth = FakeAuth::new(&[Some("tok-a"), Some("tok-b")]);
    codex(&server, auth, Effort::Low)
        .resolve(&json!({"goal": "g"}))
        .await
        .unwrap_err();
    assert_eq!(server.hits(), 2);
}

#[tokio::test]
async fn no_sign_in_or_a_failed_refresh_ends_the_run_before_any_request() {
    let server = MockServer::start(vec![streamed(&value(Some("x")))]).await;
    let error = codex(&server, FakeAuth::new(&[None]), Effort::Low)
        .resolve(&json!({"goal": "g"}))
        .await
        .unwrap_err();
    assert_eq!(JevStop::status_of(&error), JevStatus::Error);
    assert!(error.to_string().contains("no Codex sign-in"), "{error}");

    let error = codex(&server, Arc::new(BrokenAuth), Effort::Low)
        .resolve(&json!({"goal": "g"}))
        .await
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("could not refresh"), "{message}");
    // The auth crate's error, which can quote the token endpoint, is not.
    assert!(!message.contains("rt-leak"), "{message}");
    assert_eq!(server.hits(), 0);
}

#[tokio::test]
async fn rejections_keep_the_backends_status_and_message() {
    let server = MockServer::start(vec![Reply::Status(
        400,
        Vec::new(),
        r#"{"detail":"Unsupported parameter: max_output_tokens"}"#,
    )])
    .await;
    let error = codex(&server, FakeAuth::new(&[Some("tok-x")]), Effort::Low)
        .resolve(&json!({"goal": "g"}))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Text helper (Codex) returned HTTP 400 ({\"detail\":\"Unsupported parameter: \
         max_output_tokens\"}); nothing typed."
    );
    assert!(!format!("{error:#}").contains("tok-x"));

    let failed = Reply::sse(
        "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"model_not_found\",\"message\":\"no access\"}}}\n\n",
    );
    let server = MockServer::start(vec![failed]).await;
    let error = codex(&server, FakeAuth::new(&[Some("t")]), Effort::Low)
        .resolve(&json!({"goal": "g"}))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Text helper (Codex) failed (model_not_found: no access); nothing typed."
    );
    assert_eq!(server.hits(), 1);
}

#[tokio::test]
async fn the_retry_schedule_and_resend_limit_still_hold() {
    let busy = MockServer::start(vec![
        Reply::status(429),
        Reply::status(503),
        streamed(&value(Some("Bern"))),
    ])
    .await;
    let written = codex(&busy, FakeAuth::new(&[Some("t")]), Effort::Low)
        .resolve(&json!({"goal": "Bern", "field": {"label": "City"}}))
        .await
        .unwrap();
    assert_eq!(written.value, "Bern");
    assert_eq!(busy.hits(), 3);

    // A cut-off stream may have been billed: sent again only once.
    let cut = MockServer::start(vec![Reply::sse(
        "data: {\"type\":\"response.created\",\"response\":{}}\n\n",
    )])
    .await;
    let error = codex(&cut, FakeAuth::new(&[Some("t")]), Effort::Low)
        .resolve(&json!({"goal": "g"}))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Text helper (Codex) sent an unreadable reply; nothing typed."
    );
    assert_eq!(cut.hits(), 2);
}

#[test]
fn a_tokens_debug_output_leaves_it_out() {
    let token = CodexToken {
        access: "tok-secret".into(),
        account_id: Some("acct-secret".into()),
    };
    let shown = format!("{token:?}");
    assert!(
        !shown.contains("tok-secret") && !shown.contains("acct-secret"),
        "{shown}"
    );
}

/// One field through the real Codex sign-in. Prints the model, latency and
/// usage, or the backend's exact refusal; never the token.
#[tokio::test]
#[ignore = "live: needs a Codex sign-in (`roder auth login codex`) and spends a model call"]
async fn live_codex_text_helper() {
    let helper = TextHelper::new(TextModel {
        model: crate::text_model::CODEX_MODEL.into(),
        source: "codex",
        note: None,
        fallback: None,
        transport: Transport::Codex {
            effort: Effort::Low,
        },
    });
    let context = json!({"goal": "Book a table for two in Zurich on Friday",
        "field": {"label": "City", "role": "textbox", "value": ""}});
    for _ in 0..3 {
        match helper.resolve(&context).await {
            Ok(written) => eprintln!(
                "{} typed {:?} in {} ms, usage {}",
                written.model, written.value, written.latency_ms, written.usage
            ),
            Err(error) => panic!("{error:#}"),
        }
    }
}

/// A default choice whose sign-in cannot produce a token types through the
/// model behind it, says so, and stays on it for the rest of the run.
#[tokio::test]
async fn a_default_choice_falls_back_when_the_sign_in_is_unusable() {
    let context = json!({"goal": "Fly to Oslo", "field": {"label": "City"}});
    for auth in [
        Arc::new(BrokenAuth) as Arc<dyn CodexAuth>,
        FakeAuth::new(&[None]),
    ] {
        let backend = MockServer::start(vec![streamed(&value(Some("never")))]).await;
        let chat = MockServer::start(vec![chat_reply("Oslo")]).await;
        let helper = codex_with(&backend, auth, Effort::Low, Some(chat_fallback(&chat)));

        let written = helper.resolve(&context).await.unwrap();
        assert_eq!(written.value, "Oslo");
        assert_eq!(written.model, "deepseek-chat");
        let current = helper.current();
        assert_eq!(current.model, "deepseek-chat");
        assert_eq!(current.source, "roder-provider");
        assert_eq!(current.note, Some(crate::text_model::codex_unusable()));
        // The next field goes straight to the stand-in.
        helper.resolve(&context).await.unwrap();
        assert_eq!(backend.hits(), 0);
        assert_eq!(chat.hits(), 2);
        let shown = format!("{current:?} {}", current.label());
        for leak in ["rt-leak", "refresh_token", "sk-fallback", "400"] {
            assert!(!shown.contains(leak), "{leak} in {shown}");
        }
    }
}

#[tokio::test]
async fn a_default_choice_falls_back_when_the_same_token_is_refused() {
    let backend = MockServer::start(vec![Reply::status(401)]).await;
    let chat = MockServer::start(vec![chat_reply("Oslo")]).await;
    let helper = codex_with(
        &backend,
        FakeAuth::new(&[Some("tok-stale")]),
        Effort::Low,
        Some(chat_fallback(&chat)),
    );
    let written = helper
        .resolve(&json!({"goal": "Oslo", "field": {"label": "City"}}))
        .await
        .unwrap();
    assert_eq!(written.model, "deepseek-chat");
    assert_eq!(backend.hits(), 1);
    assert_eq!(
        helper.current().note,
        Some(crate::text_model::codex_unusable())
    );
}

/// Only an unusable sign-in falls back: a backend rejection or an explicit
/// choice ends the run as before.
#[tokio::test]
async fn other_failures_and_explicit_choices_do_not_fall_back() {
    let chat = MockServer::start(vec![chat_reply("Oslo")]).await;
    let rejected = MockServer::start(vec![Reply::status(400)]).await;
    let helper = codex_with(
        &rejected,
        FakeAuth::new(&[Some("t")]),
        Effort::Low,
        Some(chat_fallback(&chat)),
    );
    assert!(helper.resolve(&json!({"goal": "g"})).await.is_err());
    assert_eq!(helper.current().model, "gpt-6-sol");

    let explicit = codex(&rejected, FakeAuth::new(&[None]), Effort::Low);
    let error = explicit.resolve(&json!({"goal": "g"})).await.unwrap_err();
    assert_eq!(JevStop::status_of(&error), JevStatus::Error);
    assert!(error.to_string().contains("no Codex sign-in"), "{error}");
    assert_eq!(chat.hits(), 0);
}
