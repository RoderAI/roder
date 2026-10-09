//! The automatic fallback on our own pages, through the same session call
//! `jev_browse` makes: Jev ends blocked (a popover only an icon closes, a
//! canvas-only control, a drag, a hover menu, a page with nothing to act
//! on), and a scripted fallback model goes on with the full browser tools in
//! the same tab. Chrome's own target list is counted before and after, so a
//! fallback that opened a tab or another browser would show.

use std::sync::Arc;

use serde_json::{Value, json};

use super::Harness;
use super::evals::{Step, StepDecider};
use super::fallback_script::ScriptedFallback;
use super::sessions::{call_falling_back, targets, test_sessions};
use crate::fallback::{FallbackMode, FallbackSettings};
use crate::runner::Ceilings;

pub(super) fn ceilings(mode: FallbackMode) -> Ceilings {
    Ceilings {
        refuse_cookie_banners: false,
        fallback: FallbackSettings {
            mode,
            ..FallbackSettings::default()
        },
        ..Ceilings::default()
    }
}

pub(super) fn jev(plan: Value) -> Arc<StepDecider> {
    let plan: Vec<Step> = serde_json::from_value(plan).unwrap();
    Arc::new(StepDecider::new(&plan))
}

/// One call on a fresh session: Jev plays `plan`, the scripted fallback
/// `fallback`. Checks the tab count stays one tab above where it began,
/// and that the session still owns exactly the tab Jev used.
pub(super) async fn fall_back(
    harness: &Harness,
    page: &str,
    plan: Value,
    fallback: Value,
    ceilings: &Ceilings,
) -> Value {
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    let model = Arc::new(ScriptedFallback::from_json(fallback));
    let result = call_falling_back(
        harness,
        &sessions,
        "fallback",
        json!({"goal": "scripted", "url": harness.site.url(page)}),
        jev(plan),
        model,
        ceilings,
    )
    .await
    .unwrap();
    let after = harness.settled_page_targets(before + 1).await;
    if after != before + 1 {
        let listed = harness.owned_pages().await;
        panic!("{before} -> {after} tabs: {listed:#?}\n{result:#}");
    }
    assert_eq!(targets(&sessions, "fallback").len(), 1, "{result:#}");
    result
}

fn ran(result: &Value) {
    assert_eq!(result["fallback"]["ran"], true, "{result:#}");
    assert_eq!(result["status"], "done", "{result:#}");
    let drivers = result["drivers"].as_array().unwrap();
    assert_eq!(drivers[0]["driver"], "jev");
    assert_eq!(drivers[1]["driver"], "fallback");
    assert_eq!(drivers[1]["model"], "scripted/fallback (none)");
    // Each scripted reply reports 1,000 input and 50 output tokens.
    let calls = drivers[1]["model_calls"].as_u64().unwrap();
    assert_eq!(drivers[1]["usage"]["input_tokens"], json!(1000 * calls));
    assert_eq!(drivers[1]["usage"]["output_tokens"], json!(50 * calls));
    assert_eq!(result["session"]["tab"], "t1");
}

#[tokio::test]
async fn a_popover_only_an_icon_closes_is_closed_by_the_fallback() {
    let harness = harness_or_skip!();
    let result = fall_back(
        &harness,
        "popover.html?close=icon",
        json!([{"click": "Date: Today"}, {"click": "28"}, {"click": "7:00 PM", "repeat": 3}]),
        json!([
            {"tool": "click", "args": {"ref_of": "tag:span"}},
            {"tool": "click", "args": {"ref_of": "7:00 PM"}},
            {"say": "DONE: 7:00 PM at Luna Cafe is selected"}
        ]),
        &ceilings(FallbackMode::Auto),
    )
    .await;
    ran(&result);
    assert_eq!(result["jev_status"], "blocked");
    assert_eq!(result["stop_cause"], "covered");
    assert_eq!(result["fallback"]["trigger"]["kind"], "covered");
    assert!(
        result["visible_text"]
            .as_str()
            .unwrap()
            .contains("Selected 7:00 PM at Luna Cafe"),
        "{result:#}"
    );
    assert_eq!(result["fallback"]["actions"].as_array().unwrap().len(), 2);
    assert_eq!(
        result["fallback"]["message"],
        "7:00 PM at Luna Cafe is selected"
    );
    // The result's page is Jev's own read of the final page.
    assert!(result["controls"].as_array().is_some_and(|c| !c.is_empty()));
}

#[tokio::test]
async fn a_canvas_only_control_is_pressed_at_its_coordinates() {
    let harness = harness_or_skip!();
    let result = fall_back(
        &harness,
        "canvas-pad.html",
        json!([{"click": "Drawing pad", "repeat": 3}]),
        json!([
            {"tool": "click", "args": {"at": {"label": "Drawing pad", "fx": 0.84, "fy": 0.22}}},
            {"say": "DONE: the pad says Canvas OK pressed"}
        ]),
        &ceilings(FallbackMode::Auto),
    )
    .await;
    ran(&result);
    assert_eq!(result["fallback"]["trigger"]["kind"], "stalled");
    assert!(
        result["visible_text"]
            .as_str()
            .unwrap()
            .contains("Canvas OK pressed"),
        "{result:#}"
    );
}

