//! The fallback inherits Jev's rules and keeps to its own ceilings, on real
//! pages through the session layer: the irreversible-action gate, the
//! operator's allowed origins, an access block, the step, token and time
//! ceilings, which statuses fall back at all, and the hand-over.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::fallback_script::ScriptedFallback;
use super::fallback_tests::{ceilings, fall_back, jev};
use super::sessions::{call_falling_back, test_sessions};
use crate::fallback::FallbackMode;
use crate::fallback::settings::FallbackSettings;
use crate::scope::JevOriginScope;

fn fallback_status(result: &Value) -> (&str, &str) {
    (
        result["status"].as_str().unwrap_or_default(),
        result["stopped_because"].as_str().unwrap_or_default(),
    )
}

/// With the operator's gate on, the fallback stops before "Pay now" just as
/// Jev would: nothing is posted. Authorizing the call lets it through only
/// when the fallback also marks that press.
#[tokio::test]
async fn the_gate_stops_the_fallback_before_a_payment() {
    let harness = harness_or_skip!();
    let mut gated = ceilings(FallbackMode::Auto);
    gated.confirm_irreversible = true;
    let result = fall_back(
        &harness,
        "pay.html",
        json!([{"blocked": true}]),
        json!([{"tool": "click", "args": {"ref_of": "Pay now"}}, {"say": "DONE: paid"}]),
        &gated,
    )
    .await;
    let (status, why) = fallback_status(&result);
    assert_eq!(status, "needs_confirmation", "{result:#}");
    assert!(
        why.contains("\"Pay now\"") && why.contains("nothing was pressed"),
        "{why}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(harness.site.posts().is_empty(), "the fallback paid");
    // The digest tells the caller to ask the user, like Jev's gate.
    let text = crate::report::tool_text(&mut result.clone());
    assert!(
        text.contains("Ask the user to confirm that exact step"),
        "{text}"
    );

    let paid = harness.with_new_site().await;
    let sessions = test_sessions();
    let model = Arc::new(ScriptedFallback::from_json(json!([
        {"tool": "click", "args": {"ref_of": "Pay now", "authorize_irreversible": true}},
        {"say": "DONE: paid"}
    ])));
    let result = call_falling_back(
        &paid,
        &sessions,
        "authorized",
        json!({"goal": "pay", "url": paid.site.url("pay.html"), "authorize_irreversible": true}),
        jev(json!([{"blocked": true}])),
        model.clone(),
        &gated,
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "done", "{result:#}");
    // Only an authorized call offers the model the switch at all.
    assert!(
        model
            .click_parameters
            .lock()
            .unwrap()
            .contains(&"authorize_irreversible".to_string())
    );
    let unauthorized = Arc::new(ScriptedFallback::from_json(json!([{"say": "BLOCKED: x"}])));
    fall_back_with(&harness, "pay.html", unauthorized.clone(), &gated).await;
    let offered = unauthorized.click_parameters.lock().unwrap().clone();
    assert!(!offered.is_empty() && !offered.contains(&"authorize_irreversible".to_string()));
    assert_eq!(
        paid.site
            .wait_for_posts(1, Duration::from_secs(2))
            .await
            .len(),
        1
    );
}

/// A navigation outside the operator's origins is refused before it loads,
/// and a page the fallback reaches that refuses automated access ends it
/// `access_denied`, never worked around.
#[tokio::test]
async fn origins_and_access_blocks_bind_the_fallback() {
    let harness = harness_or_skip!();
    let mut scoped = ceilings(FallbackMode::Auto);
    scoped.scope = JevOriginScope::any()
        .narrow(&[harness.site.origin()])
        .unwrap();
    let result = fall_back(
        &harness,
        "canvas-pad.html",
        json!([{"click": "Drawing pad", "repeat": 3}]),
        json!([
            {"tool": "navigate", "args": {"url": "https://example.com/"}},
            {"say": "BLOCKED: that site is outside what I may visit"}
        ]),
        &scoped,
    )
    .await;
    assert_eq!(result["status"], "blocked", "{result:#}");
    let first = &result["fallback"]["actions"][0];
    assert!(
        first["result"]
            .as_str()
            .unwrap()
            .contains("outside the allowed origins"),
        "{first:#}"
    );
    assert!(
        result["url"]
            .as_str()
            .unwrap()
            .starts_with(harness.site.origin())
    );

    let denied = harness.site.url("access-denied.html?status=403");
    let result = fall_back(
        &harness,
        "canvas-pad.html",
        json!([{"click": "Drawing pad", "repeat": 3}]),
        json!([
            {"tool": "navigate", "args": {"url": denied}},
            {"tool": "look"},
            {"say": "DONE: never reached"}
        ]),
        &ceilings(FallbackMode::Auto),
    )
    .await;
    let (status, why) = fallback_status(&result);
    assert_eq!(status, "access_denied", "{result:#}");
    assert!(why.contains("refused automated access"), "{why}");
    // It stopped at once: the model was not asked to go on.
    assert_eq!(result["fallback"]["actions"].as_array().unwrap().len(), 1);
}

/// Its own ceilings: steps, tokens and time, each named when reached.
#[tokio::test]
async fn the_fallback_stops_at_its_own_ceilings() {
    let harness = harness_or_skip!();
    let mut limited = ceilings(FallbackMode::Auto);
    let looks = json!([
        {"tool": "look"}, {"tool": "look"}, {"tool": "look"}, {"tool": "look"}, {"say": "DONE: x"}
    ]);
    limited.fallback = FallbackSettings {
        max_steps: 2,
        ..FallbackSettings::default()
    };
    let steps = fall_back(
        &harness,
        "keyboard-only.html",
        json!([{"blocked": true}]),
        looks.clone(),
        &limited,
    )
    .await;
    assert_eq!(steps["status"], "budget_exceeded", "{steps:#}");
    assert!(
        steps["stopped_because"]
            .as_str()
            .unwrap()
            .contains("2-step ceiling")
    );
    assert_eq!(steps["fallback"]["actions"].as_array().unwrap().len(), 2);

    limited.fallback = FallbackSettings {
        max_tokens: 1500,
        ..FallbackSettings::default()
    };
    let tokens = fall_back(
        &harness,
        "keyboard-only.html",
        json!([{"blocked": true}]),
        looks,
        &limited,
    )
    .await;
    assert_eq!(tokens["status"], "budget_exceeded", "{tokens:#}");
    assert!(
        tokens["stopped_because"]
            .as_str()
            .unwrap()
            .contains("1500-token ceiling")
    );
    // Two replies of 1,050 tokens: the ceiling is checked before each call.
    assert_eq!(tokens["drivers"][1]["model_calls"], 2);

    limited.fallback = FallbackSettings {
        max_duration: Duration::from_secs(2),
        ..FallbackSettings::default()
    };
    let time = fall_back(
        &harness,
        "keyboard-only.html",
        json!([{"blocked": true}]),
        json!([{"tool": "wait", "args": {"ms": 5000}}, {"say": "DONE: x"}]),
        &limited,
    )
    .await;
    assert_eq!(time["status"], "timed_out", "{time:#}");
    let fallback_ms = time["drivers"][1]["elapsed_ms"].as_u64().unwrap();
    assert!(fallback_ms < 4000, "{fallback_ms}");
    // Jev's own time is reported apart from the fallback's.
    assert!(time["drivers"][0]["elapsed_ms"].as_u64().unwrap() < fallback_ms + 30_000);
}

/// A run that ends needs_input does not fall back, and nothing is typed.
#[tokio::test]
async fn needs_input_never_falls_back() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let model = Arc::new(ScriptedFallback::from_json(json!([{"say": "DONE: x"}])));
    let result = call_falling_back(
        &harness,
        &sessions,
        "input",
        json!({"goal": "send", "url": harness.site.url("contact.html")}),
        jev(json!([{"fill": "Name"}])),
        model.clone(),
        &ceilings(FallbackMode::Auto),
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "needs_input", "{result:#}");
    assert!(result.get("fallback").is_none(), "{result:#}");
    assert_eq!(model.played(), 0);
}

