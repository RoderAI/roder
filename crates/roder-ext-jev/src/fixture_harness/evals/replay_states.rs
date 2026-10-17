//! Capture real scripted browser states for paid request-level experiments.
//! Expected operations are metadata and never included in the model request.
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{StepDecider, TaskValues, load_tasks, run_task, validate, write_json};
use crate::{JevDecision, JevDecisionClient, JevDecisionTransport, JevTypeSafeDecisionClient};

struct Capture {
    records: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl JevDecisionTransport for Capture {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        self.records
            .lock()
            .unwrap()
            .push(json!({"request":request}));
        // Parsing the real client's request/response exercises its image path;
        // this reply never drives the browser. The scripted decision does.
        let probabilities = request["questions"]["operation"]["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .map(|key| (key.clone(), json!(if key == "DONE" { 1.0 } else { 0.0 })))
            .collect::<serde_json::Map<_, _>>();
        Ok(
            json!({"answers":{"operation":{"choice":"DONE","confidence":1.0,"probabilities":probabilities}}}),
        )
    }
}

struct ScriptedCapture {
    task: String,
    scripted: StepDecider,
    client: JevTypeSafeDecisionClient,
    records: Arc<Mutex<Vec<Value>>>,
}

impl ScriptedCapture {
    async fn select(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
        gated: bool,
    ) -> anyhow::Result<JevDecision> {
        let decision = if gated {
            let decision = self
                .scripted
                .choose_gated(observation, goal, history)
                .await?;
            self.client.choose_gated(observation, goal, history).await?;
            decision
        } else {
            let decision = self.scripted.choose(observation, goal, history).await?;
            self.client.choose(observation, goal, history).await?;
            decision
        };
        let mut records = self.records.lock().unwrap();
        let record = records.last_mut().unwrap();
        record["task"] = json!(self.task);
        record["history_len"] = json!(history.len());
        record["expected_operation"] = json!(decision.operation);
        Ok(decision)
    }
}

#[async_trait]
impl JevDecisionClient for ScriptedCapture {
    fn uses_images(&self) -> bool {
        true
    }

    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.select(observation, goal, history, false).await
    }

    async fn choose_gated(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.select(observation, goal, history, true).await
    }
}

#[tokio::test]
#[ignore = "keyless replay-state export; requires Chrome and writes image-bearing fixture requests"]
async fn export_replay_states() {
    let tasks = load_tasks().unwrap();
    validate(&tasks).unwrap();
    assert!(!tasks.is_empty());
    let base = super::super::Harness::start()
        .await
        .expect("Chrome required");
    let records = Arc::new(Mutex::new(Vec::new()));
    for task in &tasks {
        let harness = base.with_new_site().await;
        let capture = Arc::new(Capture {
            records: records.clone(),
        });
        let client = JevTypeSafeDecisionClient::with_transport("jev-1.14.0", capture)
            .with_profile(crate::jev_prompt::Profile::DomTargets)
            .with_images(true);
        let decision = Arc::new(ScriptedCapture {
            task: task.id.clone(),
            scripted: StepDecider::new(&task.script.plan),
            client,
            records: records.clone(),
        });
        let outcome = run_task(
            &harness,
            task,
            decision,
            Arc::new(TaskValues::new(&task.values)),
            task.expect.probes().cloned().collect(),
            None,
        )
        .await
        .unwrap();
        let grade = task.expect.grade(&outcome);
        assert!(grade.passed(), "{}: {:?}", task.id, grade.failures());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("replay-states-{stamp}-{}", std::process::id());
    eprintln!(
        "states: {}",
        write_json(&name, &*records.lock().unwrap())
            .unwrap()
            .display()
    );
}
