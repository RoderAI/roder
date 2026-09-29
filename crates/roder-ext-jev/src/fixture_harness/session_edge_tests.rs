//! What a session must not do to the user's tabs and data between calls:
//! adopt or close tabs the user opened from Jev's tab, load over a kept tab
//! when a refused one is gone, reopen a page outside the operator's
//! origins, or report a typed secret back through the address it compares.

use std::sync::Arc;

use serde_json::{Value, json};

use super::Harness;
use super::scripted::{FieldValues, PlanDecider, pick};
use super::sessions::{call, call_under, call_with, targets, test_sessions};
use crate::engine::JevDecisionClient;
use crate::runner::Ceilings;
use crate::scope::JevOriginScope;

fn done() -> Arc<dyn JevDecisionClient> {
    Arc::new(PlanDecider::new(Vec::new()))
}

fn clicks(label: &'static str) -> Arc<dyn JevDecisionClient> {
    Arc::new(PlanDecider::new(vec![pick("click", label)]))
}

/// Run `expression` in `target` as the user would, with a user gesture so
/// the page may open tabs.
async fn as_user(harness: &Harness, target: &str, expression: &str) {
    let mut connection = harness.connect().await.unwrap();
    let attached = connection
        .call(
            "Target.attachToTarget",
            json!({"targetId": target, "flatten": true}),
            None,
        )
        .await
        .unwrap();
    let session = attached["sessionId"].as_str().unwrap().to_string();
    connection
        .call(
            "Runtime.evaluate",
            json!({"expression": expression, "userGesture": true}),
            Some(&session),
        )
        .await
        .unwrap();
}

/// The page targets whose opener is `opener`.
async fn opened_by(harness: &Harness, opener: &str) -> Vec<String> {
    let mut connection = harness.connect().await.unwrap();
    let targets = connection
        .call("Target.getTargets", json!({}), None)
        .await
        .unwrap();
    targets["targetInfos"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|target| target["type"] == "page" && target["openerId"] == opener)
        .filter_map(|target| target["targetId"].as_str().map(str::to_string))
        .collect()
}

async fn url_of(harness: &Harness, target: &str) -> String {
    let mut connection = harness.connect().await.unwrap();
    let info = connection
        .call("Target.getTargetInfo", json!({"targetId": target}), None)
        .await
        .unwrap();
    info["targetInfo"]["url"].as_str().unwrap().to_string()
}

/// Tabs the user opened from Jev's tab between two calls are the user's:
/// the next call neither moves into the newest nor closes the others.
#[tokio::test]
async fn tabs_the_user_opened_between_calls_are_left_alone() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read the inbox", "url": harness.site.url("between-calls.html")}),
        done(),
    )
    .await
    .unwrap();
    let tab = targets(&sessions, "t");
    // Each click its own gesture, as the user makes them.
    as_user(&harness, &tab[0], "document.getElementById('q3').click()").await;
    as_user(
        &harness,
        &tab[0],
        "document.getElementById('annual').click()",
    )
    .await;
    assert_eq!(harness.settled_page_targets(before + 3).await, before + 3);
    let users = opened_by(&harness, &tab[0]).await;
    assert_eq!(users.len(), 2);

    let next = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Mark it read", "url": ""}),
        clicks("Mark read"),
    )
    .await
    .unwrap();
    assert_eq!(next["status"], "done", "{next:#}");
    assert_eq!(next["session"]["tab"], "t1", "{next:#}");
    assert_eq!(next["title"], "Inbox", "{next:#}");
    assert!(
        next["visible_text"]
            .as_str()
            .unwrap()
            .contains("Marked read."),
        "{next:#}"
    );
    assert_eq!(next["actions"][0]["opened_tab"], Value::Null, "{next:#}");
    assert_eq!(targets(&sessions, "t"), tab);
    // Both of the user's tabs are still open.
    assert_eq!(harness.settled_page_targets(before + 3).await, before + 3);
    for user in &users {
        harness.close_target(user).await.unwrap();
    }
    // A tab Jev's own click opens in a later call is still followed.
    let opened = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Open the Q3 report", "url": ""}),
        clicks("Open the Q3 report"),
    )
    .await
    .unwrap();
    assert_eq!(opened["title"], "Report Q3", "{opened:#}");
    assert_eq!(opened["session"]["tab"], "t2", "{opened:#}");
}