/// Hand-over mode runs nothing and tells the caller about the full tools on
/// this tab; off leaves Jev's result alone.
#[tokio::test]
async fn handover_names_the_tools_and_the_tab_and_off_is_silent() {
    let harness = harness_or_skip!();
    let model = Arc::new(ScriptedFallback::from_json(json!([{"say": "DONE: x"}])));
    let result = fall_back(
        &harness,
        "hover-menu.html",
        json!([{"click": "Products", "repeat": 3}]),
        json!([{"say": "DONE: never asked"}]),
        &ceilings(FallbackMode::Handover),
    )
    .await;
    assert_eq!(result["status"], "blocked");
    assert_eq!(result["fallback"]["ran"], false);
    assert_eq!(result["fallback"]["tab"], "t1");
    assert!(result.get("drivers").is_some());
    let tools = result["fallback"]["tools"].as_array().unwrap();
    assert!(tools.contains(&json!("jev_tab_hover")) && tools.contains(&json!("jev_tab_drag")));
    let text = crate::report::tool_text(&mut result.clone());
    for said in [
        "Roder's full browser tools work on this same tab",
        "this thread's Jev tab, t1",
        "jev_tab_look",
        "instead of calling jev_browse again",
    ] {
        assert!(text.contains(said), "{said}: {text}");
    }
    let off = fall_back(
        &harness,
        "hover-menu.html",
        json!([{"click": "Products", "repeat": 3}]),
        json!([{"say": "DONE: never asked"}]),
        &ceilings(FallbackMode::Off),
    )
    .await;
    assert!(off.get("fallback").is_none(), "{off:#}");
    assert_eq!(model.played(), 0);
}

