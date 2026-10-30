//! Real DOM, the actual bounded agent loop, and the shared model boundary.
use super::evals::{Step, StepDecider};
use crate::{JevDecision, JevDecisionClient, JevEngine, JevEngineConfig, JevStatus};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

struct Capture {
    seen: Mutex<Vec<Value>>,
    script: StepDecider,
}
#[async_trait]
impl JevDecisionClient for Capture {
    async fn choose(
        &self,
        page: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.seen.lock().unwrap().push(page.clone());
        self.script.choose(page, goal, history).await
    }
}

#[tokio::test]
async fn all_decision_drivers_receive_actionable_semantic_diffs_after_input() {
    let harness = harness_or_skip!();
    let mut page = harness.open("keys.html").await.unwrap();
    page.evaluate(r#"document.body.innerHTML='<nav>Static navigation</nav><main><form aria-label="Flights"><label>To<input id="to" role="combobox" aria-expanded="false"></label><button type="button" id="open">Suggestions</button><ul role="listbox" aria-label="To suggestions" id="list" hidden><li role="option">London all airports</li></ul></form></main>';document.querySelector('#open').onclick=()=>{const f=document.querySelector('#to');f.value='London';f.focus();f.setAttribute('aria-expanded','true');document.querySelector('#list').hidden=false};true"#).await.unwrap();
    let plan: Vec<Step> = serde_json::from_value(json!([{"click":"Suggestions"}])).unwrap();
    let capture = Arc::new(Capture {
        seen: Mutex::new(Vec::new()),
        script: StepDecider::new(&plan),
    });
    let config =
        JevEngineConfig::new("Show London suggestions", capture.clone()).with_wait(Duration::ZERO);
    let mut engine = JevEngine::start(Box::new(page), config).await.unwrap();
    let result = engine.run(Duration::from_secs(15)).await;
    assert_eq!(result.status, JevStatus::Done, "{result:?}");
    let seen = capture.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0]["snapshot"]["mode"], "full");
    assert_eq!(seen[1]["snapshot"]["mode"], "diff");
    let dom = seen[1]["dom_snapshot"].as_str().unwrap();
    assert!(dom.contains("= \"London\" [expanded] [focused]"), "{dom}");
    assert!(dom.contains("listbox \"To suggestions\""), "{dom}");
    assert!(!dom.contains("Static navigation"), "{dom}");
    assert!(
        !seen[1]["text"]
            .as_str()
            .unwrap()
            .contains("Static navigation")
    );
    let row = seen[1]["snapshot"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["role"] == "combobox")
        .unwrap();
    assert!(
        seen[1]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["ref"] == row["ref"])
    );
    // Both real provider adapters consume this same boundary. The native
    // request associates each rendered ref with its actual offered target.
    let (body, _, _) = crate::decide::request_body(
        &seen[1],
        "Show suggestions",
        &[],
        "jev-latest",
        chrono::Local::now().date_naive(),
    );
    assert_eq!(body["state"]["dom_snapshot"], seen[1]["dom_snapshot"]);
    assert!(
        body["questions"]["type_text_target"]["criteria"]
            .as_object()
            .unwrap()
            .values()
            .any(|a| a["ref"] == row["ref"])
    );
}
