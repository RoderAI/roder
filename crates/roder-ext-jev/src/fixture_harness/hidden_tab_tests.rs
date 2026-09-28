//! What a background tab does to timing, measured on a Chrome with a real
//! window (ignored: it opens one on screen). Jev opens its own tab in the
//! background and shows it at once only for a foreground task. This measures
//! whether that tab is hidden, how long two animation frames and a 50 ms
//! timer take there, and what a click and the settle after it cost, with
//! focus emulation on (as `load` sets it) and off. Headless Chrome is
//! measured alongside for comparison. Rows go to
//! `target/jev-fixtures/hidden_tab.jsonl`.
//!
//! ```text
//! cargo test -p roder-ext-jev hidden_tab -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use super::scripted::find;
use super::{Harness, timed_step, write_jsonl};
use crate::page::Page;

const RUNS: usize = 3;

/// Two frames (or `null` after 3 s without them), then a 50 ms timer.
const MEASURE_JS: &str = "(async () => { const t0=performance.now(); \
  const raf=await new Promise(done => { const timer=setTimeout(()=>done(null),3000); \
    requestAnimationFrame(()=>requestAnimationFrame(()=>{ clearTimeout(timer); \
      done(performance.now()-t0); })); }); \
  const t1=performance.now(); \
  const timer=await new Promise(done => setTimeout(()=>done(performance.now()-t1),50)); \
  return {hidden:document.hidden, state:document.visibilityState, \
    double_raf_ms:raf, timer_50_ms:timer}; })()";

#[derive(Debug, Serialize)]
struct HiddenRow {
    mode: &'static str,
    run: usize,
    hidden: Value,
    state: Value,
    double_raf_ms: Value,
    timer_50_ms: Value,
    act_ms: f64,
    click_settle_ms: f64,
}

/// How a measured tab is set up.
#[derive(Clone, Copy)]
struct Mode {
    name: &'static str,
    /// Shown at creation, as a foreground task's tab is.
    foreground: bool,
    /// Focus emulation left on, as `load` sets it.
    emulate_focus: bool,
}

async fn measure(harness: &Harness, mode: Mode) -> Vec<HiddenRow> {
    let mut rows = Vec::new();
    for run in 0..RUNS {
        let connection = harness.connect().await.unwrap();
        let mut tab = Page::create(connection, mode.foreground).await.unwrap();
        tab.load(&harness.site.url("settle-static.html"))
            .await
            .unwrap();
        if !mode.emulate_focus {
            tab.emulate_focus(false).await.unwrap();
        }
        let measured = tab.evaluate_async(MEASURE_JS).await.unwrap();
        let observation = tab.observe().await.unwrap();
        let action = find(&observation, "click", "Show note").unwrap().clone();
        let (next, timing) = timed_step(&mut tab, &observation, &action, None)
            .await
            .unwrap();
        tab.close().await.ok();
        assert!(
            next["text"].as_str().unwrap().contains("Note shown"),
            "{}: {next:#}",
            mode.name
        );
        rows.push(HiddenRow {
            mode: mode.name,
            run,
            hidden: measured["hidden"].clone(),
            state: measured["state"].clone(),
            double_raf_ms: measured["double_raf_ms"].clone(),
            timer_50_ms: measured["timer_50_ms"].clone(),
            act_ms: timing.act_ms,
            click_settle_ms: timing.settle_ms,
        });
    }
    rows
}

#[tokio::test]
#[ignore = "opens a Chrome window on screen"]
async fn hidden_tab_timing() {
    let Some(headed) = Harness::start_headed().await else {
        eprintln!("skipping: no Chrome binary found (set JEV_CHROME_BINARY)");
        return;
    };
    let headless = harness_or_skip!();
    let mode = |name, foreground, emulate_focus| Mode {
        name,
        foreground,
        emulate_focus,
    };
    let mut rows = Vec::new();
    rows.extend(measure(&headless, mode("headless background", false, true)).await);
    rows.extend(measure(&headed, mode("windowed background", false, true)).await);
    rows.extend(measure(&headed, mode("windowed shown", true, true)).await);
    // What focus emulation prevents: a background tab that is hidden. The
    // click must still land and the settle stay bounded without frames.
    let hidden = measure(&headed, mode("windowed hidden", false, false)).await;
    rows.extend(hidden);
    for row in &rows {
        eprintln!(
            "{:<20} run {}: hidden {} ({}), two frames {} ms, 50 ms timer {} ms, \
             click act {:.0} ms, click settle {:.0} ms",
            row.mode,
            row.run,
            row.hidden,
            row.state,
            row.double_raf_ms,
            row.timer_50_ms,
            row.act_ms,
            row.click_settle_ms
        );
    }
    write_jsonl("hidden_tab", &rows).unwrap();
    for row in &rows {
        let hidden = row.mode == "windowed hidden";
        assert_eq!(row.hidden, Value::Bool(hidden), "{row:?}");
        // Without frames the click skips the pointer move rather than wait
        // 5 s for its reply, and the settle is bounded by clamped timers.
        assert!(row.act_ms < 1000.0, "{row:?}");
        assert!(row.click_settle_ms < 3000.0, "{row:?}");
    }
}

/// A pointer move's reply in a background tab, with focus emulation on (as
/// Jev runs) and off: a hidden tab holds it for 5 s waiting for a frame,
/// which is why a hidden tab's press skips the move.
#[tokio::test]
#[ignore = "opens a Chrome window on screen"]
async fn hidden_tab_pointer_move() {
    let Some(headed) = Harness::start_headed().await else {
        eprintln!("skipping: no Chrome binary found (set JEV_CHROME_BINARY)");
        return;
    };
    for emulate in [true, false] {
        let connection = headed.connect().await.unwrap();
        let mut tab = Page::create(connection, false).await.unwrap();
        tab.load(&headed.site.url("settle-static.html"))
            .await
            .unwrap();
        tab.emulate_focus(emulate).await.unwrap();
        let started = Instant::now();
        tab.call(
            "Input.dispatchMouseEvent",
            json!({"type": "mouseMoved", "x": 40, "y": 40}),
        )
        .await
        .unwrap();
        let moved = started.elapsed();
        tab.close().await.ok();
        eprintln!("focus emulation {emulate}: pointer move answered in {moved:?}");
        if emulate {
            assert!(moved < Duration::from_secs(1), "{moved:?}");
        }
    }
}