/// A cookie banner is Jev's banner refusal's: with refusal off (the
/// operator's `JEV_REFUSE_COOKIE_BANNERS=0`) the fallback presses nothing in
/// it, and with refusal on it may refuse but never accept.
#[tokio::test]
async fn the_fallback_never_accepts_a_banner_and_leaves_it_when_refusal_is_off() {
    let harness = harness_or_skip!();
    let covered = json!([{"click": "Show more", "repeat": 3}]);
    let off = ceilings(FallbackMode::Auto);
    assert!(!off.refuse_cookie_banners);
    let result = fall_back(
        &harness,
        "cookie-banner.html",
        covered.clone(),
        json!([
            {"tool": "click", "args": {"ref_of": "Reject all"}},
            {"tool": "click", "args": {"ref_of": "Accept all"}},
            {"say": "BLOCKED: a cookie banner covers the article"}
        ]),
        &off,
    )
    .await;
    assert_eq!(result["status"], "blocked", "{result:#}");
    for step in result["fallback"]["actions"].as_array().unwrap() {
        assert_eq!(step["error"], true, "{step:#}");
        assert!(
            step["result"]
                .as_str()
                .unwrap()
                .contains("cookie-banner refusal off"),
            "{step:#}"
        );
    }
    assert!(
        !result["visible_text"]
            .as_str()
            .unwrap()
            .contains("Tomatoes")
    );

    // With refusal on, Jev refuses what it can itself; a banner that only
    // offers acceptance is left, and the fallback may not accept it either.
    let mut on = ceilings(FallbackMode::Auto);
    on.refuse_cookie_banners = true;
    let result = fall_back(
        &harness,
        "cookie-accept-only.html",
        json!([{"blocked": true}]),
        json!([
            {"tool": "click", "args": {"ref_of": "Accept"}},
            {"tool": "click", "args": {"ref_of": "OK"}},
            {"tool": "click", "args": {"ref_of": "Open soup recipe"}},
            {"say": "DONE: the soup recipe shows"}
        ]),
        &on,
    )
    .await;
    let steps = result["fallback"]["actions"].as_array().unwrap();
    for step in &steps[..2] {
        assert!(
            step["result"].as_str().unwrap().contains("only a refusal"),
            "{result:#}"
        );
    }
    assert_ne!(steps[2]["error"], true, "{result:#}");
    assert_eq!(result["status"], "done", "{result:#}");
    let text = result["visible_text"].as_str().unwrap();
    assert!(
        text.contains("Leek and potato soup") && text.contains("uses cookies"),
        "{text}"
    );
}

