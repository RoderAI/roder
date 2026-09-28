//! Typed secrets never leave a run: the login and one-time-code tasks run
//! on real Chrome, and everything the run hands on (its result, serialized
//! and in full, every history the decision client was sent, every field
//! context the resolver was sent, and the eval row) is searched for the
//! password and the code.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use super::{Row, StepDecider, TaskValues, load_all, run_task};
use crate::engine::{JevDecision, JevDecisionClient, JevTextValue, JevTextValueResolver};
use crate::fixture_harness::Harness;

/// Everything a wrapped client or resolver was sent, serialized.
#[derive(Default)]
struct Sent(Mutex<Vec<String>>);

impl Sent {
    fn push(&self, value: &Value) {
        self.0.lock().unwrap().push(value.to_string());
    }

    fn all(&self) -> String {
        self.0.lock().unwrap().join("\n")
    }
}

struct RecordingDecider {
    inner: StepDecider,
    sent: Arc<Sent>,
}

#[async_trait]
impl JevDecisionClient for RecordingDecider {
    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.sent.push(&Value::Array(history.to_vec()));
        self.inner.choose(observation, goal, history).await
    }
}

struct RecordingValues {
    inner: TaskValues,
    sent: Arc<Sent>,
}

#[async_trait]
impl JevTextValueResolver for RecordingValues {
    async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue> {
        self.sent.push(field_context);
        self.inner.resolve(field_context).await
    }
}

#[tokio::test]
async fn typed_secrets_appear_nowhere_a_run_leaves_behind() {
    let tasks = load_all()
        .unwrap()
        .into_iter()
        .filter(|task| ["login_form", "one_time_code_resolved"].contains(&task.id.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(tasks.len(), 2);
    let base = harness_or_skip!();
    for task in &tasks {
        let secrets = task
            .values
            .iter()
            .filter(|(label, _)| ["Password", "Code"].contains(&label.as_str()))
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        assert_eq!(secrets.len(), 1, "{}", task.id);
        let harness: Harness = base.with_new_site().await;
        let histories = Arc::new(Sent::default());
        let contexts = Arc::new(Sent::default());
        let decision = Arc::new(RecordingDecider {
            inner: StepDecider::new(&task.script.plan),
            sent: histories.clone(),
        });
        let text = Arc::new(RecordingValues {
            inner: TaskValues::new(&task.values),
            sent: contexts.clone(),
        });
        let probes = task.expect.probes().cloned().collect();
        let outcome = run_task(&harness, task, decision, text, probes)
            .await
            .unwrap();
        let failures = task.expect.grade(&outcome);
        let row = Row::new(task, "keyless", Vec::new(), &outcome, failures);
        assert!(row.pass, "{}: {:?}", task.id, row.failures);
        // The secret did reach the page: the grader read it in the POST.
        assert_eq!(outcome.posts.len(), 1, "{}", task.id);

        let everything = [
            serde_json::to_string(&outcome.result).unwrap(),
            format!("{:?}", outcome.result),
            histories.all(),
            contexts.all(),
            serde_json::to_string(&row).unwrap(),
        ]
        .join("\n");
        for secret in &secrets {
            assert!(
                !everything.contains(secret.as_str()),
                "{}: the secret leaked:\n{everything}",
                task.id
            );
        }
        // What the step and the model were told instead.
        assert!(everything.contains("[secret]"), "{}", task.id);
        assert!(histories.all().contains("[secret]"), "{}", task.id);
    }
}
