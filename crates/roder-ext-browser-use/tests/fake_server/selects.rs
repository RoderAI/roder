//! Native `<select>` elements: refused before the server sees them, and the
//! per-thread record of the selects the model was shown.

use roder_api::tools::ToolRegistry;
use roder_ext_browser_use::BrowserUseConfig;
use serde_json::json;

use crate::compact_state::counter;
use crate::support::{OPENAI_KEY, fake_launch, keyed_config, registry_for, run, run_thread};

/// Opens the order form, whose index 7 is a native select the observation
/// shows to the model.
async fn open_form(registry: &ToolRegistry, thread: &str) -> roder_api::tools::ToolResult {
    let form = run_thread(
        registry,
        thread,
        "browser_use_navigate",
        json!({"url": "https://example.com/form"}),
    )
    .await;
    assert!(!form.is_error, "{}", form.text);
    assert!(
        form.text
            .contains("[7] select \"Pick one Alpha Banana Cherry\""),
        "{}",
        form.text
    );
    form
}

/// The clicks and typed texts the server has received so far.
async fn calls_seen(registry: &ToolRegistry, thread: &str) -> (u32, u32) {
    let state = run_thread(registry, thread, "browser_use_get_state", json!({})).await;
    assert!(!state.is_error, "{}", state.text);
    (
        counter(&state.text, "clicks"),
        counter(&state.text, "types"),
    )
}

#[tokio::test]
async fn a_click_on_a_native_select_is_refused_and_the_server_never_sees_it() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    let form = open_form(&registry, "thread").await;
    assert_eq!(counter(&form.text, "clicks"), 0);

    let refused = run(&registry, "browser_use_click", json!({"index": 7})).await;
    assert!(refused.is_error, "{}", refused.text);
    for expected in [
        "element 7 is a native <select>",
        "reports success",
        "jev_browse",
        "chrome_select",
        "different browser",
        "Nothing was sent",
    ] {
        assert!(
            refused.text.contains(expected),
            "{expected}: {}",
            refused.text
        );
    }
    // No OpenAI key is configured, so the agent is not offered.
    assert!(
        !refused.text.contains("browser_use_agent"),
        "{}",
        refused.text
    );
    assert_eq!(refused.data["provider"], "browser-use");
    assert_eq!(
        calls_seen(&registry, "thread").await,
        (0, 0),
        "the click reached the server"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn typing_into_a_native_select_is_refused_and_the_server_never_sees_it() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    open_form(&registry, "thread").await;

    let refused = run(
        &registry,
        "browser_use_type",
        json!({"index": 7, "text": "Banana"}),
    )
    .await;
    assert!(refused.is_error, "{}", refused.text);
    assert!(
        refused.text.contains("element 7 is a native <select>"),
        "{}",
        refused.text
    );
    assert!(
        refused.text.contains("clears the select's choice"),
        "typing is not offered as a route: {}",
        refused.text
    );
    assert!(!refused.text.contains("Banana"), "{}", refused.text);
    assert_eq!(
        calls_seen(&registry, "thread").await,
        (0, 0),
        "the typed text reached the server"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn the_refusal_offers_the_agent_only_when_a_key_is_configured() {
    let server = fake_launch(keyed_config(), None);
    let registry = registry_for(server.clone(), true);
    open_form(&registry, "thread").await;
    let refused = run(&registry, "browser_use_click", json!({"index": 7})).await;
    assert!(refused.is_error, "{}", refused.text);
    assert!(
        refused.text.contains("browser_use_agent"),
        "{}",
        refused.text
    );
    assert!(
        refused.text.contains("its own temporary browser"),
        "the agent's browser is not this page: {}",
        refused.text
    );
    assert!(refused.text.contains("jev_browse"));
    assert!(refused.text.contains("chrome_select"));
    assert!(!refused.text.contains(OPENAI_KEY));
    server.shutdown().await;
}

#[tokio::test]
async fn everything_that_is_not_a_native_select_still_reaches_the_server() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    open_form(&registry, "thread").await;

    for index in [3, 9, 11] {
        let clicked = run(&registry, "browser_use_click", json!({"index": index})).await;
        assert!(!clicked.is_error, "{index}: {}", clicked.text);
        assert!(
            clicked.text.contains("browser_click ok"),
            "{}",
            clicked.text
        );
    }
    let typed = run(
        &registry,
        "browser_use_type",
        json!({"index": 5, "text": "Ada"}),
    )
    .await;
    assert!(!typed.is_error, "{}", typed.text);
    assert!(typed.text.contains("browser_type ok"), "{}", typed.text);
    // A click that names coordinates as well is a coordinate click on the
    // server, which ignores the index.
    let at = run(
        &registry,
        "browser_use_click",
        json!({"index": 7, "coordinate_x": 10, "coordinate_y": 20}),
    )
    .await;
    assert!(!at.is_error, "{}", at.text);
    // An index that is not an integer is not one the page listed.
    let odd = run(&registry, "browser_use_click", json!({"index": "7"})).await;
    assert!(!odd.is_error, "{}", odd.text);

    assert_eq!(calls_seen(&registry, "thread").await, (5, 1));
    server.shutdown().await;
}

#[tokio::test]
async fn the_selects_are_those_of_the_last_state_the_thread_was_shown() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    open_form(&registry, "thread").await;
    assert!(
        run(&registry, "browser_use_click", json!({"index": 7}))
            .await
            .is_error
    );

    // Another page: index 7 is an ordinary element there.
    let other = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/elements/10"}),
    )
    .await;
    assert!(!other.is_error, "{}", other.text);
    let clicked = run(&registry, "browser_use_click", json!({"index": 7})).await;
    assert!(!clicked.is_error, "{}", clicked.text);
    assert!(
        clicked.text.contains("browser_click ok"),
        "{}",
        clicked.text
    );

    // A state the wrapper cannot read says nothing about selects.
    open_form(&registry, "thread").await;
    run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/plain"}),
    )
    .await;
    let unread = run(&registry, "browser_use_click", json!({"index": 7})).await;
    assert!(!unread.is_error, "{}", unread.text);
    server.shutdown().await;
}