/// A booking widget in a frame of another site: Jev reads it but never acts
/// in it, and with the gate on the fallback cannot press into it either,
/// since what a press there hits cannot be checked. Nothing is posted.
#[tokio::test]
async fn the_gate_stops_a_press_into_another_sites_frame() {
    let harness = harness_or_skip!();
    let mut gated = ceilings(FallbackMode::Auto);
    gated.confirm_irreversible = true;
    let page = format!(
        "reserve-search.html?date={}&seats=3&q=Mission%20District",
        super::frame_text_tests::today_iso()
    );
    let result = fall_back(
        &harness,
        &page,
        json!([
            {"click": "8:15 PM Dining Room", "context": "Angie's Pizza"},
            {"blocked": true}
        ]),
        json!([
            {"tool": "click", "args": {"at": {"label": "tag:iframe", "fx": 0.5, "fy": 0.8}}},
            {"say": "DONE: reserved"}
        ]),
        &gated,
    )
    .await;
    let (status, why) = fallback_status(&result);
    assert_eq!(status, "needs_confirmation", "{result:#}");
    assert!(why.contains("frame of another site"), "{why}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        harness.site.posts().is_empty(),
        "the fallback pressed into the widget"
    );
}

/// A control under a modal backdrop is not pressed by keyboard either:
/// Tab then Enter (or Space) would get around what covers it. The live
/// corpus found the fallback doing exactly that.
#[tokio::test]
async fn a_covered_control_is_not_pressed_by_keyboard() {
    let harness = harness_or_skip!();
    let result = fall_back(
        &harness,
        "covered.html",
        json!([{"click": "Buy now", "repeat": 3}]),
        json!([
            {"tool": "key", "args": {"key": "Tab"}},
            {"tool": "key", "args": {"key": "Enter"}},
            {"tool": "key", "args": {"key": "Space"}},
            {"say": "BLOCKED: a backdrop covers the button"}
        ]),
        &ceilings(FallbackMode::Auto),
    )
    .await;
    let steps = result["fallback"]["actions"].as_array().unwrap();
    for step in &steps[1..3] {
        assert_eq!(step["error"], true, "{result:#}");
        assert!(
            step["result"]
                .as_str()
                .unwrap()
                .contains("would get around what covers it"),
            "{result:#}"
        );
    }
    assert_eq!(result["status"], "blocked", "{result:#}");
    assert_eq!(
        evaluate_on(&harness, "covered.html", "window.clicks ?? 0").await,
        json!(0)
    );
}

/// Evaluate `expression` in the tab showing `page`, over a connection of
/// the test's own.
async fn evaluate_on(harness: &super::Harness, page: &str, expression: &str) -> Value {
    let mut connection = harness.connect().await.unwrap();
    let targets = connection
        .call("Target.getTargets", json!({}), None)
        .await
        .unwrap();
    let target = targets["targetInfos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["url"].as_str().is_some_and(|url| url.contains(page)))
        .unwrap()["targetId"]
        .clone();
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
            json!({"expression": expression, "returnByValue": true}),
            Some(&session),
        )
        .await
        .unwrap()["result"]["value"]
        .clone()
}

/// Jev answers BLOCKED on `page` and `model` goes on, on a fresh session.
async fn fall_back_with(
    harness: &super::Harness,
    page: &str,
    model: Arc<ScriptedFallback>,
    ceilings: &crate::runner::Ceilings,
) -> Value {
    call_falling_back(
        harness,
        &test_sessions(),
        "with",
        json!({"goal": "scripted", "url": harness.site.url(page)}),
        jev(json!([{"blocked": true}])),
        model,
        ceilings,
    )
    .await
    .unwrap()
}
