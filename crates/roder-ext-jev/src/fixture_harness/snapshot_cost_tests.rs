//! What one observation, and one step, costs on a large page (`big.html`):
//! time in the page, and bytes on the DevTools socket (see `proxy`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use super::proxy::CountingProxy;
use super::scripted::{PlanDecider, pick};
use super::write_jsonl;
use crate::cdp::Connection;
use crate::engine::{JevEngineConfig, JevStatus};
use crate::page::{CONTEXT_JS, Page, SNAPSHOT_JS, create_target};
use crate::runner::{TabChoice, Task, drive};
use crate::session::SessionTabs;

async fn median_ms(page: &mut Page, script: &str, runs: usize) -> f64 {
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let started = Instant::now();
        page.evaluate_snapshot(script).await.unwrap();
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    times[runs / 2]
}

/// The median of `runs` whole `Page::observe` calls, with the bytes the
/// median-sized call sent to Chrome and received from it.
async fn observe_cost(page: &mut Page, proxy: &CountingProxy, runs: usize) -> (f64, u64, u64) {
    let mut calls = Vec::with_capacity(runs);
    for _ in 0..runs {
        proxy.take();
        let started = Instant::now();
        page.observe().await.unwrap();
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let (sent, received) = proxy.take();
        calls.push((ms, sent, received));
    }
    calls.sort_by(|a, b| a.0.total_cmp(&b.0));
    calls[runs / 2]
}

fn bytes(value: &Value) -> usize {
    value.to_string().len()
}

#[derive(Serialize)]
struct Cost {
    cards: usize,
    elements: u64,
    actions: usize,
    /// `snapshot.js` alone, sent whole (as the scripts read before #4).
    snapshot_ms: f64,
    /// `context.js` over `snapshot.js`, sent whole.
    inline_ms: f64,
    /// A whole `Page::observe`, as the loop reads the page.
    observe_ms: f64,
    observe_sent_bytes: u64,
    observe_received_bytes: u64,
    observation_bytes: usize,
    guards_bytes: usize,
    marker_bytes: usize,
    page_key_bytes: usize,
}

#[tokio::test]
async fn a_large_page_is_observed_quickly() {
    let harness = harness_or_skip!();
    let proxy = CountingProxy::start(harness.endpoint()).await;
    let mut rows = Vec::new();
    for cards in [100, 400, 1000] {
        let connection = Connection::connect(&proxy.url).await.unwrap();
        let url = harness.site.url(&format!("big.html?n={cards}"));
        let mut page = Page::open(connection, &url, false).await.unwrap();
        let observation = page.observe().await.unwrap();
        let elements = page
            .evaluate("document.getElementsByTagName('*').length")
            .await
            .unwrap()
            .as_u64()
            .unwrap_or_default();
        let (observe_ms, observe_sent_bytes, observe_received_bytes) =
            observe_cost(&mut page, &proxy, 7).await;
        let snapshot_ms = median_ms(&mut page, SNAPSHOT_JS, 7).await;
        let inline_ms = median_ms(&mut page, &format!("({CONTEXT_JS})({SNAPSHOT_JS})"), 7).await;
        rows.push(Cost {
            cards,
            elements,
            actions: observation["actions"].as_array().map_or(0, Vec::len),
            snapshot_ms,
            inline_ms,
            observe_ms,
            observe_sent_bytes,
            observe_received_bytes,
            observation_bytes: bytes(&observation),
            guards_bytes: bytes(&observation["guards"]),
            marker_bytes: bytes(&observation["marker"]),
            page_key_bytes: bytes(&observation["page_key"]),
        });
        page.close().await.ok();
    }
    let path = write_jsonl("snapshot_cost", &rows).unwrap();
    eprintln!(
        "{}\nwritten to {}",
        serde_json::to_string_pretty(&json!(
            rows.iter()
                .map(|row| serde_json::to_value(row).unwrap())
                .collect::<Vec<Value>>()
        ))
        .unwrap(),
        path.display()
    );
    // Generous: a debug build under parallel tests, but not unbounded.
    assert!(rows.iter().all(|row| row.observe_ms < 3000.0));
    // The document installed the scripts as it loaded, so a read sends a
    // call, not the scripts (about 70 KB whole).
    for row in &rows {
        assert!(
            row.observe_sent_bytes < 4_000,
            "{} cards: {} bytes sent per observe",
            row.cards,
            row.observe_sent_bytes
        );
    }
    // Guards carry a digest of the text around each control, not the text:
    // a thousand cards' guards stay well under a few hundred bytes each.
    let big = rows.last().unwrap();
    assert!(
        big.guards_bytes < big.actions * 400,
        "{} bytes of guards for {} actions",
        big.guards_bytes,
        big.actions
    );
    assert!(
        big.marker_bytes < 400,
        "{} bytes of marker",
        big.marker_bytes
    );
}

