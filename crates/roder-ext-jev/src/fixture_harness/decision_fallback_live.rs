//! The fallback after an exhausted decision error, with the real fallback
//! model (`JEV_EVAL_FALLBACK=model`: `gpt-6-sol` at low effort): the session
//! call `jev_browse` makes, the real hosted decision client and transport
//! at a local server, the `contact_form` task graded by the corpus's grader.
//!
//! - HTTP 200 with a body that cannot be decoded, every time: Jev ends
//!   `error` / `decision_unusable` and the call falls back to the model.
//! - HTTP 401: Jev ends `error`, nothing falls back, the model is not asked.
//!
//! POSTs, derived from the code: an undecodable body is `InvalidBody`, sent
//! again `RetryPolicy::resend_limit` (1) times, so a decision costs 2; the
//! loop asks again at most twice (`agent::unusable::ASKS_AGAIN`), so 3
//! decisions and 3 x 2 = 6 POSTs. A 401 is `Retry::Never` and no reply, so 1.
//!
//! The live test is `#[ignore]`d; the keyless ones check the counts on the
//! transport alone and run both scenarios with a scripted fallback.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::Harness;
use super::evals::{Expect, Outcome, Task, fallback_live, load_tasks, write_json};
use super::fallback_script::ScriptedFallback;
use super::sessions::{call_falling_back, test_sessions};
use super::site::Post;
use crate::decide::{JevTypeSafeDecisionClient, TypeSafeHttpTransport};
use crate::engine::{JevDecisionClient, JevDecisionTransport, JevRunResult, JevStatus};
use crate::fallback::model::{FallbackModel, Reply, Turn};
use crate::http::RetryPolicy;
use crate::http::tests::{MockServer, Reply as Answer};
use crate::runner::Ceilings;

const TASK: &str = "contact_form";
/// Decisions asked before giving up: the first plus the loop's private
/// `agent::unusable::ASKS_AGAIN` (2); the counting test below catches drift.
const UNUSABLE_REPLIES: usize = 3;

/// POSTs one decision costs when the body cannot be decoded: the request and
/// the transport's resends of it.
fn posts_per_unusable_decision() -> usize {
    1 + RetryPolicy::default().resend_limit
}

/// A fallback model that counts the replies it is asked for.
struct Counted {
    inner: Arc<dyn FallbackModel>,
    replies: AtomicUsize,
}

