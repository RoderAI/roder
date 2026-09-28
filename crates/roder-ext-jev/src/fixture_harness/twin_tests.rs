//! Controls that read alike: each twin carries a `context` naming the card,
//! row, section or table column it belongs to, and the loop can pick one by it.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::scripted::{PlanDecider, pick_in};
use super::{timed_step, write_jsonl};
use crate::decide::request_body;
use crate::engine::JevStatus;

/// (label, context) of every observed action with this kind and label.
fn twins<'a>(observation: &'a Value, kind: &str, label: &str) -> Vec<(&'a Value, String)> {
    observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["kind"] == json!(kind) && action["label"] == json!(label))
        .map(|action| {
            let context = action["context"].as_str().unwrap_or_default().to_string();
            (action, context)
        })
        .collect()
}

fn contexts(observation: &Value, kind: &str, label: &str) -> Vec<String> {
    twins(observation, kind, label)
        .into_iter()
        .map(|(_, context)| context)
        .collect()
}

fn distinct(contexts: &[String]) -> bool {
    contexts.iter().collect::<HashSet<_>>().len() == contexts.len()
}

/// What each twin was named, for reading the output by eye.
#[derive(serde::Serialize)]
struct Named {
    page: &'static str,
    label: &'static str,
    contexts: Vec<String>,
}

#[tokio::test]
async fn product_cards_are_named_by_their_product() {
    let harness = harness_or_skip!();
    let mut page = harness.open("products.html").await.unwrap();
    let observation = page.observe().await.unwrap();

    // Every card ends in a shared "Details" heading, so each is named by its
    // first title; card three's hidden "Clearance" heading is skipped.
    let carts = contexts(&observation, "click", "Add to cart");
    assert_eq!(
        carts,
        [
            "Trail Runner",
            "Canvas Tote",
            "Wool Beanie",
            "Steel Bottle",
            "Rain Shell",
            "Desk Lamp",
            "Yoga Mat",
            "Travel Mug"
        ],
        "{observation:#}"
    );
    // Nothing around the icon buttons names them, so they differ by position.
    let shares = contexts(&observation, "click", "Share");
    assert_eq!(shares, ["1 of 3", "2 of 3", "3 of 3"]);
    // A control with a unique label on screen carries no context.
    let heading = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["node"].is_i64())
        .filter(|action| {
            action["label"] != json!("Add to cart") && action["label"] != json!("Share")
        })
        .find(|action| action.get("context").is_some());
    assert_eq!(heading, None);

    // The model sees each twin's context beside its label.
    let (body, _, _) = request_body(
        &observation,
        "Add the tote",
        &[],
        "jev-latest",
        chrono::NaiveDate::default(),
    );
    let mut bare = observation.clone();
    for action in bare["actions"].as_array_mut().unwrap() {
        action.as_object_mut().unwrap().remove("context");
    }
    let (without, _, _) = request_body(
        &bare,
        "Add the tote",
        &[],
        "jev-latest",
        chrono::NaiveDate::default(),
    );
    write_jsonl(
        "twin_payload",
        &[json!({
            "page": "products.html",
            "request_bytes": body.to_string().len(),
            "without_context_bytes": without.to_string().len(),
        })],
    )
    .unwrap();
    let criteria = body["questions"]["click_target"]["criteria"]
        .as_object()
        .unwrap();
    let tote = criteria
        .values()
        .find(|entry| entry["context"] == json!("Canvas Tote"))
        .expect("the tote's twin is offered with its context");
    assert!(tote["element"].as_str().unwrap().ends_with("Add to cart"));

    // Clicking by context reaches the right card.
    let (tote, _) = twins(&observation, "click", "Add to cart")
        .into_iter()
        .find(|(_, context)| context == "Canvas Tote")
        .unwrap();
    let tote = tote.clone();
    let (_, timing) = timed_step(&mut page, &observation, &tote, None)
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("window.cart").await.unwrap(),
        json!(["Canvas Tote"])
    );

    write_jsonl(
        "twin_contexts_products",
        &[
            Named {
                page: "products.html",
                label: "Add to cart",
                contexts: carts,
            },
            Named {
                page: "products.html",
                label: "Share",
                contexts: shares,
            },
        ],
    )
    .unwrap();
    write_jsonl("twin_click_timing", &[timing]).unwrap();
}

