//! Secrets never reach the reported result: neither the text model's key
//! nor a password or one-time code Jev typed, even on a page that shows
//! what was typed back (in its text, its address, a field's label or a
//! dialog).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::annotate;
use crate::chrome::ChromeEndpoint;
use crate::engine::{
    JevActOutcome, JevBrowser, JevDecision, JevDecisionClient, JevEngine, JevEngineConfig,
    JevTextValue, JevTextValueResolver,
};
use crate::text_model::{TextModel, Transport};

const PASSWORD: &str = "hunter2-correct-horse";
const CODE: &str = "482913";

/// A sign-in page that echoes everything typed into it everywhere it can.
struct EchoingPage {
    typed: Arc<Mutex<Vec<String>>>,
}

impl EchoingPage {
    fn page(&self) -> Value {
        let typed = self.typed.lock().unwrap().join(" ");
        let field = |id: &str, node: u64, label: &str, input_type: &str| {
            json!({"id": id, "node": node, "kind": "fill", "role": "textbox", "value": "",
                   "label": label, "input_type": input_type, "filled": !typed.is_empty()})
        };
        let mut page = json!({
            "url": format!("https://login.test/?echo={typed}"),
            "title": format!("Signed in {typed}"),
            "text": format!("You typed {typed}"),
            "fingerprint": format!("f{}", typed.len()),
            "marker": [typed],
            "actions": [
                field("e1", 1, "Password", "password"),
                field("e2", 2, "Code", "one-time-code"),
                {"id": "e3", "node": 3, "kind": "click", "role": "button", "value": "",
                 "label": format!("Sign in as {typed}")},
            ],
        });
        if !typed.is_empty() {
            page["dialogs"] = json!([{"type": "alert", "message": typed, "accepted": true}]);
        }
        page
    }
}

#[async_trait]
impl JevBrowser for EchoingPage {
    async fn observe(&mut self) -> anyhow::Result<Value> {
        Ok(self.page())
    }

    async fn fresh(&mut self, _: &Value, _: Option<&Value>) -> anyhow::Result<bool> {
        Ok(true)
    }

    async fn act(
        &mut self,
        action: &Value,
        _: &Value,
        text: Option<&str>,
        _: Duration,
    ) -> anyhow::Result<JevActOutcome> {
        let Some(text) = text else {
            return Ok(JevActOutcome::done());
        };
        self.typed.lock().unwrap().push(text.to_string());
        // A field that refuses and quotes what it shows.
        Ok(match action["label"] == "Code" {
            true => JevActOutcome::refused(format!("The field shows {text:?}")),
            false => JevActOutcome::done(),
        })
    }
}

/// Types the password, then the code, then answers DONE; keeps every
/// history it was sent.
struct Plan(Mutex<Vec<String>>);

#[async_trait]
impl JevDecisionClient for Plan {
    async fn choose(&self, _: &Value, _: &str, history: &[Value]) -> anyhow::Result<JevDecision> {
        self.0
            .lock()
            .unwrap()
            .push(Value::Array(history.to_vec()).to_string());
        let choice = ["e1", "e2"].get(history.len()).unwrap_or(&"DONE");
        let mut probabilities = Map::new();
        probabilities.insert(choice.to_string(), json!(1.0));
        Ok(JevDecision {
            choice: choice.to_string(),
            operation: if *choice == "DONE" {
                "DONE"
            } else {
                "TYPE_TEXT"
            }
            .into(),
            target: None,
            confidence: 1.0,
            target_confidence: None,
            probabilities,
            latency_ms: 0,
            usage: json!({}),
            model: None,
            irreversible: None,
        })
    }
}

/// The supervisor's resolver: the password and the code by field.
struct Vault(Mutex<Vec<String>>);

#[async_trait]
impl JevTextValueResolver for Vault {
    async fn resolve(&self, context: &Value) -> anyhow::Result<JevTextValue> {
        self.0.lock().unwrap().push(context.to_string());
        let value = match context["field"]["label"].as_str() {
            Some("Code") => CODE,
            _ => PASSWORD,
        };
        Ok(JevTextValue {
            value: value.into(),
            model: "vault".into(),
            latency_ms: 0,
            usage: json!({}),
        })
    }
}

#[tokio::test]
async fn secrets_never_reach_the_reported_result() {
    // The port makes its own HTTP calls, so a key can only leak through the
    // annotated result; it carries the model name, never the key.
    let mut value = json!({"status":"done"});
    let text = TextModel {
        model: "deepseek-chat".into(),
        source: "roder-provider",
        note: None,
        fallback: None,
        transport: Transport::Chat {
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-secret-value".into(),
            reasoning: None,
        },
    };
    let endpoint = ChromeEndpoint::new("http://127.0.0.1:9222", false);
    annotate(&mut value, Some(&endpoint), Some(&text), true);
    assert!(!value.to_string().contains("sk-secret-value"));

    // Nor does a password or one-time code Jev typed, however the page
    // shows it back.
    let typed = Arc::new(Mutex::new(Vec::new()));
    let plan = Arc::new(Plan(Mutex::default()));
    let vault = Arc::new(Vault(Mutex::default()));
    let config = JevEngineConfig::new("Sign in", plan.clone())
        .with_text_resolver(vault.clone())
        .with_wait(Duration::ZERO);
    let browser = EchoingPage {
        typed: typed.clone(),
    };
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    let result = engine.run(Duration::from_secs(5)).await;
    // Both were typed.
    assert_eq!(*typed.lock().unwrap(), [PASSWORD, CODE]);
    assert_eq!(result.actions[0].text.as_deref(), Some("[secret]"));
    assert_eq!(result.actions[1].text.as_deref(), Some("[secret]"));
    assert_eq!(
        result.actions[1].refused.as_deref(),
        Some("The field shows \"[secret]\"")
    );

    let mut value = serde_json::to_value(&result).unwrap();
    annotate(&mut value, Some(&endpoint), Some(&text), true);
    let everything = [
        value.to_string(),
        format!("{result:?}"),
        plan.0.lock().unwrap().join("\n"),
        vault.0.lock().unwrap().join("\n"),
    ]
    .join("\n");
    for secret in [PASSWORD, CODE, "sk-secret-value"] {
        assert!(
            !everything.contains(secret),
            "{secret} leaked:\n{everything}"
        );
    }
    assert!(result.visible_text.contains("You typed [secret] [secret]"));
    assert!(
        result.url.ends_with("echo=[secret] [secret]"),
        "{}",
        result.url
    );
}
