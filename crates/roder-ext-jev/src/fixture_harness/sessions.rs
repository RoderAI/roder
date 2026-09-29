//! Driving the real session layer on a test Chrome with scripted decisions:
//! the same `JevSessions::call` the tool makes, with the models and the
//! browser injected instead of resolved from Roder.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::Harness;
use crate::chrome::ChromeEndpoint;
use crate::engine::{JevDecisionClient, JevTextValueResolver};
use crate::fallback::FallbackSettings;
use crate::fallback::model::FallbackModel;
use crate::runner::{Ceilings, JevRequest};
use crate::session::{JevSessions, SessionDeps, SessionLimits, SessionModels};

/// A test's models and browser. Each call gets its own model key, so the
/// session takes the decider each call brings instead of keeping the first.
pub(crate) struct TestDeps {
    endpoint: String,
    decision: Arc<dyn JevDecisionClient>,
    text: Option<Arc<dyn JevTextValueResolver>>,
    key: String,
    /// What drives the automatic fallback; none hands it over.
    fallback: Option<Arc<dyn FallbackModel>>,
}

impl TestDeps {
    pub(crate) fn new(
        endpoint: &str,
        decision: Arc<dyn JevDecisionClient>,
        text: Option<Arc<dyn JevTextValueResolver>>,
    ) -> Self {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        Self {
            endpoint: endpoint.to_string(),
            decision,
            text,
            key: format!("test-{}", CALLS.fetch_add(1, Ordering::Relaxed)),
            fallback: None,
        }
    }

    pub(crate) fn with_fallback(mut self, model: Arc<dyn FallbackModel>) -> Self {
        self.fallback = Some(model);
        self
    }
}

#[async_trait]
impl SessionDeps for TestDeps {
    fn model_key(&self) -> String {
        self.key.clone()
    }

    async fn models(&self) -> anyhow::Result<SessionModels> {
        Ok(SessionModels {
            key: self.key.clone(),
            decision: self.decision.clone(),
            text: self.text.clone(),
            helper: None,
        })
    }

    async fn endpoint(&self) -> anyhow::Result<ChromeEndpoint> {
        Ok(ChromeEndpoint::new(self.endpoint.clone(), false))
    }

    async fn fallback_model(
        &self,
        _settings: &FallbackSettings,
        _thread: &str,
    ) -> Result<Arc<dyn FallbackModel>, String> {
        self.fallback
            .clone()
            .ok_or_else(|| "no fallback model in this test".to_string())
    }
}

/// A registry with the tool's limits and a generous idle time.
pub(crate) fn test_sessions() -> JevSessions {
    JevSessions::new(SessionLimits {
        idle: Duration::from_secs(600),
        max_sessions: 8,
        max_tabs: 3,
    })
}

/// One `jev_browse` call with these arguments on `thread`, as the tool
/// makes it, under the operator's defaults.
pub(crate) async fn call(
    harness: &Harness,
    sessions: &JevSessions,
    thread: &str,
    args: Value,
    decision: Arc<dyn JevDecisionClient>,
) -> anyhow::Result<Value> {
    call_with(harness, sessions, thread, args, decision, None).await
}

pub(crate) async fn call_with(
    harness: &Harness,
    sessions: &JevSessions,
    thread: &str,
    args: Value,
    decision: Arc<dyn JevDecisionClient>,
    text: Option<Arc<dyn JevTextValueResolver>>,
) -> anyhow::Result<Value> {
    let ceilings = Ceilings {
        refuse_cookie_banners: false,
        ..Ceilings::default()
    };
    call_under(harness, sessions, thread, args, decision, text, &ceilings).await
}

/// One call under the operator settings `ceilings`.
pub(crate) async fn call_under(
    harness: &Harness,
    sessions: &JevSessions,
    thread: &str,
    mut args: Value,
    decision: Arc<dyn JevDecisionClient>,
    text: Option<Arc<dyn JevTextValueResolver>>,
    ceilings: &Ceilings,
) -> anyhow::Result<Value> {
    if args.get("timeout_seconds").is_none() {
        args["timeout_seconds"] = json!(30);
    }
    let request = JevRequest::parse_with(&args, ceilings)?;
    let deps = TestDeps::new(harness.endpoint(), decision, text);
    sessions.call(thread, request, &deps).await
}

/// One call whose automatic fallback, if it runs, is driven by `fallback`.
pub(crate) async fn call_falling_back(
    harness: &Harness,
    sessions: &JevSessions,
    thread: &str,
    mut args: Value,
    decision: Arc<dyn JevDecisionClient>,
    fallback: Arc<dyn FallbackModel>,
    ceilings: &Ceilings,
) -> anyhow::Result<Value> {
    if args.get("timeout_seconds").is_none() {
        args["timeout_seconds"] = json!(30);
    }
    let request = JevRequest::parse_with(&args, ceilings)?;
    let deps = TestDeps::new(harness.endpoint(), decision, None).with_fallback(fallback);
    sessions.call(thread, request, &deps).await
}

/// The target ids `thread`'s session owns, oldest first.
pub(crate) fn targets(sessions: &JevSessions, thread: &str) -> Vec<String> {
    sessions
        .peek(thread)
        .map(|summary| summary.targets)
        .unwrap_or_default()
}
