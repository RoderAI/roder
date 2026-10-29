//! One delegation retains the facts needed from pages it has left.
mod support;
use roder_ext_jev::{JevEngine, JevEngineConfig, JevStatus};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use support::{ScriptedBrowser, ScriptedDecider, page};

#[tokio::test]
async fn multi_page_research_reaches_driver_and_parent_without_another_delegation() {
    let mut prices = page("prices", &[("open", "click")]);
    prices["url"] = json!("https://scripted.test/prices");
    prices["text"] = json!("Medium box retail price 25.80");
    let mut details = page("details", &[("more", "scroll")]);
    details["url"] = json!("https://scripted.test/product");
    details["text"] = json!("Inside dimensions 11 by 8.5 by 5.5 inches");
    let decider = Arc::new(ScriptedDecider::new(&["open", "DONE"]));
    let log = decider.log();
    let config = JevEngineConfig::new(
        "Compare prices then open and verify product dimensions",
        decider,
    )
    .with_wait(Duration::ZERO);
    let mut engine = JevEngine::start(
        Box::new(ScriptedBrowser::new(vec![prices, details])),
        config,
    )
    .await
    .unwrap();
    let result = engine.run(Duration::from_secs(5)).await;
    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.actions.len(), 1);
    for text in [
        &result.evidence,
        log.lock()
            .unwrap()
            .evidence
            .last()
            .unwrap()
            .as_str()
            .unwrap(),
    ] {
        assert!(text.contains("https://scripted.test/prices"));
        assert!(text.contains("25.80"));
        assert!(text.contains("https://scripted.test/product"));
        assert!(text.contains("11 by 8.5"));
    }
}