#[tokio::test]
async fn calendar_days_are_named_by_month_and_weekday() {
    let harness = harness_or_skip!();
    let mut page = harness.open("calendar.html").await.unwrap();
    let observation = page.observe().await.unwrap();

    // 1 October 2026 is a Thursday and 1 November a Sunday: the weekday
    // comes from the table's columns, past the empty leading cells.
    assert_eq!(
        contexts(&observation, "click", "1"),
        [
            "October 2026; column: Thursday",
            "November 2026; column: Sunday"
        ],
        "{observation:#}"
    );
    // November names itself with a plain line: the day "2" is cut out as a
    // whole word only, so "November 2026" survives intact.
    assert_eq!(
        contexts(&observation, "click", "2"),
        [
            "October 2026; column: Friday",
            "November 2026; column: Monday"
        ]
    );
    assert_eq!(
        contexts(&observation, "click", "20"),
        [
            "October 2026; column: Tuesday",
            "November 2026; column: Friday"
        ]
    );
    // Only October has a 31st: no twin, but its weekday still comes along.
    assert_eq!(contexts(&observation, "click", "31"), ["column: Saturday"]);
    // October's caption does not name the navigation outside its table, and
    // a header cell never names a control outside its row.
    assert_eq!(
        contexts(&observation, "click", "›"),
        ["October 2026", "November 2026"]
    );
    for day in 1..=30 {
        let named = contexts(&observation, "click", &day.to_string());
        assert_eq!(named.len(), 2, "day {day}");
        assert!(distinct(&named), "day {day}: {named:?}");
        assert!(named[0].starts_with("October 2026; column: "), "{named:?}");
        assert!(named[1].starts_with("November 2026; column: "), "{named:?}");
    }

    // The loop picks November's 20th by its context.
    let decider = Arc::new(PlanDecider::new(vec![pick_in(
        "click",
        "20",
        "November 2026; column: Friday",
    )]));
    let result = harness
        .run("calendar.html", decider, None, Duration::from_secs(30))
        .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.model_calls, 2);
    let kinds = result
        .actions
        .iter()
        .map(|action| (action.action.as_str(), action.page_changed))
        .collect::<Vec<_>>();
    assert_eq!(kinds, vec![("20", Some(true))]);
    assert!(result.visible_text.contains("Selected: November 20"));

    // Directly: the twin named November is November's.
    let (november, _) = twins(&observation, "click", "20")
        .into_iter()
        .find(|(_, context)| context.starts_with("November"))
        .unwrap();
    let november = november.clone();
    timed_step(&mut page, &observation, &november, None)
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("window.picked").await.unwrap(),
        json!("November 20")
    );

    write_jsonl(
        "twin_contexts_calendar",
        &[Named {
            page: "calendar.html",
            label: "20",
            contexts: contexts(&observation, "click", "20"),
        }],
    )
    .unwrap();
}

