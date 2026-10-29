//! Native recovery makes observable progress without duplicating side effects.
mod support;
use async_trait::async_trait;
use roder_ext_jev::{JevActOutcome, JevBrowser, JevEngine, JevEngineConfig, JevStatus};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use support::{ScriptedBrowser, ScriptedDecider, labelled_page};

#[tokio::test]
async fn uncertain_navigation_replans_once_without_repeating_the_click() {
    let mut before = labelled_page("before", &[("shop", "click", "Shop")]);
    let mut after = labelled_page(
        "after",
        &[("shop", "click", "Shop"), ("supplies", "click", "Supplies")],
    );
    for page in [&mut before, &mut after] {
        for action in page["actions"].as_array_mut().unwrap() {
            action["role"] = json!("link");
        }
    }
    let browser = ScriptedBrowser::new(vec![before, after.clone(), after.clone(), after]);
    let log = browser.log();
    let decision = ScriptedDecider::new(&["shop", "shop", "supplies", "DONE"])
        .with_confidences(&[1.0, 0.63, 1.0, 1.0]);
    let mut engine = JevEngine::start(
        Box::new(browser),
        JevEngineConfig::new("Read supplies", Arc::new(decision)).with_wait(Duration::ZERO),
    )
    .await
    .unwrap();
    let result = engine.run(Duration::from_secs(5)).await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(
        log.lock()
            .unwrap()
            .acts
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        vec!["shop", "supplies"]
    );
    assert!(
        result
            .recoveries
            .iter()
            .any(|r| r["kind"] == "navigation_repeat")
    );
}

struct ReadingBrowser(ScriptedBrowser, Value);
#[async_trait]
impl JevBrowser for ReadingBrowser {
    fn document_read_available(&self) -> bool {
        true
    }
    async fn document_text(&mut self) -> anyhow::Result<Option<Value>> {
        Ok(Some(self.1.clone()))
    }
    async fn observe(&mut self) -> anyhow::Result<Value> {
        self.0.observe().await
    }
    async fn fresh(&mut self, o: &Value, a: Option<&Value>) -> anyhow::Result<bool> {
        self.0.fresh(o, a).await
    }
    async fn act(
        &mut self,
        a: &Value,
        o: &Value,
        t: Option<&str>,
        w: Duration,
    ) -> anyhow::Result<JevActOutcome> {
        let result = self.0.act(a, o, t, w).await;
        if a["label"] == "Change contents" {
            self.1["text"] = json!("New source fact");
        }
        result
    }
}
#[tokio::test]
async fn document_reads_page_into_retained_evidence_without_browser_input() {
    let page = labelled_page("page", &[]);
    let browser = ScriptedBrowser::new(vec![page.clone()]);
    let log = browser.log();
    let text = format!("First fact\n{}\nLast fact", "x".repeat(6500));
    let browser = ReadingBrowser(browser, json!({"url":page["url"],"text":text}));
    let decision = ScriptedDecider::new(&["read_document", "read_document", "DONE"]);
    let mut engine = JevEngine::start(
        Box::new(browser),
        JevEngineConfig::new("Read both facts", Arc::new(decision)).with_wait(Duration::ZERO),
    )
    .await
    .unwrap();
    let result = engine.run(Duration::from_secs(5)).await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert!(
        result.evidence.contains("First fact") && result.evidence.contains("Last fact"),
        "{}",
        result.evidence
    );
    assert_eq!(result.actions.len(), 2);
    assert!(log.lock().unwrap().acts.is_empty());
}

#[tokio::test]
async fn another_uncertain_navigation_does_not_reset_the_recovery_budget() {
    let mut page = labelled_page("before", &[("shop", "click", "Shop")]);
    page["actions"][0]["role"] = json!("link");
    let mut after = page.clone();
    after["fingerprint"] = json!("after");
    let browser = ScriptedBrowser::new(vec![page, after]);
    let log = browser.log();
    let decision =
        ScriptedDecider::new(&["shop", "shop", "shop"]).with_confidences(&[1.0, 0.63, 0.63]);
    let mut engine = JevEngine::start(
        Box::new(browser),
        JevEngineConfig::new("Read supplies", Arc::new(decision)),
    )
    .await
    .unwrap();
    let result = engine.run(Duration::from_secs(5)).await;
    assert_eq!(result.status, JevStatus::Blocked);
    assert_eq!(log.lock().unwrap().acts.len(), 1);
    assert_eq!(
        result
            .recoveries
            .iter()
            .filter(|r| r["kind"] == "navigation_repeat")
            .count(),
        1
    );
}

