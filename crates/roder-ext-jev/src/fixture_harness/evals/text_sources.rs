//! Where the live tiers' typed values come from, and what each call cost.
//!
//! - [`Supervised`]: a supervisor's value resolver for the secrets a task
//!   holds, the text model for everything else, as a host that keeps
//!   passwords and one-time codes out of the goal would run Jev.
//! - [`Recorded`]: any resolver, with each call's latency, token usage and
//!   outcome kept for the eval row. Values are never recorded.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::TaskValues;
use crate::engine::{JevStop, JevTextValue, JevTextValueResolver};
use crate::usage::{JevBilled, JevCallUsage};

/// The task's own values for its secret fields, when it holds one for the
/// field; the text model for every other field, and for a secret the task
/// does not hold (which the text helper refuses unless the goal gives it).
pub(crate) struct Supervised {
    secrets: TaskValues,
    labels: Vec<String>,
    text: Arc<dyn JevTextValueResolver>,
}

impl Supervised {
    pub(crate) fn new(
        values: &std::collections::BTreeMap<String, String>,
        text: Arc<dyn JevTextValueResolver>,
    ) -> Self {
        Self {
            secrets: TaskValues::new(values),
            labels: values.keys().cloned().collect(),
            text,
        }
    }
}

#[async_trait]
impl JevTextValueResolver for Supervised {
    async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue> {
        let field = &field_context["field"];
        let label = field["label"].as_str().unwrap_or_default();
        if crate::secret::is_secret(field) && self.labels.iter().any(|known| known == label) {
            return self.secrets.resolve(field_context).await;
        }
        self.text.resolve(field_context).await
    }
}

/// One resolver call as the row keeps it.
#[derive(Debug, Clone)]
struct Call {
    latency_ms: u64,
    status: &'static str,
    usage: Value,
    source: String,
}

/// A resolver whose calls are kept for the row.
pub(crate) struct Recorded {
    inner: Arc<dyn JevTextValueResolver>,
    /// What answered a call that failed: `text-model` or `task-values`.
    failing_source: &'static str,
    calls: Mutex<Vec<Call>>,
}

impl Recorded {
    pub(crate) fn new(
        inner: Arc<dyn JevTextValueResolver>,
        failing_source: &'static str,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner,
            failing_source,
            calls: Mutex::new(Vec::new()),
        })
    }

    /// The calls' latencies, summed usage and outcomes, for a row. A call
    /// the task's own values answered is listed but costs nothing.
    pub(crate) fn report(&self) -> Value {
        let calls = self.calls.lock().unwrap().clone();
        let model_calls = calls
            .iter()
            .filter(|call| call.source != "task-values")
            .collect::<Vec<_>>();
        json!({
            "latency_ms": model_calls.iter().map(|call| call.latency_ms).collect::<Vec<_>>(),
            "usage": JevCallUsage::sum(model_calls.iter().map(|call| &call.usage)),
            "calls": calls.iter().map(|call| json!({
                "source": call.source,
                "status": call.status,
                "latency_ms": call.latency_ms,
            })).collect::<Vec<_>>(),
        })
    }
}

#[async_trait]
impl JevTextValueResolver for Recorded {
    async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue> {
        let started = Instant::now();
        let result = self.inner.resolve(field_context).await;
        let latency_ms = started.elapsed().as_millis() as u64;
        let call = match &result {
            Ok(written) => Call {
                latency_ms,
                status: "ok",
                usage: written.usage.clone(),
                source: written.model.clone(),
            },
            Err(error) => Call {
                latency_ms,
                status: match JevStop::status_of(error) {
                    crate::engine::JevStatus::NeedsInput => "needs_input",
                    crate::engine::JevStatus::Unavailable => "unavailable",
                    _ => "error",
                },
                usage: JevBilled::usage_of(error).cloned().unwrap_or(Value::Null),
                source: self.failing_source.into(),
            },
        };
        self.calls.lock().unwrap().push(call);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Model;

    #[async_trait]
    impl JevTextValueResolver for Model {
        async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue> {
            Ok(JevTextValue {
                value: format!(
                    "model:{}",
                    field_context["field"]["label"].as_str().unwrap()
                ),
                model: "fake-model".into(),
                latency_ms: 3,
                usage: json!({"input_tokens": 10, "output_tokens": 2}),
            })
        }
    }

    fn field(label: &str, input_type: &str) -> Value {
        json!({"goal": "g", "field": {"label": label, "input_type": input_type}})
    }

    #[tokio::test]
    async fn the_supervisor_answers_only_the_secrets_it_holds() {
        let values = BTreeMap::from([
            ("Username".to_string(), "ada".to_string()),
            ("Password".to_string(), "pw-1234".to_string()),
        ]);
        let resolver = Recorded::new(
            Arc::new(Supervised::new(&values, Arc::new(Model))),
            "text-model",
        );
        let value = |written: JevTextValue| written.value;
        assert_eq!(
            value(
                resolver
                    .resolve(&field("Password", "password"))
                    .await
                    .unwrap()
            ),
            "pw-1234"
        );
        // An ordinary field comes from the model even when the task lists it.
        assert_eq!(
            value(resolver.resolve(&field("Username", "text")).await.unwrap()),
            "model:Username"
        );
        // A secret the task does not hold goes to the model, which enforces
        // that the goal gives it.
        assert_eq!(
            value(
                resolver
                    .resolve(&field("Code", "one-time-code"))
                    .await
                    .unwrap()
            ),
            "model:Code"
        );
        let report = resolver.report();
        assert_eq!(report["latency_ms"].as_array().unwrap().len(), 2);
        assert_eq!(report["usage"]["input_tokens"], json!(20));
        assert_eq!(report["calls"][0]["source"], json!("task-values"));
        assert!(!report.to_string().contains("pw-1234"));
    }
}
