//! Injectable JEV execution primitives.
//!
//! The built-in `jev_browse` tool connects these interfaces to Chrome and the
//! configured model providers. Hosted runners can instead keep browser
//! ownership, evidence capture, and secret handling in their supervisor while
//! reusing the same bounded JEV decision loop.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::agent::Agent;

/// A stale observation means the browser changed after JEV made its decision.
#[derive(Debug)]
pub struct StaleObservation(String);

impl StaleObservation {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for StaleObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for StaleObservation {}

/// Browser operations needed by the JEV loop.
#[async_trait]
pub trait JevBrowser: Send {
    async fn observe(&mut self) -> anyhow::Result<Value>;

    async fn fresh(&mut self, observation: &Value, action: Option<&Value>) -> anyhow::Result<bool>;

    async fn act(
        &mut self,
        action: &Value,
        observation: &Value,
        text: Option<&str>,
        wait: Duration,
    ) -> anyhow::Result<()>;

    async fn activate(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn close(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// One model decision in the JEV action space.
#[derive(Debug, Clone)]
pub struct JevDecision {
    pub choice: String,
    pub operation: String,
    pub target: Option<String>,
    pub confidence: f64,
    pub probabilities: Map<String, Value>,
    pub latency_ms: u64,
    pub usage: Value,
}

impl JevDecision {
    pub fn probability_of(&self, choice: &str) -> Value {
        self.probabilities
            .get(choice)
            .cloned()
            .unwrap_or(Value::Null)
    }
}

/// Chooses the next action from an observation produced by [`JevBrowser`].
#[async_trait]
pub trait JevDecisionClient: Send + Sync {
    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision>;
}

/// Sends a fully-formed TypeSafe request and returns its JSON response.
///
/// Hosted runners can use this seam to route decisions through an authenticated
/// control plane without moving provider credentials into the runner.
#[async_trait]
pub trait JevDecisionTransport: Send + Sync {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value>;
}

/// A value generated for a fill action.
#[derive(Debug, Clone)]
pub struct JevTextValue {
    pub value: String,
    pub model: String,
    pub latency_ms: u64,
    pub usage: Value,
}

/// Resolves a fill value from JEV's field context.
///
/// Supervisors may return an opaque value reference and resolve it only while
/// executing the browser action, so secret values never enter the JEV trace.
#[async_trait]
pub trait JevTextValueResolver: Send + Sync {
    async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JevStatus {
    Ready,
    Done,
    Blocked,
}

impl JevStatus {
    pub(crate) fn stopped(self) -> bool {
        matches!(self, Self::Done | Self::Blocked)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JevActionRecord {
    pub step: usize,
    pub action: String,
    pub kind: String,
    pub text: Option<String>,
    pub url: String,
    pub page_changed: Option<bool>,
    pub elapsed_ms: u64,
    /// Detailed planner telemetry is available to embedded hosts without
    /// expanding the compact `jev_browse` tool result.
    #[serde(skip)]
    pub choice: String,
    #[serde(skip)]
    pub probability: Value,
    #[serde(skip)]
    pub confidence: f64,
    #[serde(skip)]
    pub decision_latency_ms: u64,
    #[serde(skip)]
    pub text_helper: Option<String>,
    #[serde(skip)]
    pub text_latency_ms: u64,
    #[serde(skip)]
    pub operation: String,
    #[serde(skip)]
    pub target: Option<String>,
    #[serde(skip)]
    pub usage: Value,
    #[serde(skip)]
    pub executed_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JevDecisionRecord {
    pub choice: String,
    pub operation: String,
    pub target: Option<String>,
    pub confidence: f64,
    pub probabilities: Map<String, Value>,
    pub latency_ms: u64,
    pub usage: Value,
}

/// Typed outcome from one bounded JEV run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JevRunResult {
    pub status: JevStatus,
    pub url: String,
    pub title: String,
    pub visible_text: String,
    pub actions: Vec<JevActionRecord>,
    pub elapsed_ms: u64,
    pub observed_elements: usize,
    pub model_calls: usize,
    pub text_calls: usize,
    #[serde(skip)]
    pub decisions: Vec<JevDecisionRecord>,
    pub stopped_because: Option<String>,
    pub untrusted: bool,
}

pub struct JevEngineConfig {
    pub(crate) goal: String,
    pub(crate) decision: Arc<dyn JevDecisionClient>,
    pub(crate) text: Option<Arc<dyn JevTextValueResolver>>,
    pub(crate) wait: Duration,
}

impl JevEngineConfig {
    pub fn new(goal: impl Into<String>, decision: Arc<dyn JevDecisionClient>) -> Self {
        Self {
            goal: goal.into(),
            decision,
            text: None,
            wait: Duration::from_millis(800),
        }
    }

    pub fn with_text_resolver(mut self, text: Arc<dyn JevTextValueResolver>) -> Self {
        self.text = Some(text);
        self
    }

    pub fn with_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }
}

/// The reusable, bounded JEV agent loop.
pub struct JevEngine {
    agent: Agent,
}

impl JevEngine {
    pub async fn start(
        browser: Box<dyn JevBrowser>,
        config: JevEngineConfig,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            agent: Agent::start(browser, config).await?,
        })
    }

    pub async fn run(&mut self, timeout: Duration) -> JevRunResult {
        let stopped_because = match tokio::time::timeout(timeout, self.agent.run()).await {
            Ok(stopped) => stopped,
            Err(_) => Some("Jev browser task timed out".into()),
        };
        self.agent.result(stopped_because)
    }

    pub async fn activate(&mut self) -> anyhow::Result<()> {
        self.agent.activate().await
    }

    pub async fn close(&mut self) -> anyhow::Result<()> {
        self.agent.close().await
    }
}