/// A new tab asked for after a refused page goes to the refused page's tab,
/// but when the user has closed that tab the call opens the new tab it
/// asked for, rather than loading over the page the session kept before.
#[tokio::test]
async fn a_closed_refused_tab_does_not_send_the_url_over_the_kept_one() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    let kept = harness.site.url("reserve.html");
    call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": kept}),
        done(),
    )
    .await
    .unwrap();
    let refused = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Find a table", "url": harness.site.url("access-denied.html?status=403"),
               "tab": "new"}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(refused["status"], "access_denied", "{refused:#}");
    let owned = targets(&sessions, "t");
    assert_eq!(owned.len(), 2);
    harness.close_target(&owned[1]).await.unwrap();
    assert_eq!(harness.settled_page_targets(before + 1).await, before + 1);

    let next = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": harness.site.url("basic.html"), "tab": "new"}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(next["status"], "done", "{next:#}");
    assert_eq!(next["session"]["tab_note"], "new", "{next:#}");
    assert_eq!(next["session"]["tab"], "t3", "{next:#}");
    assert_eq!(next["session"]["tabs_open"], 2, "{next:#}");
    // The first tab still shows its page.
    assert_eq!(url_of(&harness, &owned[0]).await, kept);
    assert_eq!(targets(&sessions, "t")[0], owned[0]);
    assert_eq!(harness.settled_page_targets(before + 2).await, before + 2);
}

/// With the operator's origins set, a call that finds its tabs gone does
/// not reopen a page outside them, even the one the last call ended on.
#[tokio::test]
async fn a_reopen_never_loads_a_page_outside_the_operators_origins() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    let ceilings = Ceilings {
        scope: JevOriginScope::any()
            .narrow(&[harness.site.origin()])
            .unwrap(),
        refuse_cookie_banners: false,
        ..Ceilings::default()
    };
    let left = call_under(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Visit the partner catalogue", "url": harness.site.url("offsite.html")}),
        clicks("Visit the partner catalogue"),
        None,
        &ceilings,
    )
    .await
    .unwrap();
    assert_eq!(left["status"], "blocked", "{left:#}");
    let outside = left["url"].as_str().unwrap().to_string();
    assert!(outside.starts_with("http://localhost:"), "{left:#}");
    let tab = targets(&sessions, "t");
    harness.close_target(&tab[0]).await.unwrap();
    assert_eq!(harness.settled_page_targets(before).await, before);

    let next = call_under(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": ""}),
        done(),
        None,
        &ceilings,
    )
    .await
    .unwrap();
    assert_eq!(next["status"], "blocked", "{next:#}");
    assert!(
        next["stopped_because"]
            .as_str()
            .unwrap()
            .contains("outside the allowed origins"),
        "{next:#}"
    );
    assert_eq!(next["actions"], json!([]), "{next:#}");
    // No tab was opened to load it.
    assert_eq!(harness.page_targets().await, before);
}

/// A password typed into a form that submits by GET lands in the address.
/// The session records that address scrubbed, and the next call compares
/// the live one scrubbed too: it is neither reported as a move nor shown.
#[tokio::test]
async fn a_secret_in_the_address_stays_out_of_the_next_result() {
    const PASSWORD: &str = "hunter2-7431";
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let signed = call_with(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Sign in", "url": harness.site.url("login-get.html")}),
        Arc::new(PlanDecider::new(vec![
            pick("fill", "Password"),
            pick("click", "Sign in"),
        ])),
        Some(Arc::new(FieldValues::new(&[("Password", PASSWORD)]))),
    )
    .await
    .unwrap();
    let url = signed["url"].as_str().unwrap();
    assert!(url.contains("password=[secret]"), "{signed:#}");

    let mut next = call(
        &harness,
        &sessions,
        "t",
        json!({"goal": "Read", "url": ""}),
        done(),
    )
    .await
    .unwrap();
    assert_eq!(next["session"]["tab_note"], "continued", "{next:#}");
    assert_eq!(next["session"]["moved_to"], Value::Null, "{next:#}");
    let text = crate::report::tool_text(&mut next);
    assert!(!text.contains(PASSWORD), "{text}");
    assert!(!next.to_string().contains(PASSWORD), "{next:#}");
}
