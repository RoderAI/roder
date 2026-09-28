//! Reaching controls: offscreen targets, capped lists that keep their pagers,
//! and the multi-point hit test with its covered outcome.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::scripted::{PlanDecider, find, pick};
use super::{timed_step, write_jsonl};
use crate::decide::request_body;
use crate::engine::JevStatus;

/// Observed element actions, without the scroll and wait controls.
fn elements(observation: &Value) -> Vec<&Value> {
    observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["node"].is_i64())
        .collect()
}

fn offscreen(action: &Value) -> bool {
    action.get("offscreen") == Some(&json!(true))
}

#[tokio::test]
async fn a_target_below_the_fold_is_offered_and_clicked_without_a_scroll_step() {
    let harness = harness_or_skip!();
    let mut page = harness.open("long.html").await.unwrap();
    let observation = page.observe().await.unwrap();

    let top = find(&observation, "click", "Top action").expect("onscreen control");
    assert!(!offscreen(top), "{top}");
    let below = find(&observation, "click", "Subscribe")
        .expect("offscreen control kept")
        .clone();
    assert!(offscreen(&below), "{below}");
    // Onscreen controls come first.
    assert_eq!(top["id"], json!("e1"));
    assert_eq!(below["id"], json!("e2"));
    assert!(find(&observation, "scroll", "Scroll down").is_some());

    let (next, timing) = timed_step(&mut page, &observation, &below, None)
        .await
        .unwrap();
    assert_eq!(page.evaluate("window.clicks ?? 0").await.unwrap(), json!(1));
    assert_eq!(
        page.evaluate("window.top_clicks ?? 0").await.unwrap(),
        json!(0)
    );
    assert!(next["scroll"]["y"].as_f64().unwrap() > 0.0, "{next:#}");
    assert!(next["text"].as_str().unwrap().contains("Subscribed"));
    // Scrolled into view, the same control is now on screen.
    assert!(!offscreen(find(&next, "click", "Subscribe").unwrap()));
    write_jsonl("offscreen_click_timing", &[timing]).unwrap();

    // Through the loop: one decision reaches it, with no scroll step.
    let decider = Arc::new(PlanDecider::new(vec![pick("click", "Subscribe")]));
    let result = harness
        .run("long.html", decider, None, Duration::from_secs(30))
        .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.model_calls, 2);
    let kinds = result
        .actions
        .iter()
        .map(|action| (action.kind.as_str(), action.page_changed))
        .collect::<Vec<_>>();
    assert_eq!(kinds, vec![("click", Some(true))]);
    assert!(result.visible_text.contains("Subscribed"));
}