struct StaleOnceBrowser {
    inner: ScriptedBrowser,
    stale: bool,
}
#[async_trait]
impl JevBrowser for StaleOnceBrowser {
    async fn observe(&mut self) -> anyhow::Result<Value> {
        self.inner.observe().await
    }
    async fn fresh(&mut self, o: &Value, a: Option<&Value>) -> anyhow::Result<bool> {
        self.inner.fresh(o, a).await
    }
    async fn act(
        &mut self,
        a: &Value,
        o: &Value,
        t: Option<&str>,
        w: Duration,
    ) -> anyhow::Result<JevActOutcome> {
        if std::mem::take(&mut self.stale) {
            return Err(roder_ext_jev::StaleObservation::new(
                "Page changed since this decision: scroll_y. Observe again.",
            )
            .into());
        }
        self.inner.act(a, o, t, w).await
    }
}
async fn stale_navigation(form_changed: bool) -> roder_ext_jev::JevRunResult {
    let mut page = labelled_page("old", &[("next", "click", "Details")]);
    page["actions"][0]["role"] = json!("link");
    page["page_key"] = json!([1, "https://scripted.test/", 0, 0, 1200, 800, []]);
    page["guards"] = json!({"1":[1,"link","Details",null,null,null,null,false,false,null,null,null,"https://scripted.test/details",null,"Details"]});
    let mut moved = page.clone();
    moved["page_key"][3] = json!(100);
    moved["fingerprint"] = json!("scrolled");
    if form_changed {
        moved["page_key"][6] = json!([[2, "changed value"]]);
    }
    let browser = StaleOnceBrowser {
        inner: ScriptedBrowser::new(vec![page, moved]),
        stale: true,
    };
    let choices = if form_changed {
        vec!["next", "next", "DONE"]
    } else {
        vec!["next", "DONE"]
    };
    let mut engine = JevEngine::start(
        Box::new(browser),
        JevEngineConfig::new("Open details", Arc::new(ScriptedDecider::new(&choices)))
            .with_wait(Duration::ZERO),
    )
    .await
    .unwrap();
    engine.run(Duration::from_secs(5)).await
}
#[tokio::test]
async fn unchanged_exact_navigation_can_be_revalidated_without_another_model_call() {
    let result = stale_navigation(false).await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.model_calls, 2);
    assert_eq!(result.actions.len(), 1);
    assert!(
        result
            .recoveries
            .iter()
            .any(|r| r["kind"] == "stale_revalidated")
    );
}
#[tokio::test]
async fn changed_form_state_requires_a_new_decision_even_for_the_same_link() {
    let result = stale_navigation(true).await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.model_calls, 3);
    assert!(
        !result
            .recoveries
            .iter()
            .any(|r| r["kind"] == "stale_revalidated")
    );
}

#[tokio::test]
async fn research_mode_reads_on_arrival_within_the_existing_action_budget() {
    let page = labelled_page("source", &[]);
    let browser = ScriptedBrowser::new(vec![page.clone()]);
    let log = browser.log();
    let browser = ReadingBrowser(
        browser,
        json!({"url":page["url"],"text":"Observed price $25 and dimensions 11 x 8 x 5"}),
    );
    let mut engine = JevEngine::start(
        Box::new(browser),
        JevEngineConfig::new(
            "Read price and dimensions",
            Arc::new(ScriptedDecider::new(&["DONE"])),
        )
        .with_research(true)
        .with_max_actions(1),
    )
    .await
    .unwrap();
    let result = engine.run(Duration::from_secs(5)).await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(result.model_calls, 1);
    assert_eq!(result.actions.len(), 1);
    assert!(result.evidence.contains("Observed price $25"));
    assert!(log.lock().unwrap().acts.is_empty());
}

#[tokio::test]
async fn research_requires_each_checklist_step_before_returning_done() {
    let page = labelled_page("source", &[]);
    let browser = ReadingBrowser(
        ScriptedBrowser::new(vec![page.clone()]),
        json!({"url":page["url"],"text":"Price $25. Dimensions 11 x 8 x 5. Tracking included."}),
    );
    let config = JevEngineConfig::new(
        "Compare and check tracking",
        Arc::new(ScriptedDecider::new(&["DONE", "DONE", "DONE"])),
    )
    .with_research(true)
    .with_requirements(vec![
        "Compare prices".into(),
        "Check dimensions".into(),
        "Check tracking".into(),
    ])
    .unwrap();
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    let result = engine.run(Duration::from_secs(5)).await;
    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.model_calls, 3);
    assert_eq!(
        result
            .recoveries
            .iter()
            .filter(|r| r["kind"] == "requirement_completed")
            .count(),
        3
    );
    assert!(
        result
            .recoveries
            .iter()
            .all(|r| r["verification"] == "driver_claim")
    );
}

#[tokio::test]
async fn advancing_requirements_does_not_reset_the_model_call_budget() {
    let page = labelled_page("source", &[]);
    let browser = ReadingBrowser(
        ScriptedBrowser::new(vec![page.clone()]),
        json!({"url":page["url"],"text":"A source fact"}),
    );
    let config = JevEngineConfig::new(
        "Read three facts",
        Arc::new(ScriptedDecider::new(&["DONE", "DONE", "DONE"])),
    )
    .with_research(true)
    .with_max_actions(1)
    .with_requirements(vec![
        "Read first fact".into(),
        "Read second fact".into(),
        "Read third fact".into(),
    ])
    .unwrap();
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    let result = engine.run(Duration::from_secs(5)).await;
    assert_eq!(result.status, JevStatus::BudgetExceeded);
    assert_eq!(result.model_calls, 2);
    assert_eq!(
        result
            .recoveries
            .iter()
            .filter(|r| r["kind"] == "requirement_completed")
            .count(),
        2
    );
}

#[tokio::test]
async fn document_snapshot_is_refreshed_after_content_changes_without_navigation() {
    let page = labelled_page("source", &[("change", "click", "Change contents")]);
    let browser = ReadingBrowser(
        ScriptedBrowser::new(vec![page.clone()]),
        json!({"url":page["url"],"text":"Old source fact"}),
    );
    let config = JevEngineConfig::new(
        "Read, change, read again",
        Arc::new(ScriptedDecider::new(&[
            "read_document",
            "change",
            "read_document",
            "DONE",
        ])),
    );
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    let result = engine.run(Duration::from_secs(5)).await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert!(
        result.evidence.contains("New source fact"),
        "{}",
        result.evidence
    );
}
