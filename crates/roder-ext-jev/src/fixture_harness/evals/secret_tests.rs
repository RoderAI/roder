//! Typed secrets never leave a run: the login, one-time-code and GET-form
//! tasks run on real Chrome, and everything the run hands on (its result,
//! serialized and in full, the text the caller reads, the decision request
//! built from every page and history the decision client was sent, every
//! field context the resolver was sent, and the eval row) is searched for
//! the password and the code, as typed and in every encoding an address or
//! a JSON echo carries them.

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
        // The body the hosted decision service would be sent for this page.
        let today = chrono::Local::now().date_naive();
        let (body, _, _) = crate::decide::request_body(observation, goal, history, "jev", today);
        self.sent.push(&body);
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

/// `secret` as typed, and as a query string, `encodeURIComponent` and a
/// JSON string carry it.
fn carried(secret: &str) -> Vec<String> {
    let encoded = secret
        .bytes()
        .map(|byte| match byte {
            b if b.is_ascii_alphanumeric() || b"-_.".contains(&b) => (b as char).to_string(),
            b => format!("%{b:02X}"),
        })
        .collect::<String>();
    let quoted = serde_json::to_string(secret).unwrap();
    vec![
        secret.to_string(),
        encoded.replace("%20", "+"),
        encoded,
        quoted[1..quoted.len() - 1].to_string(),
    ]
}

#[tokio::test]
async fn typed_secrets_appear_nowhere_a_run_leaves_behind() {
    const TASKS: [&str; 3] = [
        "login_form",
        "one_time_code_resolved",
        "secret_in_get_address",
    ];
    let tasks = load_all()
        .unwrap()
        .into_iter()
        .filter(|task| TASKS.contains(&task.id.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(tasks.len(), TASKS.len());
    let base = harness_or_skip!();
    for task in &tasks {
        let secrets = task
            .values
            .iter()
            .filter(|(label, _)| ["Password", "Code"].contains(&label.as_str()))
            .flat_map(|(_, value)| carried(value))
            .collect::<Vec<_>>();
        assert!(!secrets.is_empty(), "{}", task.id);
        let harness: Harness = base.with_new_site().await;
        let sent = Arc::new(Sent::default());
        let contexts = Arc::new(Sent::default());
        let decision = Arc::new(RecordingDecider {
            inner: StepDecider::new(&task.script.plan),
            sent: sent.clone(),
        });
        let text = Arc::new(RecordingValues {
            inner: TaskValues::new(&task.values),
            sent: contexts.clone(),
        });
        let probes = task.expect.probes().cloned().collect();
        let outcome = run_task(&harness, task, decision, text, probes, None)
            .await
            .unwrap();
        let graded = task.expect.grade(&outcome);
        let row = Row::new(task, "keyless", Vec::new(), &outcome, graded);
        assert!(row.passed(), "{}: {:?}", task.id, row.failures);
        // The secret did reach the site: the grader read it in the POST, or
        // the GET form put it in the address the run ended on.
        match task.expect.posts.as_ref().map(Vec::len) {
            Some(posts) if posts > 0 => assert_eq!(outcome.posts.len(), 1, "{}", task.id),
            _ => assert!(
                outcome.result.url.contains("password=[secret]"),
                "{}",
                task.id
            ),
        }
        let mut data = serde_json::to_value(&outcome.result).unwrap();
        let everything = [
            serde_json::to_string(&outcome.result).unwrap(),
            format!("{:?}", outcome.result),
            crate::report::tool_text(&mut data),
            sent.all(),
            contexts.all(),
            serde_json::to_string(&row).unwrap(),
        ]
        .join("\n");
        for secret in &secrets {
            assert!(
                !everything.contains(secret.as_str()),
                "{}: the secret leaked as {secret:?}:\n{everything}",
                task.id
            );
        }
        // What the step and the model were told instead.
        assert!(everything.contains("[secret]"), "{}", task.id);
        assert!(sent.all().contains("[secret]"), "{}", task.id);
    }
}
