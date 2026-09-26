use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use roder_ext_jev::{
    JevBrowser, JevDecision, JevDecisionClient, JevEngine, JevEngineConfig, JevStatus,
    JevTextValue, JevTextValueResolver,
};
use serde_json::{Map, Value, json};

#[derive(Default)]
struct BrowserState {
    observations: usize,
    executed_text: Option<String>,
}

struct HostedBrowser {
    state: Arc<Mutex<BrowserState>>,
}

#[async_trait]
impl JevBrowser for HostedBrowser {
    async fn observe(&mut self) -> anyhow::Result<Value> {
        let mut state = self.state.lock().unwrap();
        let first = state.observations == 0;
        state.observations += 1;
        Ok(if first {
            json!({
                "url": "https://store.test/checkout",
                "title": "Checkout",
                "text": "Email",
                "fingerprint": "before",
                "marker": ["checkout", 1],
                "actions": [{
                    "id": "fill-email",
                    "kind": "fill",
                    "label": "Email",
                    "role": "textbox",
                    "value": "",
                    "node": 1
                }]
            })
        } else {
            json!({
                "url": "https://store.test/checkout",
                "title": "Checkout",
                "text": "Email accepted",
                "fingerprint": "after",
                "marker": ["checkout", 2],
                "actions": []
            })
        })
    }

    async fn fresh(
        &mut self,
        _observation: &Value,
        _action: Option<&Value>,
    ) -> anyhow::Result<bool> {
        Ok(true)
    }

    async fn act(
        &mut self,
        _action: &Value,
        _observation: &Value,
        text: Option<&str>,
        _wait: Duration,
    ) -> anyhow::Result<()> {
        self.state.lock().unwrap().executed_text = text.map(str::to_string);
        Ok(())
    }
}

struct HostedDecisionClient;

#[async_trait]
impl JevDecisionClient for HostedDecisionClient {
    async fn choose(
        &self,
        _observation: &Value,
        _goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        let (choice, operation) = if history.is_empty() {
            ("fill-email", "TYPE_TEXT")
        } else {
            ("DONE", "DONE")
        };
        let mut probabilities = Map::new();
        probabilities.insert(choice.into(), json!(1.0));
        Ok(JevDecision {
            choice: choice.into(),
            operation: operation.into(),
            target: None,
            confidence: 1.0,
            probabilities,
            latency_ms: 1,
            usage: json!({}),
        })
    }
}

struct SecretReferenceResolver;

#[async_trait]
impl JevTextValueResolver for SecretReferenceResolver {
    async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue> {
        assert_eq!(field_context["field"]["label"], json!("Email"));
        Ok(JevTextValue {
            value: "verify-secret://shopper/email".into(),
            model: "fixture-resolver".into(),
            latency_ms: 1,
            usage: json!({}),
        })
    }
}

#[tokio::test]
async fn hosted_runner_can_inject_browser_decisions_and_text_values() {
    let state = Arc::new(Mutex::new(BrowserState::default()));
    let browser = HostedBrowser {
        state: Arc::clone(&state),
    };
    let config = JevEngineConfig::new("Complete checkout", Arc::new(HostedDecisionClient))
        .with_text_resolver(Arc::new(SecretReferenceResolver));

    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    let result = engine.run(Duration::from_secs(1)).await;

    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.model_calls, 2);
    assert_eq!(result.decisions.len(), 2);
    assert_eq!(result.decisions[1].operation, "DONE");
    assert_eq!(result.text_calls, 1);
    assert_eq!(result.actions.len(), 1);
    assert_eq!(result.actions[0].operation, "TYPE_TEXT");
    assert_eq!(result.actions[0].choice, "fill-email");
    assert_eq!(result.actions[0].confidence, 1.0);
    assert_eq!(
        result.actions[0].text_helper.as_deref(),
        Some("fixture-resolver")
    );
    assert_eq!(
        result.actions[0].text.as_deref(),
        Some("verify-secret://shopper/email")
    );
    assert_eq!(
        state.lock().unwrap().executed_text.as_deref(),
        Some("verify-secret://shopper/email")
    );
}
