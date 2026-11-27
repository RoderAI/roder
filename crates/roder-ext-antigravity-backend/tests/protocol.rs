#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use roder_api::backend::{AgentBackend, AgentBackendSession, BackendEvent, BackendStart};
use roder_api::policy_mode::PolicyMode;
use roder_ext_antigravity_backend::AntigravityBackend;
use serde_json::Value;

struct Fixture(PathBuf);

impl Fixture {
    fn new(scenario: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "roder-antigravity-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("agy"), include_str!("mock_acp.py")).unwrap();
        std::fs::set_permissions(path.join("agy"), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(path.join("scenario"), scenario).unwrap();
        Self(path)
    }
    fn backend(&self) -> AntigravityBackend {
        AntigravityBackend::with_program(self.0.join("agy"))
    }
    fn start(&self) -> BackendStart {
        BackendStart {
            cwd: self.0.clone(),
            model: None,
            resume_thread: None,
            policy_mode: PolicyMode::Default,
        }
    }
    fn trace(&self) -> Vec<Value> {
        std::fs::read_to_string(self.0.join("trace"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn argv(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.0.join("argv"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn finish(session: &mut dyn AgentBackendSession) -> (String, String) {
    let mut text = String::new();
    loop {
        match session.next_event().await.unwrap() {
            BackendEvent::Text(chunk) => text.push_str(&chunk),
            BackendEvent::Usage(usage) => {
                assert_eq!(usage.prompt_tokens, 120);
                assert_eq!(usage.completion_tokens, 30);
                assert_eq!(usage.total_tokens, 150);
                assert_eq!(usage.cached_prompt_tokens, 20);
            }
            BackendEvent::TurnFinished { status, .. } => return (text, status),
            _ => {}
        }
    }
}

#[tokio::test]
async fn discovers_models_and_switches_config_before_streaming() {
    let fixture = Fixture::new("");
    let models = fixture
        .backend()
        .list_models(fixture.0.clone())
        .await
        .unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "gemini-one");
    assert!(models[0].is_default);
    assert_eq!(models[0].reasoning_efforts, ["low", "high"]);
    assert!(models[1].reasoning_efforts.is_empty());
    let mut session = fixture.backend().connect(fixture.start()).await.unwrap();
    session
        .send(
            "hello".into(),
            None,
            Some("gemini-two".into()),
            Some("high".into()),
        )
        .await
        .unwrap();
    assert_eq!(session.model(), "gemini-two");
    assert_eq!(session.reasoning(), "high");
    assert_eq!(
        finish(session.as_mut()).await,
        ("hello subscription".into(), "completed".into())
    );
    assert_eq!(session.snapshot().turns[0].items.len(), 2);
    let trace = fixture.trace();
    assert!(
        trace
            .iter()
            .any(|message| message["method"] == "authenticate"
                && message["params"]["methodId"] == "oauth-personal")
    );
    assert_eq!(
        trace
            .iter()
            .filter(|message| message["method"] == "session/set_config_option")
            .count(),
        2
    );
    session.archive().await.unwrap();
}

#[tokio::test]
async fn approvals_use_exact_server_options_and_session_scoped_ids() {
    let fixture = Fixture::new("");
    let mut one = fixture.backend().connect(fixture.start()).await.unwrap();
    let mut two = fixture.backend().connect(fixture.start()).await.unwrap();
    let mut keys = Vec::new();
    for (session, approved) in [(&mut one, true), (&mut two, false)] {
        session
            .send("approval".into(), None, None, None)
            .await
            .unwrap();
        assert!(matches!(
            session.next_event().await.unwrap(),
            BackendEvent::TurnStarted { .. }
        ));
        let BackendEvent::Approval { request_id, .. } = session.next_event().await.unwrap() else {
            panic!("expected approval")
        };
        keys.push(request_id.clone());
        session.approve(&request_id, approved).await.unwrap();
        assert_eq!(finish(session.as_mut()).await.1, "completed");
    }
    assert_ne!(keys[0], keys[1]);
    let trace = fixture.trace();
    let responses = trace
        .iter()
        .filter(|message| message["id"] == 99)
        .collect::<Vec<_>>();
    assert_eq!(responses[0]["result"]["outcome"]["optionId"], "yes-exact");
    assert_eq!(responses[1]["result"]["outcome"]["optionId"], "no-exact");
    one.archive().await.unwrap();
    two.archive().await.unwrap();
}

#[tokio::test]
async fn questions_are_not_auto_approved_even_in_bypass() {
    let fixture = Fixture::new("");
    let mut session = fixture.backend().connect(fixture.start()).await.unwrap();
    session
        .send("question".into(), Some(PolicyMode::Bypass), None, None)
        .await
        .unwrap();
    session.next_event().await.unwrap();
    let BackendEvent::Question {
        request_id,
        questions,
    } = session.next_event().await.unwrap()
    else {
        panic!("expected question")
    };
    assert_eq!(questions[0].options, ["Yes", "No"]);
    session
        .answer(&request_id, vec![(questions[0].id.clone(), "No".into())])
        .await
        .unwrap();
    assert_eq!(finish(session.as_mut()).await.1, "completed");
    assert!(
        fixture
            .trace()
            .iter()
            .any(|message| message["result"]["outcome"]["optionId"] == "no-exact")
    );
}

#[tokio::test]
async fn cancellation_and_unsupported_requests_fail_closed() {
    let fixture = Fixture::new("");
    let mut session = fixture.backend().connect(fixture.start()).await.unwrap();
    session
        .send("cancel".into(), None, None, None)
        .await
        .unwrap();
    session.interrupt().await.unwrap();
    assert_eq!(finish(session.as_mut()).await.1, "interrupted");
    session
        .send("unsupported".into(), None, None, None)
        .await
        .unwrap();
    assert_eq!(finish(session.as_mut()).await.1, "completed");
    assert!(
        fixture
            .trace()
            .iter()
            .any(|message| message["id"] == 100 && message["error"]["code"] == -32601)
    );
}

#[tokio::test]
async fn authentication_and_session_failures_have_safe_login_guidance() {
    for scenario in ["auth-failure", "session-failure"] {
        let fixture = Fixture::new(scenario);
        let error = match fixture.backend().connect(fixture.start()).await {
            Ok(_) => panic!("expected authentication failure"),
            Err(error) => format!("{error:#}"),
        };
        assert!(error.contains("roder auth login antigravity"));
        assert!(!error.contains("secret-must-not-appear"));
        assert!(
            !fixture
                .trace()
                .iter()
                .any(|message| message["method"] == "session/prompt")
        );
    }
}

#[tokio::test]
async fn advertised_resume_replays_history_and_unsupported_plan_is_rejected() {
    let fixture = Fixture::new("");
    let mut start = fixture.start();
    start.resume_thread = Some("existing".into());
    let mut session = fixture.backend().connect(start).await.unwrap();
    assert_eq!(session.thread_id(), "existing");
    assert_eq!(session.snapshot().turns.len(), 1);
    assert_eq!(session.snapshot().turns[0].items.len(), 2);
    assert!(
        session
            .send("hello".into(), Some(PolicyMode::Plan), None, None)
            .await
            .is_err()
    );
    assert!(
        session
            .send("hello".into(), None, Some("nonexistent".into()), None)
            .await
            .is_err()
    );
    let fixture = Fixture::new("no-load");
    let mut start = fixture.start();
    start.resume_thread = Some("existing".into());
    assert!(fixture.backend().connect(start).await.is_err());
}

#[tokio::test]
async fn logout_requires_advertised_support_and_never_authenticates() {
    for (scenario, success) in [("", true), ("no-logout", false)] {
        let fixture = Fixture::new(scenario);
        assert_eq!(
            fixture.backend().logout(fixture.0.clone()).await.is_ok(),
            success
        );
        let trace = fixture.trace();
        assert!(
            !trace
                .iter()
                .any(|message| message["method"] == "authenticate")
        );
        assert_eq!(
            trace.iter().any(|message| message["method"] == "logout"),
            success
        );
    }
}

#[tokio::test]
async fn native_skip_permissions_is_only_enabled_for_bypass_at_startup() {
    for policy in [
        PolicyMode::Default,
        PolicyMode::AcceptAll,
        PolicyMode::Plan,
        PolicyMode::Bypass,
    ] {
        let fixture = Fixture::new("supports-plan");
        let mut start = fixture.start();
        start.policy_mode = policy;
        let mut session = fixture.backend().connect(start).await.unwrap();
        let expected = if policy == PolicyMode::Bypass {
            vec!["acp", "--dangerously-skip-permissions"]
        } else {
            vec!["acp"]
        };
        assert_eq!(fixture.argv(), vec![expected], "{policy:?}");
        session.archive().await.unwrap();
    }
}

#[tokio::test]
async fn native_bypass_cannot_be_tightened_without_a_new_process() {
    let fixture = Fixture::new("supports-plan");
    let mut start = fixture.start();
    start.policy_mode = PolicyMode::Bypass;
    let mut session = fixture.backend().connect(start).await.unwrap();
    for policy in [PolicyMode::Default, PolicyMode::AcceptAll, PolicyMode::Plan] {
        let error = session
            .send("must not run".into(), Some(policy), None, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("start a new session"));
    }
    assert!(
        !fixture
            .trace()
            .iter()
            .any(|message| message["method"] == "session/prompt")
    );
    // Rejected tightening must not poison the original Bypass session.
    session
        .send("hello".into(), None, None, None)
        .await
        .unwrap();
    assert_eq!(finish(session.as_mut()).await.1, "completed");
    session.archive().await.unwrap();
}

#[tokio::test]
async fn default_process_can_switch_to_host_bypass_and_back_without_native_flag() {
    let fixture = Fixture::new("supports-plan");
    let mut session = fixture.backend().connect(fixture.start()).await.unwrap();
    session
        .send("approval".into(), Some(PolicyMode::Bypass), None, None)
        .await
        .unwrap();
    assert_eq!(finish(session.as_mut()).await.1, "completed");
    assert!(
        fixture.trace().iter().any(|message| message["id"] == 99
            && message["result"]["outcome"]["optionId"] == "yes-exact")
    );
    session
        .send("hello".into(), Some(PolicyMode::Default), None, None)
        .await
        .unwrap();
    assert_eq!(finish(session.as_mut()).await.1, "completed");
    assert_eq!(fixture.argv(), vec![vec!["acp"]]);
    session.archive().await.unwrap();
}

#[tokio::test]
async fn account_probes_and_logout_never_skip_permissions() {
    let fixture = Fixture::new("");
    fixture
        .backend()
        .list_models(fixture.0.clone())
        .await
        .unwrap();
    fixture
        .backend()
        .list_threads(fixture.0.clone(), None)
        .await
        .unwrap();
    fixture.backend().logout(fixture.0.clone()).await.unwrap();
    assert_eq!(fixture.argv(), vec![vec!["acp"]; 3]);
}