#[tokio::test]
async fn repeated_table_links_are_named_by_row_and_column() {
    let harness = harness_or_skip!();
    let mut page = harness.open("contacts.html").await.unwrap();
    let observation = page.observe().await.unwrap();

    // The row names each link, its own label cut out; the header that spans
    // two rows names the column.
    let edits = contexts(&observation, "click", "Edit");
    assert_eq!(
        edits,
        [
            "Ada Lovelace ada@example.test Call; column: Actions",
            "Alan Turing alan@example.test Call; column: Actions",
            "Grace Hopper grace@example.test Call; column: Actions",
            "Edsger Dijkstra edsger@example.test Call; column: Actions"
        ],
        "{observation:#}"
    );
    assert!(distinct(&edits));
    // A header spanning two columns names both with its sub-header.
    let calls = contexts(&observation, "click", "Call");
    assert!(
        calls
            .iter()
            .all(|context| context.ends_with("; column: Contact / Phone")),
        "{calls:?}"
    );
    assert!(distinct(&calls));

    let (grace, _) = twins(&observation, "click", "Edit")
        .into_iter()
        .find(|(_, context)| context.starts_with("Grace Hopper"))
        .unwrap();
    let grace = grace.clone();
    timed_step(&mut page, &observation, &grace, None)
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("window.edited").await.unwrap(),
        json!("Grace Hopper")
    );

    write_jsonl(
        "twin_contexts_contacts",
        &[
            Named {
                page: "contacts.html",
                label: "Edit",
                contexts: edits,
            },
            Named {
                page: "contacts.html",
                label: "Call",
                contexts: calls,
            },
        ],
    )
    .unwrap();
}

#[tokio::test]
async fn a_context_never_makes_a_decision_stale() {
    let harness = harness_or_skip!();
    let mut page = harness.open("products.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    assert!(
        observation["marker"]
            .to_string()
            .find("Trail Runner")
            .is_some()
    );
    // The marker is built before contexts are added, so none reaches it.
    assert!(!observation["marker"].to_string().contains("\"context\""));
    assert!(page.fresh(&observation, None).await.unwrap());
}

#[tokio::test]
async fn an_offscreen_control_carries_its_section() {
    let harness = harness_or_skip!();
    let mut page = harness.open("long.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    // Page text is viewport-only, so the heading above it is its only context.
    let below = contexts(&observation, "click", "Subscribe");
    assert_eq!(below, ["Long page"], "{observation:#}");
    let top = twins(&observation, "click", "Top action");
    assert_eq!(top[0].0.get("context"), None);
}

/// Median milliseconds of `runs` evaluations of `script`.
async fn median_ms(page: &mut crate::page::Page, script: &str, runs: usize) -> f64 {
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let started = std::time::Instant::now();
        page.evaluate(script).await.unwrap();
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    times[runs / 2]
}

#[derive(serde::Serialize)]
struct ContextCost {
    twins: usize,
    snapshot_ms: f64,
    with_context_ms: f64,
}

#[tokio::test]
async fn many_twins_stay_bounded_and_distinct() {
    use crate::page::{CONTEXT_JS, SNAPSHOT_JS};

    let harness = harness_or_skip!();
    let mut rows = Vec::new();
    for twins in [8, 250] {
        let mut page = harness
            .open(&format!("twin-grid.html?n={twins}"))
            .await
            .unwrap();
        let observation = page.observe().await.unwrap();
        let named = contexts(&observation, "click", "Add to cart");
        assert_eq!(named.len(), twins.min(250));
        assert!(distinct(&named), "{named:?}");
        // The nearest 50 are named by their card; the rest by position.
        let walked = named.iter().filter(|c| c.starts_with("Product ")).count();
        assert_eq!(walked, twins.min(50), "{named:?}");
        assert!(named.iter().all(|context| context.chars().count() <= 120));

        let snapshot_ms = median_ms(&mut page, SNAPSHOT_JS, 7).await;
        let with_context_ms =
            median_ms(&mut page, &format!("({CONTEXT_JS})({SNAPSHOT_JS})"), 7).await;
        rows.push(ContextCost {
            twins,
            snapshot_ms,
            with_context_ms,
        });
    }
    let path = write_jsonl("twin_context_cost", &rows).unwrap();
    eprintln!("context cost written to {}", path.display());
    // Generous: a debug build under parallel tests, but not unbounded.
    assert!(rows.iter().all(|row| row.with_context_ms < 3000.0));
}
