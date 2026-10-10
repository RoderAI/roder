//! The in-page settle against pages that are static, render late from an
//! XHR, spin forever, or never go quiet. Timings are written as JSONL to
//! `target/jev-fixtures/settle_timing.jsonl`; run with `--nocapture` to see
//! the medians.

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use super::scripted::find;
use super::{Harness, millis, timed_step, write_jsonl};
use crate::engine::StaleObservation;

const RUNS: usize = 3;

#[derive(Debug, Serialize)]
struct SettleRow {
    page: String,
    label: String,
    run: usize,
    /// `Page::open`: navigation, readiness and the first settle.
    open_ms: f64,
    /// Whether the first observation shows `on_open`.
    open_rendered: bool,
    /// The click: freshness check, hit test and CDP input.
    act_ms: f64,
    /// The settle after it. The quiet clock starts at the click, so the time
    /// the act's reply took already counts toward the quiet window.
    settle_ms: f64,
    /// The read after it, which settles again if the document went away.
    observe_ms: f64,
    /// Whether the observation after the click's settle shows `on_click`.
    rendered: bool,
}

/// One page kind: the page, the control clicked, the text expected on the
/// first observation and after the click, and the least time the click to
/// the next read (act, settle and observe) can take. Upper bounds are left
/// to the cap, since a loaded machine only ever makes a step slower.
struct Scenario {
    page: &'static str,
    label: &'static str,
    on_open: &'static str,
    on_click: &'static str,
    /// False where drawing in time depends on the machine: a reply with no
    /// indicator is only waited for while it lands inside the quiet window.
    must_draw: bool,
    floor_ms: f64,
}

/// `SETTLE_CAP` plus room for the DevTools round trips on a busy machine.
pub(super) const SETTLE_CEILING_MS: f64 = 4500.0;

/// Open the page `RUNS` times, click once each, and record how long opening
/// and the post-click settle took and whether each read saw the page drawn.
async fn measure(harness: &Harness, scenario: &Scenario) -> Vec<SettleRow> {
    let mut rows = Vec::new();
    for run in 0..RUNS {
        let started = Instant::now();
        let mut tab = harness.open(scenario.page).await.unwrap();
        let open_ms = millis(started.elapsed());
        let observation = tab.observe().await.unwrap();
        let action = find(&observation, "click", scenario.label)
            .unwrap_or_else(|| panic!("{} on {}: {observation:#}", scenario.label, scenario.page))
            .clone();
        let (next, timing) = timed_step(&mut tab, &observation, &action, None)
            .await
            .unwrap();
        tab.close().await.ok();
        rows.push(SettleRow {
            page: scenario.page.to_string(),
            label: scenario.label.to_string(),
            run,
            open_ms,
            open_rendered: text(&observation).contains(scenario.on_open),
            act_ms: timing.act_ms,
            settle_ms: timing.settle_ms,
            observe_ms: timing.observe_ms,
            rendered: text(&next).contains(scenario.on_click),
        });
    }
    rows
}

fn text(observation: &Value) -> &str {
    observation["text"].as_str().unwrap_or_default()
}

fn click_to_read(row: &SettleRow) -> f64 {
    row.act_ms + row.settle_ms + row.observe_ms
}

