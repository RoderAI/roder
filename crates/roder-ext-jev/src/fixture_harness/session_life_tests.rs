//! A session over time on a test Chrome: a background tab, separate
//! threads, calls that overlap, idle expiry,
//! eviction, a dead process's ledger, secrets kept across calls, the cost of
//! going back to a tab, and a continued call's decision request.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::scripted::PlanDecider;
use super::sessions::{call, targets, test_sessions};
use crate::cdp::Connection;
use crate::engine::{JevDecision, JevDecisionClient, JevEngine, JevEngineConfig};
use crate::page::{OwnedTab, Page};
use crate::secret::Secrets;
use crate::session::{JevSessions, SessionLimits};

fn done() -> Arc<dyn JevDecisionClient> {
    Arc::new(PlanDecider::new(Vec::new()))
}

/// Answers DONE after holding every decision back, and keeps what each
/// decision was shown.
struct Recorder {
    delay: Duration,
    seen: Mutex<Vec<(Value, String, usize)>>,
}

impl Recorder {
    fn new(delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            delay,
            seen: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl JevDecisionClient for Recorder {
    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        tokio::time::sleep(self.delay).await;
        self.seen
            .lock()
            .unwrap()
            .push((observation.clone(), goal.to_string(), history.len()));
        PlanDecider::new(Vec::new())
            .choose(observation, goal, history)
            .await
    }
}

#[tokio::test]
async fn concurrent_calls_serialize_and_busy_at_the_deadline() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let url = harness.site.url("basic.html");
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": url}),
        done(),
    )
    .await
    .unwrap();
    let before = harness.page_targets().await;
    let slow = Recorder::new(Duration::from_millis(1500));
    let (first, second) = tokio::join!(
        call(
            &harness,
            &sessions,
            "t",
            json!({"goal": "Read slowly", "url": ""}),
            slow.clone()
        ),
        async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            call(
                &harness,
                &sessions,
                "t",
                json!({"goal": "Read", "url": ""}),
                done(),
            )
            .await
        }
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(first["status"], "done");
    assert_eq!(second["status"], "done", "{second:#}");
    assert!(
        second["session"]["waited_ms"].as_u64().unwrap() >= 500,
        "{second:#}"
    );
    assert!(
        crate::report::digest(&second, "Mon 2026-09-28, 17:42 local time (UTC-07:00)")
            .contains("Waited")
    );
    assert_eq!(second["session"]["call"], 3);
    assert_eq!(harness.page_targets().await, before);

    // A call whose deadline passes while another holds the tab does nothing.
    let (_, busy) = tokio::join!(
        call(
            &harness,
            &sessions,
            "t",
            json!({"goal": "Read slowly", "url": ""}),
            slow.clone()
        ),
        async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            call(
                &harness,
                &sessions,
                "t",
                json!({"goal": "Read", "url": "", "timeout_seconds": 1}),
                done(),
            )
            .await
        }
    );
    let busy = busy.unwrap();
    assert_eq!(busy["status"], "busy", "{busy:#}");
    assert_eq!(busy["stopped_because"], crate::session::BUSY);
}

#[tokio::test]
async fn idle_session_expires() {
    let harness = harness_or_skip!();
    let sessions = JevSessions::new(SessionLimits {
        idle: Duration::from_secs(1),
        max_sessions: 8,
        max_tabs: 3,
    });
    let before = harness.page_targets().await;
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.url("basic.html")}),
        done(),
    )
    .await
    .unwrap();
    sessions.sweep().await;
    assert!(sessions.peek("t").is_some(), "not idle yet");
    tokio::time::sleep(Duration::from_millis(1100)).await;
    sessions.sweep().await;
    assert!(sessions.peek("t").is_none());
    assert_eq!(harness.settled_page_targets(before).await, before);
}

#[tokio::test]
async fn lru_evicts_oldest_idle() {
    let harness = harness_or_skip!();
    let sessions = JevSessions::new(SessionLimits {
        idle: Duration::from_secs(600),
        max_sessions: 2,
        max_tabs: 3,
    });
    let before = harness.page_targets().await;
    for thread in ["a", "b", "c"] {
        call(
            &harness,
            &sessions,
            thread,
            json!({"goal": "Read", "url": harness.site.url("basic.html")}),
            done(),
        )
        .await
        .unwrap();
    }
    assert!(sessions.peek("a").is_none());
    assert!(!targets(&sessions, "b").is_empty() && !targets(&sessions, "c").is_empty());
    assert_eq!(harness.settled_page_targets(before + 2).await, before + 2);
}