/// The bytes on the DevTools socket per step of a scripted run on a page of a
/// thousand cards: each step settles, reads, checks freshness and acts.
#[tokio::test]
async fn a_step_on_a_large_page_is_cheap_on_the_wire() {
    let harness = harness_or_skip!();
    let proxy = CountingProxy::start(harness.endpoint()).await;
    let decider = Arc::new(PlanDecider::new(vec![
        pick("click", "Add to cart"),
        pick("scroll", "Scroll down"),
        pick("click", "Add to cart"),
        pick("scroll", "Scroll down"),
    ]));
    let started = tokio::time::Instant::now();
    let task = Task {
        url: Some(harness.site.url("big.html?n=1000")),
        tab: TabChoice::Current,
        refused_tab: None,
        foreground: false,
        started,
        deadline: started + Duration::from_secs(60),
        config: JevEngineConfig::new("cost goal", decider).with_wait(Duration::from_millis(100)),
    };
    let mut tabs = SessionTabs::new(3);
    proxy.take();
    proxy.take_registered();
    let result = drive(&proxy.url, &mut tabs, task).await.unwrap().result;
    let (sent, received) = proxy.take();
    let registered = proxy.take_registered();
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    let steps = result.actions.len() as u64;
    assert_eq!(steps, 4, "{result:#?}");
    let row = json!({
        "steps": steps,
        "sent_bytes_per_step": sent / steps,
        "received_bytes_per_step": received / steps,
        "registered_bytes": registered,
        "elapsed_ms": result.elapsed_ms,
    });
    let path = write_jsonl("step_cost", &[&row]).unwrap();
    eprintln!("{row}\nwritten to {}", path.display());
    for target in tabs.targets() {
        harness.close_target(&target).await.ok();
    }
    // Every read and freshness check is a short call on installed scripts;
    // what is left is the settle, the cookie-banner check and the press
    // (about 10 KB each), not the observation scripts (about 70 KB a read).
    assert!(sent / steps < 60_000, "{row}");
}

/// A document that loaded before the tab registered the scripts (a tab Jev
/// attached to) installs them on its first read, and later reads send only
/// the call.
#[tokio::test]
async fn a_document_without_the_snapshot_installs_it_on_the_first_read() {
    let harness = harness_or_skip!();
    let proxy = CountingProxy::start(harness.endpoint()).await;
    let mut connection = Connection::connect(&proxy.url).await.unwrap();
    let target = create_target(&mut connection).await.unwrap();
    let mut page = Page::attach(connection, target, false).await.unwrap();
    page.goto(&harness.site.url("big.html?n=100"))
        .await
        .unwrap();
    let mut sent = Vec::new();
    for _ in 0..3 {
        proxy.take();
        let observation = page.observe().await.unwrap();
        assert!(
            observation["actions"]
                .as_array()
                .is_some_and(|a| a.len() > 100)
        );
        sent.push(proxy.take().0);
    }
    page.close().await.ok();
    assert!(sent[0] > 40_000, "the first read installs: {sent:?}");
    assert!(sent[1] < 4_000 && sent[2] < 4_000, "{sent:?}");
}

/// A page that will not let the scripts be installed is still read, with
/// the scripts sent whole.
#[tokio::test]
async fn a_page_that_refuses_the_install_is_still_observed() {
    let harness = harness_or_skip!();
    let mut connection = harness.connect().await.unwrap();
    let target = create_target(&mut connection).await.unwrap();
    let mut page = Page::attach(connection, target, false).await.unwrap();
    page.goto(&harness.site.url("refuses-install.html"))
        .await
        .unwrap();
    for _ in 0..2 {
        let observation = page.observe().await.unwrap();
        assert!(
            observation["actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["label"] == "Send"),
            "{observation:#}"
        );
        assert!(page.fresh(&observation, None).await.unwrap());
    }
    page.close().await.ok();
}

/// Guards and the page key compare a digest of what they used to carry
/// whole, and still name what changed.
#[tokio::test]
async fn guards_compare_digests_and_still_name_the_change() {
    let harness = harness_or_skip!();
    let mut page = harness.open("big.html?n=100").await.unwrap();
    let observation = page.observe().await.unwrap();
    let add = super::scripted::find(&observation, "click", "Add to cart")
        .unwrap()
        .clone();
    let guard = &observation["guards"][add["node"].to_string()];
    let text = guard.as_array().unwrap().last().unwrap().as_str().unwrap();
    assert!(
        text.len() == 16 && text.chars().all(|c| c.is_ascii_hexdigit()),
        "{guard}"
    );
    let form = observation["page_key"][6].as_str().unwrap();
    assert_eq!(form.len(), 16, "{}", observation["page_key"]);
    assert!(page.fresh(&observation, Some(&add)).await.unwrap());
    // The card's text changes: only the text around the control did.
    page.evaluate("document.querySelector('.card .price').textContent='$1.00'")
        .await
        .unwrap();
    assert_eq!(
        page.freshness_changes(&observation, Some(&add))
            .await
            .unwrap(),
        vec!["target_context"]
    );
    // A field is typed into: only the page's form state did.
    let observation = page.observe().await.unwrap();
    page.evaluate("document.querySelector('input[type=search]').value='boots'")
        .await
        .unwrap();
    assert_eq!(
        page.freshness_changes(&observation, Some(&add))
            .await
            .unwrap(),
        vec!["form_state"]
    );
    // The marker's digests name their field too.
    let observation = page.observe().await.unwrap();
    page.evaluate("document.title='Big store, page 2'")
        .await
        .unwrap();
    assert_eq!(
        page.freshness_changes(&observation, None).await.unwrap(),
        vec!["title"]
    );
    page.close().await.ok();
}
