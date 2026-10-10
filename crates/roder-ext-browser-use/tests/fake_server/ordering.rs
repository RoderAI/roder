//! The action report, the observation after it, and the errors that still
//! carry one.

use roder_ext_browser_use::{BrowserUseConfig, UNTRUSTED_NOTE};
use serde_json::json;

use crate::support::{fake_launch, keyed_config, registry_for, run, state_json};

#[tokio::test]
async fn the_action_report_comes_first_and_survives_a_long_state() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);

    let short = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/short"}),
    )
    .await;
    assert!(!short.is_error, "{}", short.text);
    let report = short
        .text
        .find("Navigated to: https://example.com/short")
        .expect("report");
    let observation = short
        .text
        .find("Observed page after the action")
        .expect("observation");
    assert!(report < observation, "{}", short.text);
    assert!(
        short.text[..report].contains("claim"),
        "the report is labelled as a claim: {}",
        short.text
    );
    assert!(short.text.starts_with(UNTRUSTED_NOTE), "{}", short.text);

    // The fake serves a 60,000-byte state for this page; the cut must take
    // the tail of the state, never the report.
    let long = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/long"}),
    )
    .await;
    assert!(!long.is_error, "{}", long.text);
    assert!(
        long.text.contains("Navigated to: https://example.com/long"),
        "the report was cut: {}",
        &long.text[..long.text.len().min(400)]
    );
    assert!(long.text.contains("truncated"), "the cut must be visible");
    assert!(long.text.len() < 30_000, "{} bytes", long.text.len());
    assert!(
        long.text.contains("screenshot attached"),
        "the screenshot note survives the cut"
    );
    assert_eq!(long.data["__view_image"]["detail"], "original");
    server.shutdown().await;
}

#[tokio::test]
async fn the_agent_tool_returns_its_own_report_without_a_foreign_observation() {
    let server = fake_launch(keyed_config(), None);
    let registry = registry_for(server.clone(), true);

    let before = run(&registry, "browser_use_get_state", json!({})).await;
    assert_eq!(state_json(&before.text)["state_calls"], 1);

    let agent = run(
        &registry,
        "browser_use_agent",
        json!({"task": "read the heading"}),
    )
    .await;
    assert!(!agent.is_error, "{}", agent.text);
    assert!(agent.text.contains("Task completed"), "{}", agent.text);
    assert!(
        !agent.text.contains("Observed page after the action"),
        "{}",
        agent.text
    );
    assert!(agent.text.starts_with(UNTRUSTED_NOTE), "{}", agent.text);
    assert!(
        agent.text.contains("claim"),
        "the agent report is labelled as a claim: {}",
        agent.text
    );
    assert!(agent.data.get("__view_image").is_none());

    // Only the two explicit get_state calls reached the server: the agent
    // call did not trigger a third one.
    let after = run(&registry, "browser_use_get_state", json!({})).await;
    assert_eq!(state_json(&after.text)["state_calls"], 2);
    server.shutdown().await;
}

#[tokio::test]
async fn dead_clicks_and_types_are_errors_that_still_carry_a_fresh_observation() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);

    for (tool, args) in [
        ("browser_use_click", json!({"index": 424242})),
        (
            "browser_use_type",
            json!({"index": 424242, "text": "hello"}),
        ),
    ] {
        let dead = run(&registry, tool, args).await;
        assert!(dead.is_error, "{tool}: {}", dead.text);
        assert!(
            dead.text.contains("Element with index 424242 not found"),
            "{tool}: {}",
            dead.text
        );
        assert!(
            dead.text.contains("Observed page after the action"),
            "the model still gets the page to re-pick an index from: {}",
            dead.text
        );
    }

    let live = run(&registry, "browser_use_click", json!({"index": 3})).await;
    assert!(!live.is_error, "{}", live.text);
    assert!(live.text.contains("browser_click ok"), "{}", live.text);
    server.shutdown().await;
}
