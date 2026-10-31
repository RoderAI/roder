mod support;
use async_trait::async_trait;
use roder_ext_jev::{
    JevBilled, JevDecision, JevDecisionClient, JevDecisionTransport, JevEngine, JevEngineConfig,
    JevStatus,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use support::{ScriptedBrowser, labelled_page};

#[derive(Default)]
struct Provider(Mutex<usize>);
#[async_trait]
impl JevDecisionTransport for Provider {
    async fn decide(&self, _: &Value) -> anyhow::Result<Value> {
        *self.0.lock().unwrap() += 1;
        Ok(json!({"usage":{"input_tokens":7,"output_tokens":0}}))
    }
}
struct CustomClient {
    provider: Arc<Provider>,
    components: usize,
}
#[async_trait]
impl JevDecisionClient for CustomClient {
    async fn choose(&self, _: &Value, _: &str, _: &[Value]) -> anyhow::Result<JevDecision> {
        for component in 0..self.components {
            self.provider
                .decide_budgeted(&json!({"component":component}))
                .await
                .map_err(|error| {
                    JevBilled::new(
                        json!({"input_tokens":component * 7,"output_tokens":0}),
                        error,
                    )
                })?;
        }
        Ok(JevDecision {
            choice: "DONE".into(),
            operation: "DONE".into(),
            target: None,
            confidence: 1.0,
            target_confidence: None,
            probabilities: Default::default(),
            latency_ms: 0,
            usage: json!({"input_tokens":self.components * 7,"output_tokens":0}),
            model: None,
            irreversible: None,
            downloads_forbidden: None,
        })
    }
}
async fn run(components: usize) -> (roder_ext_jev::JevRunResult, usize) {
    let provider = Arc::new(Provider::default());
    let client = Arc::new(CustomClient {
        provider: provider.clone(),
        components,
    });
    let config = JevEngineConfig::new("Read the current page", client).with_max_actions(1);
    let browser = ScriptedBrowser::new(vec![labelled_page("one", &[])]);
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    let result = engine.run(Duration::from_secs(5)).await;
    let calls = *provider.0.lock().unwrap();
    (result, calls)
}
#[tokio::test]
async fn custom_component_calls_are_counted_without_usage_calls() {
    let (result, calls) = run(2).await;
    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(calls, 2);
    assert_eq!(result.model_calls, 2);
    assert_eq!(result.usage.decision.calls, 2);
    assert_eq!(
        result.usage.decision.input_tokens,
        roder_ext_jev::JevTokenCount::Known(14)
    );
}
#[tokio::test]
async fn custom_component_request_is_denied_before_provider_contact() {
    let (result, calls) = run(3).await;
    assert_eq!(result.status, JevStatus::BudgetExceeded);
    assert_eq!(calls, 2);
    assert_eq!(result.model_calls, 2);
    assert_eq!(result.usage.decision.calls, 2);
    assert_eq!(
        result.usage.decision.input_tokens,
        roder_ext_jev::JevTokenCount::Known(14)
    );
}