#[tokio::test]
async fn a_card_is_dragged_to_another_column() {
    let harness = harness_or_skip!();
    let result = fall_back(
        &harness,
        "drag-cards.html",
        json!([{"click": "Card A", "repeat": 3}]),
        json!([
            {"tool": "drag", "args": {"from_ref_of": "Card A", "to_ref_of": "Done column"}},
            {"say": "DONE: Card A is in Done"}
        ]),
        &ceilings(FallbackMode::Auto),
    )
    .await;
    ran(&result);
    assert!(
        result["visible_text"]
            .as_str()
            .unwrap()
            .contains("Moved Card A to Done"),
        "{result:#}"
    );
}

#[tokio::test]
async fn a_hover_menu_is_opened_by_hovering() {
    let harness = harness_or_skip!();
    let result = fall_back(
        &harness,
        "hover-menu.html",
        json!([{"click": "Products", "repeat": 3}]),
        json!([
            {"tool": "hover", "args": {"ref_of": "Products"}},
            {"tool": "click", "args": {"ref_of": "Laptops"}},
            {"say": "DONE: the laptops page is open"}
        ]),
        &ceilings(FallbackMode::Auto),
    )
    .await;
    ran(&result);
    assert!(
        result["url"]
            .as_str()
            .unwrap()
            .ends_with("/pages/landing.html?page=laptops"),
        "{result:#}"
    );
    assert_eq!(result["title"], "Landing");
}

#[tokio::test]
async fn a_page_with_nothing_to_act_on_is_driven_by_the_keyboard() {
    let harness = harness_or_skip!();
    let result = fall_back(
        &harness,
        "keyboard-only.html",
        json!([{"blocked": true}]),
        json!([
            {"tool": "key", "args": {"key": "k"}},
            {"say": "DONE: the code is 4417"}
        ]),
        &ceilings(FallbackMode::Auto),
    )
    .await;
    ran(&result);
    assert_eq!(result["fallback"]["trigger"]["kind"], "nothing_to_act_on");
    assert!(
        result["visible_text"]
            .as_str()
            .unwrap()
            .contains("Code: 4417"),
        "{result:#}"
    );
    // The digest the caller reads says who did what.
    let text = crate::report::tool_text(&mut result.clone());
    assert!(text.contains("after a fallback"), "{text}");
    assert!(text.contains("Drivers: Jev"), "{text}");
    assert!(text.contains("What the fallback did:"), "{text}");
}

/// The next call goes on where the fallback left the tab, in the same tab.
#[tokio::test]
async fn jevs_next_call_goes_on_where_the_fallback_left_the_tab() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let ceilings = ceilings(FallbackMode::Auto);
    let first = call_falling_back(
        &harness,
        &sessions,
        "on",
        json!({"goal": "open laptops", "url": harness.site.url("hover-menu.html")}),
        jev(json!([{"click": "Products", "repeat": 3}])),
        Arc::new(ScriptedFallback::from_json(json!([
            {"tool": "hover", "args": {"ref_of": "Products"}},
            {"tool": "click", "args": {"ref_of": "Laptops"}},
            {"say": "DONE: laptops"}
        ]))),
        &ceilings,
    )
    .await
    .unwrap();
    assert_eq!(first["status"], "done", "{first:#}");
    let tab = targets(&sessions, "on");
    let second = call_falling_back(
        &harness,
        &sessions,
        "on",
        json!({"goal": "go back to basics", "url": ""}),
        jev(json!([{"click": "Back to basics"}])),
        Arc::new(ScriptedFallback::from_json(json!([]))),
        &ceilings,
    )
    .await
    .unwrap();
    assert_eq!(second["session"]["tab_note"], "continued", "{second:#}");
    assert_eq!(second["status"], "done");
    assert!(second.get("fallback").is_none(), "{second:#}");
    assert_eq!(targets(&sessions, "on"), tab);
    assert_eq!(second["session"]["totals"]["fallback_actions"], 2);
}

/// The screenshot tool returns its picture as a tool result, so a model the
/// picture would not reach (its engine does not forward tool-result images)
/// gets neither the tool nor advice to use it; one it reaches gets both.
#[tokio::test]
async fn the_screenshot_tool_is_offered_only_to_a_model_shown_tool_result_images() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    for sees_images in [true, false] {
        let plan = json!([
            {"tool": "click", "args": {"ref_of": "Products"}},
            {"say": "DONE: looked"}
        ]);
        let scripted = ScriptedFallback::from_json(plan);
        let model = Arc::new(match sees_images {
            true => scripted,
            false => scripted.without_images(),
        });
        let result = call_falling_back(
            &harness,
            &sessions,
            &format!("pictures-{sees_images}"),
            json!({"goal": "open laptops", "url": harness.site.url("hover-menu.html")}),
            jev(json!([{"blocked": true}])),
            model.clone(),
            &ceilings(FallbackMode::Auto),
        )
        .await
        .unwrap();
        assert_eq!(result["fallback"]["ran"], true, "{result:#}");
        let offered = model.offered_tools();
        assert_eq!(
            offered.contains(&"jev_tab_screenshot".to_string()),
            sees_images,
            "{offered:?}"
        );
        // Everything else is offered either way.
        for tool in ["look", "click", "type", "key", "scroll", "select", "wait"] {
            assert!(offered.contains(&format!("jev_tab_{tool}")), "{offered:?}");
        }
        let said = format!("{}\n{}", model.instructions(), model.opening());
        assert_eq!(said.contains("screenshot"), sees_images, "{said}");
    }
}