#[tokio::test]
async fn ledger_from_dead_process_is_swept() {
    let harness = harness_or_skip!();
    let dir = std::env::temp_dir().join(format!(
        "roder-jev-dead-ledger-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let before = harness.page_targets().await;
    // A tab a process that died left behind, and its stale ledger.
    let left = harness.open("basic.html").await.unwrap();
    let dead = dir.join("1-dead.json");
    std::fs::write(
        &dead,
        json!({"updated_at": 1, "browsers": [
            {"endpoint": harness.endpoint(), "targets": [left.target_id()]}
        ]})
        .to_string(),
    )
    .unwrap();
    assert_eq!(harness.settled_page_targets(before + 1).await, before + 1);

    let sessions = test_sessions().with_ledger(dir.clone());
    // A live session of this process is written down, not swept.
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.url("landing.html")}),
        done(),
    )
    .await
    .unwrap();
    sessions.sweep().await;
    assert!(!dead.exists());
    assert_eq!(harness.settled_page_targets(before + 1).await, before + 1);
    let ours = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(ours.len(), 1);
    let written = std::fs::read_to_string(ours[0].path()).unwrap();
    assert!(written.contains(&targets(&sessions, "t")[0]), "{written}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A secret an earlier run typed is still scrubbed from a later run's
/// page reads, given to it with `with_secrets`.
#[tokio::test]
async fn secret_typed_in_call1_scrubbed_in_call2() {
    let harness = harness_or_skip!();
    let page = harness.open("basic.html").await.unwrap();
    let target = page.target_id().to_string();
    let mut earlier = Secrets::default();
    // Text basic.html shows: as if the page echoed a password typed before.
    earlier.remember("Read the docs");
    let config = JevEngineConfig::new("Read", done())
        .with_cookie_banner_refusal(false)
        .with_secrets(earlier.clone());
    let mut engine = JevEngine::start(Box::new(page), config).await.unwrap();
    let result = engine.run(Duration::from_secs(20)).await;
    drop(engine);
    harness.close_target(&target).await.ok();
    assert!(
        !result.visible_text.contains("Read the docs"),
        "{}",
        result.visible_text
    );
    assert!(result.visible_text.contains("[secret]"));
    // And the run hands the list on.
    assert_eq!(result.typed_secrets, earlier);
}

/// Resuming is measured at about 100 ms on an idle machine, well under the
/// 300 ms the design asks for. The suite starts a headless Chrome per test
/// in parallel, so the bound asserted here is a generous ceiling that only
/// a pathological regression (a reload, a fixed sleep) crosses; the timing
/// itself is printed.
#[tokio::test]
async fn resume_is_fast() {
    let harness = harness_or_skip!();
    let page = harness.open_refusing("basic.html").await.unwrap();
    let owned = [OwnedTab {
        target: page.target_id().to_string(),
        opener: None,
    }];
    drop(page);
    let mut fastest = Duration::MAX;
    for _ in 0..3 {
        let connection = Connection::connect(harness.endpoint()).await.unwrap();
        let started = Instant::now();
        let (mut resumed, gone) = Page::resume(connection, &owned, true, true).await.unwrap();
        fastest = fastest.min(started.elapsed());
        assert!(gone.is_empty());
        // Set up again: the emulated viewport is back.
        let width = resumed.evaluate("window.innerWidth").await.unwrap();
        assert_eq!(width, json!(1120));
    }
    eprintln!("resume took {fastest:?} at best");
    assert!(fastest < Duration::from_secs(2), "{fastest:?}");
    harness.close_target(&owned[0].target).await.ok();
}

/// The first decision of a continued call is asked exactly as a fresh
/// call's: nothing from earlier calls is added to the request.
#[tokio::test]
async fn continued_call_request_matches_fresh_shape() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let url = harness.site.url("basic.html");
    let fresh = Recorder::new(Duration::ZERO);
    call(
        &harness,
        &sessions,
        "fresh",
        json!({"goal": "Read the page", "url": url}),
        fresh.clone(),
    )
    .await
    .unwrap();
    let continued = Recorder::new(Duration::ZERO);
    call(
        &harness,
        &sessions,
        "fresh",
        json!({"goal": "Read the page", "url": ""}),
        continued.clone(),
    )
    .await
    .unwrap();
    let (fresh, continued) = (fresh.seen.lock().unwrap(), continued.seen.lock().unwrap());
    let today = chrono::Local::now().date_naive();
    let body = |(observation, goal, history): &(Value, String, usize)| {
        assert_eq!(*history, 0);
        crate::decide::request_body(observation, goal, &[], "jev-latest", today).0
    };
    assert_eq!(body(&continued[0]), body(&fresh[0]));
}

#[tokio::test]
async fn background_tab_survives_call() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    let hidden = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.url("basic.html"), "foreground": false}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(hidden["browser"]["foreground"], false);
    // Used to be closed at the end of a background call.
    assert_eq!(harness.settled_page_targets(before + 1).await, before + 1);
    let next = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": "", "foreground": false}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(next["session"]["tab_note"], "continued");
    assert_eq!(harness.page_targets().await, before + 1);
}

#[tokio::test]
async fn threads_have_their_own_tabs() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    for thread in ["a", "b"] {
        call(
            &harness,
            &sessions,
            thread,
            json!({"goal": "Read", "url": harness.site.url("basic.html")}),
            done(),
        )
        .await
        .unwrap();
    }
    assert_ne!(targets(&sessions, "a"), targets(&sessions, "b"));
    assert_eq!(harness.settled_page_targets(before + 2).await, before + 2);
}