#[tokio::test]
async fn a_select_that_only_an_explicit_get_state_shows_is_refused_from_then_on() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    // The observation after the navigation has no select yet.
    let loaded = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/late-select"}),
    )
    .await;
    assert!(!loaded.text.contains("[7] select"), "{}", loaded.text);
    let early = run(&registry, "browser_use_click", json!({"index": 7})).await;
    assert!(
        !early.is_error,
        "nothing had shown index 7 as a select: {}",
        early.text
    );

    // The next state does.
    let state = run(&registry, "browser_use_get_state", json!({})).await;
    assert!(state.text.contains("[7] select "), "{}", state.text);
    assert_eq!(counter(&state.text, "clicks"), 1);
    let late = run(&registry, "browser_use_click", json!({"index": 7})).await;
    assert!(late.is_error, "{}", late.text);
    assert_eq!(calls_seen(&registry, "thread").await, (1, 0));
    server.shutdown().await;
}

#[tokio::test]
async fn a_select_is_only_known_in_the_thread_that_was_shown_it() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    open_form(&registry, "first").await;
    let own = run_thread(&registry, "first", "browser_use_click", json!({"index": 7})).await;
    assert!(own.is_error, "{}", own.text);

    // The second thread's browser has not been shown that page.
    let other = run_thread(
        &registry,
        "second",
        "browser_use_click",
        json!({"index": 7}),
    )
    .await;
    assert!(!other.is_error, "{}", other.text);
    assert!(other.text.contains("browser_click ok"), "{}", other.text);
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_lost_browser_takes_what_was_known_about_its_selects_with_it() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    open_form(&registry, "thread").await;

    let lost = run(&registry, "browser_use_click", json!({"index": 777})).await;
    assert!(lost.is_error, "{}", lost.text);
    // Index 7 meant something in the browser that is gone, not in the fresh one.
    let fresh = run(&registry, "browser_use_click", json!({"index": 7})).await;
    assert!(!fresh.is_error, "{}", fresh.text);
    assert!(
        fresh.text.contains("ran in a fresh browser"),
        "{}",
        fresh.text
    );
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_call_the_server_turns_down_leaves_what_was_known_about_its_selects() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    open_form(&registry, "thread").await;

    // The server answers this call with a JSON-RPC error and runs nothing, so
    // the page the model was shown is the page that is still there.
    let rejected = run(&registry, "browser_use_click", json!({"index": 555})).await;
    assert!(rejected.is_error, "{}", rejected.text);
    assert!(
        !rejected.text.contains("shut down"),
        "the browser was not lost: {}",
        rejected.text
    );
    let still = run(&registry, "browser_use_click", json!({"index": 7})).await;
    assert!(still.is_error, "{}", still.text);
    assert!(
        still.text.contains("element 7 is a native <select>"),
        "{}",
        still.text
    );
    assert_eq!(calls_seen(&registry, "thread").await, (0, 0));
    server.shutdown().await;
}
