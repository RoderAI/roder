//! The real [`Page`], with DOM probes read just before the engine closes it.
//!
//! The engine owns its browser, so a grader cannot reach the page after a
//! run. Reading the probes inside `close` sees the final document without
//! adding anything to the loop.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::engine::{JevActOutcome, JevBrowser};
use crate::page::Page;

/// What the probes read, keyed by expression.
#[derive(Debug, Default)]
pub(crate) struct Probed {
    pub(crate) dom: BTreeMap<String, Result<Value, String>>,
}

/// Hands the probe results back once the engine has closed the page.
pub(crate) struct ProbeHandle(Arc<Mutex<Probed>>);

impl ProbeHandle {
    pub(crate) fn take(self) -> Probed {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

pub(crate) struct ProbedPage {
    page: Page,
    probes: Vec<String>,
    out: Arc<Mutex<Probed>>,
}

impl ProbedPage {
    pub(crate) fn new(page: Page, probes: Vec<String>) -> (Self, ProbeHandle) {
        let out = Arc::new(Mutex::new(Probed::default()));
        (
            Self {
                page,
                probes,
                out: out.clone(),
            },
            ProbeHandle(out),
        )
    }
}

#[async_trait]
impl JevBrowser for ProbedPage {
    async fn observe(&mut self) -> anyhow::Result<Value> {
        let observation = self.page.observe().await?;
        // For diagnosing a task: every observation's actions and text.
        if std::env::var("JEV_EVAL_DUMP").as_deref() == Ok("1") {
            let actions = observation["actions"]
                .as_array()
                .map_or(Vec::new(), |actions| {
                    actions
                        .iter()
                        .map(|action| {
                            let mut action = action.clone();
                            if let Some(action) = action.as_object_mut() {
                                action.remove("rect");
                            }
                            action.to_string()
                        })
                        .collect()
                });
            eprintln!(
                "--- observation {}\n{}\ntext: {}",
                observation["url"],
                actions.join("\n"),
                observation["text"]
            );
        }
        Ok(observation)
    }

    async fn fresh(&mut self, observation: &Value, action: Option<&Value>) -> anyhow::Result<bool> {
        self.page.fresh(observation, action).await
    }

    async fn describe(&mut self) -> anyhow::Result<Option<crate::engine::JevPageFacts>> {
        self.page.describe().await
    }

    async fn act(
        &mut self,
        action: &Value,
        observation: &Value,
        text: Option<&str>,
        wait: Duration,
    ) -> anyhow::Result<JevActOutcome> {
        self.page.act(action, observation, text, wait).await
    }

    async fn refuse_cookie_banner(&mut self) -> anyhow::Result<Option<String>> {
        self.page.refuse_cookie_banner().await
    }

    async fn close(&mut self) -> anyhow::Result<()> {
        // Let a last click's effects land before reading them.
        self.page.settle().await;
        for expression in &self.probes {
            let value = self
                .page
                .evaluate(expression)
                .await
                .map_err(|error| format!("{error:#}"));
            self.out
                .lock()
                .unwrap()
                .dom
                .insert(expression.clone(), value);
        }
        self.page.close().await
    }
}

/// Read `probes` from `target`'s document now, over a connection of its own
/// (the engine still holds the page): Jev's end state before a fallback.
pub(crate) async fn read(endpoint: &str, target: &str, probes: &[String]) -> Probed {
    let mut probed = Probed::default();
    let read = async {
        let mut connection = crate::cdp::Connection::connect(endpoint).await?;
        let attached = connection
            .call(
                "Target.attachToTarget",
                serde_json::json!({"targetId": target, "flatten": true}),
                None,
            )
            .await?;
        let session = attached["sessionId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let mut values = BTreeMap::new();
        for expression in probes {
            let value = connection
                .call(
                    "Runtime.evaluate",
                    serde_json::json!({"expression": expression, "returnByValue": true}),
                    Some(&session),
                )
                .await
                .map(|reply| reply["result"]["value"].clone())
                .map_err(|error| format!("{error:#}"));
            values.insert(expression.clone(), value);
        }
        anyhow::Ok(values)
    };
    match read.await {
        Ok(values) => probed.dom = values,
        Err(error) => {
            for expression in probes {
                probed
                    .dom
                    .insert(expression.clone(), Err(format!("{error:#}")));
            }
        }
    }
    probed
}