impl Counted {
    fn new(inner: Arc<dyn FallbackModel>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            replies: AtomicUsize::new(0),
        })
    }

    fn replies(&self) -> usize {
        self.replies.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl FallbackModel for Counted {
    fn label(&self) -> String {
        self.inner.label()
    }

    fn sees_tool_result_images(&self) -> bool {
        self.inner.sees_tool_result_images()
    }

    async fn reply(&self, turn: Turn<'_>) -> anyhow::Result<Reply> {
        self.replies.fetch_add(1, Ordering::SeqCst);
        self.inner.reply(turn).await
    }
}

/// The hosted transport on its real retry policy, at the local server.
fn transport(server: &MockServer) -> TypeSafeHttpTransport {
    let url = server.url.clone();
    TypeSafeHttpTransport::new(url, "mock-decision-key", RetryPolicy::default())
}

fn decision_service(server: &MockServer) -> Arc<dyn JevDecisionClient> {
    let client =
        JevTypeSafeDecisionClient::with_transport("jev-latest", Arc::new(transport(server)));
    Arc::new(client)
}

/// A status as a call's result names it.
fn status_of(name: &str) -> JevStatus {
    use JevStatus::*;
    [
        Done,
        Blocked,
        BudgetExceeded,
        TimedOut,
        NeedsInput,
        Unavailable,
        Error,
    ]
    .into_iter()
    .chain([NeedsConfirmation, AccessDenied])
    .find(|status| serde_json::to_value(status).is_ok_and(|value| value == name))
    .unwrap_or_else(|| panic!("a call ended with status {name:?}"))
}

/// The call as the corpus grader reads a run: the end state the call
/// reports, and the POSTs the fixture site received.
fn call_outcome(call: &Value, posts: Vec<Post>) -> Outcome {
    let text = |key: &str| call[key].as_str().unwrap_or_default().to_string();
    let mut result = JevRunResult::before_start(
        status_of(call["status"].as_str().unwrap_or_default()),
        &text("url"),
        Duration::ZERO,
        "",
    );
    result.title = text("title");
    result.visible_text = text("visible_text");
    result.stopped_because = call["stopped_because"].as_str().map(str::to_string);
    Outcome {
        result,
        probed: Default::default(),
        posts,
        wall_ms: 0,
        watch: Default::default(),
        after: None,
    }
}

/// What one scenario found: counts and timings for the row, and every way
/// the call missed what it should be.
struct Found {
    row: Value,
    misses: Vec<String>,
}

impl Found {
    fn new(scenario: &str, run: usize) -> Self {
        Self {
            row: json!({"scenario": scenario, "run": run}),
            misses: Vec::new(),
        }
    }

    fn check(&mut self, ok: bool, miss: impl FnOnce() -> String) {
        if !ok {
            self.misses.push(miss());
        }
    }
}

/// One call of `task` on a fresh site, against a decision service that
/// always gives `answer`, with `model` as its fallback.
struct Driven {
    harness: Harness,
    server: MockServer,
    sessions: crate::session::JevSessions,
    thread: String,
    wall_ms: u64,
    found: Found,
    call: Option<Value>,
}

impl Driven {
    async fn new(
        base: &Harness,
        task: &Task,
        (scenario, run): (&str, usize),
        answer: Answer,
        model: Arc<Counted>,
    ) -> Self {
        let harness = base.with_new_site().await;
        let server = MockServer::start(vec![answer]).await;
        let sessions = test_sessions();
        let thread = format!("decision-fallback-live-{scenario}-{run}");
        let args = json!({
            "goal": task.goal,
            "url": harness.site.url(&task.page),
            "timeout_seconds": 180,
        });
        let started = Instant::now();
        let call = call_falling_back(
            &harness,
            &sessions,
            &thread,
            args,
            decision_service(&server),
            model,
            &Ceilings::default(),
        )
        .await;
        let wall_ms = started.elapsed().as_millis() as u64;
        let mut found = Found::new(scenario, run);
        let call = call
            .map_err(|error| found.misses.push(format!("the call failed: {error:#}")))
            .ok();
        Self {
            harness,
            server,
            sessions,
            thread,
            wall_ms,
            found,
            call,
        }
    }

    async fn close(self) -> Found {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        self.sessions.close(&self.thread, deadline).await.ok();
        self.found
    }
}

/// The decision service answers every request with HTTP 200 and a body that
/// is not JSON; the call must end `error` / `decision_unusable`, fall back to
/// `model` for `decision_unusable`, and the task must come out right.
async fn unusable_run(base: &Harness, task: &Task, model: Arc<Counted>, run: usize) -> Found {
    let answer = Answer::ok("this body is not json");
    let mut run = Driven::new(base, task, ("unusable", run), answer, model.clone()).await;
    let Some(call) = run.call.take() else {
        return run.close().await;
    };
    let expected = UNUSABLE_REPLIES * posts_per_unusable_decision();
    let (jev, fallback) = (&call["drivers"][0], &call["fallback"]);
    let wanted_posts = task.expect.posts.as_ref().map_or(0, Vec::len);
    let posts = run
        .harness
        .site
        .wait_for_posts(wanted_posts, Duration::from_secs(2))
        .await;
    // A task written for Jev alone is graded on its outcome after the
    // fallback, as the corpus grades a fallback row.
    let expect = Expect {
        stopped: None,
        stopped_because_contains: None,
        ..task.expect.clone()
    };
    let graded = expect.grade(&call_outcome(&call, posts));
    let marks = graded.marks();
    let actions = fallback["actions"].as_array().cloned().unwrap_or_default();
    let hits = run.server.hits();
    let first_is_a_decision = run
        .server
        .requests()
        .first()
        .is_some_and(|(_, body)| body["questions"]["operation"].is_object());
    let replies = model.replies();
    let fallback_calls = &call["drivers"][1]["model_calls"];
    let found = &mut run.found;
    let (ok, kind) = (call["jev_status"] == "error", &call["jev_status"]);
    found.check(ok, || format!("jev_status {kind} != \"error\""));
    let cause = &call["stop_cause"];
    found.check(*cause == "decision_unusable", || {
        format!("stop_cause {cause}")
    });
    found.check(hits == expected, || {
        format!("{hits} decision POSTs, expected {expected}")
    });
    found.check(first_is_a_decision, || {
        "the first POST is no decision".into()
    });
    found.check(fallback["ran"] == true, || {
        format!("ran {}", fallback["ran"])
    });
    found.check(fallback["trigger"]["kind"] == "decision_unusable", || {
        format!("trigger {}", fallback["trigger"])
    });
    found.check(jev["decisions"] == UNUSABLE_REPLIES, || {
        format!("Jev's decisions {} != {UNUSABLE_REPLIES}", jev["decisions"])
    });
    found.check(jev["actions"] == 0, || {
        format!("Jev acted {} times without a decision", jev["actions"])
    });
    found.check(replies > 0 && *fallback_calls == replies, || {
        format!("fallback model_calls {fallback_calls} vs {replies} replies asked")
    });
    let failures = graded.failures();
    found.check(marks.verdict_ok, || {
        format!("verdict: {}", failures.join("; "))
    });
    found.check(marks.truth_ok, || format!("truth: {}", failures.join("; ")));
    found.row = json!({
        "scenario": "unusable_replies",
        "run": found.row["run"],
        "jev_status": call["jev_status"],
        "stop_cause": call["stop_cause"],
        "mock_requests": hits,
        "expected_requests": expected,
        "jev_decisions": jev["decisions"],
        "jev_actions": jev["actions"],
        "fallback_ran": fallback["ran"],
        "trigger": fallback["trigger"]["kind"],
        "fallback_model": fallback["model"],
        "call_status": call["status"],
        "fallback_tool_calls": actions.len(),
        "fallback_tools": actions.iter().map(|action| action["tool"].clone()).collect::<Vec<_>>(),
        "fallback_tool_errors": actions.iter().filter(|action| action["error"] == true).count(),
        "fallback_model_calls": fallback_calls,
        "model_replies_asked": replies,
        "fallback_input_tokens": fallback["usage"]["input_tokens"],
        "fallback_output_tokens": fallback["usage"]["output_tokens"],
        "fallback_elapsed_ms": fallback["elapsed_ms"],
        "wall_ms": run.wall_ms,
        "verdict_ok": marks.verdict_ok,
        "truth_ok": marks.truth_ok,
        "false_green": marks.false_green,
        "misses": failures.len(),
    });
    run.close().await
}

/// The decision service refuses the key with HTTP 401: the call ends
/// `error`, nothing falls back, `model` is never asked, and the form is not
/// submitted.
async fn refused_run(base: &Harness, task: &Task, model: Arc<Counted>) -> Found {
    let mut run = Driven::new(
        base,
        task,
        ("refused", 1),
        Answer::status(401),
        model.clone(),
    )
    .await;
    let Some(call) = run.call.take() else {
        return run.close().await;
    };
    let posted = run.harness.site.wait_for_posts(0, Duration::ZERO).await;
    let (hits, replies) = (run.server.hits(), model.replies());
    let found = &mut run.found;
    found.check(call["status"] == "error", || {
        format!("status {}", call["status"])
    });
    found.check(call["stop_cause"] == "stopped", || {
        format!("stop_cause {}", call["stop_cause"])
    });
    let reason = call["stopped_because"].as_str().unwrap_or_default();
    found.check(reason.contains("HTTP 401"), || {
        format!("stopped_because {reason}")
    });
    found.check(call.get("fallback").is_none(), || {
        format!("a fallback was reported: {}", call["fallback"])
    });
    found.check(call.get("jev_status").is_none(), || {
        "a Jev status apart from the call's, as after a fallback".to_string()
    });
    found.check(replies == 0, || {
        format!("the model was asked {replies} times")
    });
    found.check(hits == 1, || format!("{hits} decision POSTs, expected 1"));
    found.check(posted.is_empty(), || format!("{} form posts", posted.len()));
    found.row = json!({
        "scenario": "refused_key_401",
        "run": 1,
        "status": call["status"],
        "stop_cause": call["stop_cause"],
        "mock_requests": hits,
        "expected_requests": 1,
        "fallback_reported": call.get("fallback").is_some(),
        "fallback_model_replies": replies,
        "form_posts": posted.len(),
        "wall_ms": run.wall_ms,
        "misses": found.misses.len(),
    });
    run.close().await
}

fn contact_form() -> Task {
    load_tasks()
        .unwrap()
        .into_iter()
        .find(|task| task.id == TASK)
        .unwrap_or_else(|| panic!("the corpus has no task {TASK} (JEV_EVAL_TASKS narrows it)"))
}

/// The live fallback model, or a clear failure: `live_fallback` answers only
/// to `JEV_EVAL_FALLBACK=model`, so this test sets it, and a run without the
/// model's credentials stops here rather than skipping.
async fn real_fallback() -> Arc<dyn FallbackModel> {
    let signed_in = matches!(roder_codex_auth::status().await, Ok(Some(_)));
    let keyed = std::env::var("OPENAI_API_KEY").is_ok_and(|key| !key.trim().is_empty());
    assert!(
        signed_in || keyed,
        "the live fallback needs the ChatGPT/Codex sign-in or OPENAI_API_KEY; neither is available"
    );
    // SAFETY: this test is the only one that runs under its filter, and it
    // sets the variable before it starts any task of its own.
    unsafe { std::env::set_var("JEV_EVAL_FALLBACK", "model") };
    fallback_live::live_fallback()
        .await
        .expect("JEV_EVAL_FALLBACK=model yields the live fallback model")
}

#[tokio::test]
#[ignore = "live: needs the fallback model (Codex sign-in or OPENAI_API_KEY), Chrome, and spends model calls"]
async fn decision_fallback_live_real_model() {
    let task = contact_form();
    let base = Harness::start()
        .await
        .expect("the live fallback test requires Chrome (set JEV_CHROME_BINARY)");
    let model = real_fallback().await;
    let label = model.label();
    eprintln!("fallback model: {label}");
    assert!(label.contains("gpt-6"), "the model is {label}"); // before any call is spent
    let mut found = Vec::new();
    for run in 1..=3 {
        let one = unusable_run(&base, &task, Counted::new(model.clone()), run).await;
        eprintln!("{}", one.row);
        found.push(one);
    }
    let refused = refused_run(&base, &task, Counted::new(model.clone())).await;
    eprintln!("{}", refused.row);
    found.push(refused);

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let summary = json!({
        "task": TASK,
        "fallback_model": label,
        "rows": found.iter().map(|one| one.row.clone()).collect::<Vec<_>>(),
        "misses": found.iter().map(|one| one.misses.len()).sum::<usize>(),
    });
    let path = write_json(&format!("decision-fallback-live-{stamp}"), &summary).unwrap();
    eprintln!("rows: {}", path.display());
    let misses = found
        .iter()
        .flat_map(|one| {
            let name = format!("{} #{}", one.row["scenario"], one.row["run"]);
            one.misses.iter().map(move |miss| format!("{name}: {miss}"))
        })
        .collect::<Vec<_>>();
    assert!(
        misses.is_empty(),
        "the live fallback did not hold:\n{}",
        misses.join("\n")
    );
}

/// The numbers the scenarios derive hold on the transport alone: a body that
/// cannot be decoded is sent twice and ends as an unusable reply, a 401 is
/// sent once and ends the run's decision for good.
#[tokio::test]
async fn the_expected_request_counts_follow_the_transport() {
    let request = json!({"questions": {"operation": {}}});

    let garbled = MockServer::start(vec![Answer::ok("this body is not json")]).await;
    let error = transport(&garbled).decide(&request).await.unwrap_err();
    assert_eq!(garbled.hits(), posts_per_unusable_decision());
    assert!(
        crate::usage::UnusableAnswer::is_behind(&error),
        "an undecodable body is a reply the loop asks again about: {error:#}"
    );

    let refused = MockServer::start(vec![Answer::status(401)]).await;
    let error = transport(&refused).decide(&request).await.unwrap_err();
    assert_eq!(refused.hits(), 1);
    assert!(
        !crate::usage::UnusableAnswer::is_behind(&error),
        "a refused key is no reply: {error:#}"
    );
    assert!(error.to_string().contains("HTTP 401"), "{error:#}");
}

/// The same scenarios and checks as the live test, with a scripted fallback
/// that fills the contact form, so the whole path (mock service, session
/// call, fallback, corpus grader, request counts) runs without a key.
#[tokio::test]
async fn the_scenarios_hold_with_a_scripted_fallback() {
    let harness = harness_or_skip!();
    let task = contact_form();
    let mut plan = task
        .values
        .iter()
        .map(|(label, value)| json!({"tool": "type", "args": {"ref_of": label, "text": value}}))
        .collect::<Vec<_>>();
    plan.push(json!({"tool": "click", "args": {"ref_of": "Send message"}}));
    plan.push(json!({"say": "DONE: the message was sent"}));
    let scripted: Arc<dyn FallbackModel> = Arc::new(ScriptedFallback::from_json(json!(plan)));

    let model = Counted::new(scripted.clone());
    let found = unusable_run(&harness, &task, model.clone(), 1).await;
    assert!(found.misses.is_empty(), "{:?}\n{}", found.misses, found.row);
    assert_eq!(found.row["mock_requests"], 6, "{}", found.row);
    assert_eq!(found.row["fallback_tool_calls"], plan.len() - 1);
    assert_eq!(found.row["truth_ok"], true, "{}", found.row);

    let none = Counted::new(scripted);
    let refused = refused_run(&harness, &task, none.clone()).await;
    assert!(refused.misses.is_empty(), "{:?}", refused.misses);
    assert_eq!(none.replies(), 0);
    assert_eq!(refused.row["fallback_reported"], false);
}