#[tokio::test]
async fn a_long_list_keeps_its_nearest_items_and_every_pager() {
    let harness = harness_or_skip!();
    let mut page = harness.open("paginated.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let observed = elements(&observation);
    let (on, off): (Vec<&Value>, Vec<&Value>) = observed
        .iter()
        .copied()
        .partition(|action| !offscreen(action));

    // The offscreen cap holds, and the three pagers survive it at the end
    // of a 300-item list.
    assert_eq!(off.len(), 100, "{} offscreen", off.len());
    for pager in ["2", "Next page ›", "Show 20 more"] {
        let kept = find(&observation, "click", pager).unwrap_or_else(|| panic!("{pager} kept"));
        assert!(offscreen(kept));
    }
    // A carousel button and a content link named like pagers are not kept.
    assert!(find(&observation, "click", "Next").is_none());
    assert!(find(&observation, "click", "Learn more").is_none());
    assert!(observation["omitted_actions"].as_u64().unwrap() > 150);

    // Onscreen items in document order, then the nearest offscreen ones.
    let label = |action: &Value| action["label"].as_str().unwrap().to_string();
    let last_on = label(on.last().unwrap());
    let n = last_on
        .trim_start_matches("Item ")
        .parse::<usize>()
        .unwrap();
    assert_eq!(label(off[0]), format!("Item {}", n + 1));
    assert!(observed[..on.len()].iter().all(|a| !offscreen(a)));

    // What keeping offscreen controls costs in the decision request, against
    // the onscreen-only observation this page produced before.
    let bytes = |page: &Value| {
        let (body, _, _) = request_body(
            page,
            "Open page two",
            &[],
            "jev-latest",
            chrono::NaiveDate::default(),
        );
        serde_json::to_string(&body).unwrap().len()
    };
    let mut onscreen_only = observation.clone();
    onscreen_only["actions"] = json!(
        observation["actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|action| !offscreen(action))
            .collect::<Vec<_>>()
    );
    write_jsonl(
        "offscreen_payload",
        &[json!({
            "page": "paginated.html",
            "onscreen": on.len(),
            "offscreen": off.len(),
            "request_bytes": bytes(&observation),
            "onscreen_only_request_bytes": bytes(&onscreen_only),
        })],
    )
    .unwrap();

    // An offscreen pager is directly actionable.
    let next = find(&observation, "click", "2").unwrap().clone();
    page.act(&next, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    let landed = page.observe().await.unwrap();
    assert!(
        landed["url"]
            .as_str()
            .unwrap()
            .ends_with("landing.html?page=2"),
        "{}",
        landed["url"]
    );
}

#[tokio::test]
async fn the_overall_cap_keeps_pagers_ahead_of_onscreen_controls() {
    let harness = harness_or_skip!();
    let mut page = harness.open("paginated.html?grid=300").await.unwrap();
    let observation = page.observe().await.unwrap();
    let observed = elements(&observation);

    assert_eq!(observed.len(), 250);
    for pager in ["2", "Next page ›", "Show 20 more"] {
        assert!(find(&observation, "click", pager).is_some(), "{pager}");
    }
    assert!(find(&observation, "click", "Cell 1").is_some());
    // The onscreen controls that did not fit are dropped, last first.
    assert!(find(&observation, "click", "Cell 300").is_none());
    assert_eq!(
        observed.iter().filter(|action| offscreen(action)).count(),
        3,
        "only the pagers are kept offscreen"
    );
}

#[tokio::test]
async fn a_badge_over_the_centre_is_clicked_around() {
    let harness = harness_or_skip!();
    let mut page = harness.open("badge.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let cart = find(&observation, "click", "Open cart").unwrap().clone();

    page.act(&cart, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(page.evaluate("window.clicks ?? 0").await.unwrap(), json!(1));
    assert_eq!(
        page.evaluate("window.badge_clicks ?? 0").await.unwrap(),
        json!(0)
    );
    // Not at the centre, which the badge covers.
    let rect = &cart["rect"];
    let centre = rect["x"].as_f64().unwrap() + rect["w"].as_f64().unwrap() / 2.0;
    let at = page.evaluate("window.at").await.unwrap();
    assert!((at[0].as_f64().unwrap() - centre).abs() > 10.0, "{at}");
}

#[tokio::test]
async fn a_row_is_not_clicked_through_its_nested_delete_button() {
    let harness = harness_or_skip!();
    let mut page = harness.open("row.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let row = find(&observation, "click", "Open invoice 42")
        .unwrap()
        .clone();

    // The row's centre is the Delete button; a quarter point opens the row.
    page.act(&row, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(page.evaluate("window.opened ?? 0").await.unwrap(), json!(1));
    assert_eq!(
        page.evaluate("window.deleted ?? 0").await.unwrap(),
        json!(0)
    );

    // The nested button is still its own target.
    let observation = page.observe().await.unwrap();
    let delete = find(&observation, "click", "Delete").unwrap().clone();
    page.act(&delete, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("window.deleted ?? 0").await.unwrap(),
        json!(1)
    );
    assert_eq!(page.evaluate("window.opened ?? 0").await.unwrap(), json!(1));
}

#[tokio::test]
async fn a_button_that_stays_covered_blocks_the_run_after_three_tries() {
    let harness = harness_or_skip!();
    let decider = Arc::new(PlanDecider::new(vec![pick("click", "Buy now"); 5]));
    let result = harness
        .run("covered.html", decider, None, Duration::from_secs(30))
        .await;

    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.stopped_because, None);
    // One decision per try, then the stall rule: no model call is wasted
    // on a retry that cannot land.
    assert_eq!(result.model_calls, 3);
    assert_eq!(result.actions.len(), 3);
    assert!(
        result
            .actions
            .iter()
            .all(|action| action.covered && action.page_changed == Some(false))
    );
}

#[tokio::test]
async fn controls_no_scroll_can_reach_are_not_offered() {
    let harness = harness_or_skip!();
    let mut page = harness.open("reach-edge.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    // A skip link above the document and a bar slid out of a fixed layer
    // would be offered offscreen, then fail as stale on every pick.
    assert!(
        find(&observation, "click", "Skip to main content").is_none(),
        "{observation:#}"
    );
    assert!(
        find(&observation, "click", "Accept cookies").is_none(),
        "{observation:#}"
    );
    assert!(find(&observation, "click", "Item 1").is_some());
}

#[tokio::test]
async fn controls_cut_off_by_an_unscrollable_ancestor_are_not_offered() {
    let harness = harness_or_skip!();
    let mut page = harness.open("reach-edge.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    // A closed accordion panel, and the part of a strip overflow:clip hides.
    assert!(
        find(&observation, "click", "Collapsed action").is_none(),
        "{observation:#}"
    );
    assert!(
        find(&observation, "click", "Strip second").is_none(),
        "{observation:#}"
    );
    assert!(find(&observation, "click", "Strip first").is_some());
    // An absolute menu escapes its collapsed static ancestor and shows.
    assert!(find(&observation, "click", "Floating menu").is_some());

    // A carousel's hidden slide scrolls into place, so it stays on offer.
    let slide = find(&observation, "click", "Slide two").unwrap().clone();
    page.act(&slide, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("window.slide_clicks ?? 0").await.unwrap(),
        json!(1)
    );
}

#[tokio::test]
async fn a_target_clipped_by_its_own_scroller_is_scrolled_to_and_clicked() {
    let harness = harness_or_skip!();
    let mut page = harness.open("reach-edge.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    // Inside the window, so not offscreen, but hidden by the list's scroll.
    let inner = find(&observation, "click", "Inner item").unwrap().clone();
    assert!(!offscreen(&inner), "{inner}");

    page.act(&inner, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("window.inner_clicks ?? 0").await.unwrap(),
        json!(1)
    );
    let list_scroll = page
        .evaluate("document.getElementById('list').scrollTop")
        .await
        .unwrap();
    assert!(list_scroll.as_f64().unwrap() > 0.0, "{list_scroll}");
}

#[tokio::test]
async fn a_control_its_nested_input_fills_is_still_clicked() {
    let harness = harness_or_skip!();
    let mut page = harness.open("reach-edge.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    // The ARIA 1.1 combobox wrapper, offered alongside its own input.
    let wrapper = find(&observation, "click", "City").unwrap().clone();
    assert!(find(&observation, "fill", "City").is_some());

    page.act(&wrapper, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("window.city_focus ?? 0").await.unwrap(),
        json!(1)
    );
}

#[tokio::test]
async fn changes_below_the_fold_do_not_hide_a_stall() {
    let harness = harness_or_skip!();
    let plan = (0..6).map(|_| pick("click", "Retry")).collect();
    let decider = Arc::new(PlanDecider::new(plan));
    let result = harness
        .run(
            "offscreen-ticker.html",
            decider,
            None,
            Duration::from_secs(30),
        )
        .await;
    // Only the offscreen countdown moves, so three no-op clicks still stall.
    assert_eq!(result.status, JevStatus::Blocked, "{result:#?}");
    assert_eq!(result.stopped_because, None);
    let changed = result
        .actions
        .iter()
        .map(|action| action.page_changed)
        .collect::<Vec<_>>();
    assert_eq!(changed, vec![Some(false); 3], "{result:#?}");
}