fn median(rows: &[SettleRow], value: impl Fn(&SettleRow) -> f64) -> f64 {
    let mut values = rows.iter().map(value).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[tokio::test]
async fn settle_waits_for_late_renders_and_is_bounded() {
    let harness = harness_or_skip!();
    let scenarios = [
        // The click itself restarts the quiet clock, so 200 ms is the floor.
        Scenario {
            page: "settle-static.html",
            label: "Show note",
            on_open: "Nothing yet",
            on_click: "Note shown",
            must_draw: true,
            floor_ms: 190.0,
        },
        // A link whose server answers after 400 ms: the old document's wait
        // dies with it and carries on in the new one.
        Scenario {
            page: "settle-static.html",
            label: "Slow landing",
            on_open: "Nothing yet",
            on_click: "You arrived.",
            must_draw: true,
            floor_ms: 390.0,
        },
        // A loading indicator holds the settle past the quiet window.
        Scenario {
            page: "settle-xhr.html?latency=600",
            label: "Load results",
            on_open: "Search results",
            on_click: "3 results loaded",
            must_draw: true,
            floor_ms: 590.0,
        },
        // No indicator: a reply inside the quiet window is still waited for,
        // when it is handled in time. Under load its handler can queue past
        // the window, so only the window itself is a floor.
        Scenario {
            page: "settle-xhr.html?latency=150&indicator=0",
            label: "Load results",
            on_open: "Search results",
            on_click: "3 results loaded",
            must_draw: false,
            floor_ms: 190.0,
        },
        // An SPA that loads its content on start is drawn by the first read.
        Scenario {
            page: "settle-xhr.html?latency=400&autoload=1",
            label: "Load results",
            on_open: "3 results loaded",
            on_click: "3 results loaded",
            must_draw: true,
            floor_ms: 390.0,
        },
        // A spinner that never stops is ignored after 1.5 s.
        Scenario {
            page: "settle-spinner.html",
            label: "Search",
            on_open: "Endless search",
            on_click: "Endless search",
            must_draw: true,
            floor_ms: 1490.0,
        },
        // A page that never goes quiet pays the 2.5 s cap. The floor sits
        // well under it but above the loading grace: a renderer stalled for
        // longer than the quiet window used to read as quiet, and a poll that
        // runs late now defers to the page's queued timers instead.
        Scenario {
            page: "settle-busy.html",
            label: "Refresh",
            on_open: "Not refreshed",
            on_click: "Refreshed",
            must_draw: true,
            floor_ms: 2000.0,
        },
    ];
    let mut all = Vec::new();
    for scenario in &scenarios {
        let rows = measure(&harness, scenario).await;
        let count = |pick: fn(&SettleRow) -> bool| rows.iter().filter(|row| pick(row)).count();
        eprintln!(
            "settle {} / {}: open median {:.0} ms (drawn {}/{RUNS}), \
             click settle median {:.0} ms, click to read {:.0} ms (drawn {}/{RUNS})",
            scenario.page,
            scenario.label,
            median(&rows, |row| row.open_ms),
            count(|row| row.open_rendered),
            median(&rows, |row| row.settle_ms),
            median(&rows, click_to_read),
            count(|row| row.rendered),
        );
        all.extend(rows);
    }
    write_jsonl("settle_timing", &all).unwrap();
    for (row, scenario) in all
        .iter()
        .zip(scenarios.iter().flat_map(|scenario| [scenario; RUNS]))
    {
        assert!(row.open_rendered, "{row:?}");
        assert!(row.rendered || !scenario.must_draw, "{row:?}");
        assert!(click_to_read(row) >= scenario.floor_ms, "{row:?}");
        assert!(row.settle_ms < SETTLE_CEILING_MS, "{row:?}");
    }
}

#[tokio::test]
async fn an_already_quiet_page_settles_at_once() {
    let harness = harness_or_skip!();
    let mut page = harness.open("settle-static.html").await.unwrap();
    page.observe().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let outcome = page.settle_page(None).await;
    assert_eq!(outcome["reason"], "quiet", "{outcome}");
    assert!(outcome["waited_ms"].as_u64().unwrap() < 20, "{outcome}");
}

#[tokio::test]
async fn a_combobox_fill_returns_once_suggestions_show() {
    let harness = harness_or_skip!();
    let mut page = harness.open("settle-combobox.html").await.unwrap();
    // The ticker also changes the text a fill's freshness check compares, so
    // on a loaded machine the act can go stale; observe again, as the loop
    // does. With a dozen test threads at once, most attempts do.
    let mut attempts = 0;
    let city = loop {
        let observation = page.observe().await.unwrap();
        let city = find(&observation, "fill", "City")
            .unwrap_or_else(|| panic!("{observation:#}"))
            .clone();
        // The ticker never lets the page go quiet, so only the listbox exit
        // returns before the cap.
        match page
            .act(&city, &observation, Some("Par"), Duration::from_millis(100))
            .await
        {
            Ok(_) => break city,
            Err(error) if error.is::<StaleObservation>() && attempts < 40 => attempts += 1,
            Err(error) => panic!("{error:#}"),
        }
    };
    let outcome = page.settle_page(Some(&city)).await;
    assert_eq!(outcome["reason"], "listbox", "{outcome}");
    let next = page.observe().await.unwrap();
    assert!(find(&next, "click", "Paris").is_some(), "{next:#}");
}

#[tokio::test]
async fn a_page_that_never_goes_quiet_stops_at_the_cap() {
    let harness = harness_or_skip!();
    let mut page = harness.open("settle-busy.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let refresh = find(&observation, "click", "Refresh").unwrap().clone();
    page.act(&refresh, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    // After input the page is polled on timers, so a renderer stall longer
    // than the quiet window is caught rather than read as quiet.
    let started = Instant::now();
    let outcome = page.settle_page(Some(&refresh)).await;
    let elapsed = started.elapsed();
    assert_eq!(outcome["reason"], "cap", "{outcome}");
    // The cap is held in Rust too, so only DevTools round trips add to it;
    // a loaded machine can make those slow.
    assert!(elapsed >= Duration::from_millis(2400), "{elapsed:?}");
    assert!(
        elapsed.as_secs_f64() * 1000.0 < SETTLE_CEILING_MS,
        "{elapsed:?}"
    );
}
