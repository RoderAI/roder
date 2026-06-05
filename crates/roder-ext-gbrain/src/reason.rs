//! The LLM seam for the agentic decision loop.
//!
//! The loop needs a simple `complete(system, user) -> text` primitive for its
//! decompose / draft / verify / synthesize sub-calls. Roder's canonical
//! [`InferenceEngine`](roder_api::inference::InferenceEngine) is a streaming,
//! tool-oriented `stream_turn` API that is awkward for these structured one-shot
//! calls — and is unavailable to the standalone `roder-gbrain` CLI / the eval
//! harness, which have no extension registry. So the loop is generic over this
//! small [`Reasoner`] trait.
//!
//! v1 ships [`AnthropicReasoner`] (used by the CLI + OrgMemBench eval). An
//! `InferenceEngineReasoner` that wraps a registry engine is the documented
//! runtime hook (see `agent.rs` notes) — not built in v1 to keep scope tight.

use std::time::Duration;

use serde_json::Value;

/// Minimal LLM completion seam the decision loop is generic over.
#[async_trait::async_trait]
pub trait Reasoner: Send + Sync {
    /// Single-turn completion. Returns the concatenated text (thinking blocks
    /// excluded). Implementations should be resilient to transient errors.
    async fn complete(&self, system: &str, user: &str) -> anyhow::Result<String>;

    /// Label for traces / provenance (e.g. the model id).
    fn label(&self) -> String {
        "reasoner".to_string()
    }
}

/// Pull the first JSON value (object or array) out of a model response, tolerant
/// of code fences and surrounding prose.
pub fn extract_json(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        return Some(v);
    }
    // Scan from the EARLIEST opening bracket (so `[{...}]` is read as the array,
    // not the inner object), tracking string/escape state for a balanced span.
    let first_obj = text.find('{');
    let first_arr = text.find('[');
    let (start, open, close) = match (first_obj, first_arr) {
        (Some(o), Some(a)) if o < a => (o, b'{', b'}'),
        (_, Some(a)) => (a, b'[', b']'),
        (Some(o), None) => (o, b'{', b'}'),
        (None, None) => return None,
    };
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str::<Value>(&text[start..=i]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

/// Anthropic Messages reasoner used by the CLI + eval. Self-contained (no
/// extension registry) so it runs in the standalone binary.
pub struct AnthropicReasoner {
    client: reqwest::Client,
    api_key: String,
    model: String,
    max_tokens: u32,
    thinking_budget: u32,
    max_retries: u32,
}

impl AnthropicReasoner {
    pub const DEFAULT_MODEL: &'static str = "claude-sonnet-4-6";

    /// Construct from `ANTHROPIC_API_KEY`. `model` defaults to Sonnet 4.6.
    pub fn from_env(model: Option<String>) -> anyhow::Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set (required for agentic mode)"))?;
        Ok(Self {
            client: reqwest::Client::new(),
            api_key,
            model: model.unwrap_or_else(|| Self::DEFAULT_MODEL.to_string()),
            max_tokens: 8000,
            thinking_budget: 2000,
            max_retries: 6,
        })
    }

    pub fn with_thinking_budget(mut self, budget: u32) -> Self {
        self.thinking_budget = budget;
        self
    }

    fn body(&self, system: &str, user: &str) -> Value {
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": self.max_tokens.max(self.thinking_budget + 1024),
            "system": system,
            "messages": [{ "role": "user", "content": user }],
        });
        // Opus 4.x uses adaptive thinking + output_config.effort; others use the
        // enabled+budget form (temperature must be 1 with enabled thinking).
        if self.model.contains("opus-4-8") || self.model.contains("opus-4.8") {
            body["thinking"] = serde_json::json!({ "type": "adaptive" });
            body["output_config"] = serde_json::json!({ "effort": "high" });
        } else if self.thinking_budget > 0 {
            body["temperature"] = serde_json::json!(1);
            body["thinking"] =
                serde_json::json!({ "type": "enabled", "budget_tokens": self.thinking_budget });
        }
        body
    }
}

#[async_trait::async_trait]
impl Reasoner for AnthropicReasoner {
    fn label(&self) -> String {
        self.model.clone()
    }

    async fn complete(&self, system: &str, user: &str) -> anyhow::Result<String> {
        let body = self.body(system, user);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let resp = self
                .client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let json: Value = r.json().await?;
                    let text = json
                        .get("content")
                        .and_then(Value::as_array)
                        .map(|blocks| {
                            blocks
                                .iter()
                                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                                .filter_map(|b| b.get("text").and_then(Value::as_str))
                                .collect::<Vec<_>>()
                                .join("")
                        })
                        .unwrap_or_default();
                    return Ok(text);
                }
                Ok(r) => {
                    let status = r.status();
                    // Retry transient overloads/server errors/rate limits.
                    let retryable = status.as_u16() == 429 || status.is_server_error();
                    if retryable && attempt <= self.max_retries {
                        backoff(attempt).await;
                        continue;
                    }
                    let detail = r.text().await.unwrap_or_default();
                    anyhow::bail!("anthropic {status}: {}", detail.chars().take(300).collect::<String>());
                }
                Err(err) if attempt <= self.max_retries => {
                    let _ = err;
                    backoff(attempt).await;
                    continue;
                }
                Err(err) => return Err(err.into()),
            }
        }
    }
}

async fn backoff(attempt: u32) {
    let secs = (2u64.pow(attempt.min(5))).min(30);
    tokio::time::sleep(Duration::from_secs(secs)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_handles_fences_and_prose() {
        assert_eq!(extract_json("[1,2,3]"), Some(serde_json::json!([1, 2, 3])));
        assert_eq!(
            extract_json("here you go:\n```json\n{\"a\": 1}\n```\nthanks"),
            Some(serde_json::json!({"a": 1}))
        );
        assert_eq!(
            extract_json("prose {\"nested\": {\"b\": [true]}} trailing"),
            Some(serde_json::json!({"nested": {"b": [true]}}))
        );
        // Array-of-objects must parse as the array, not the first inner object.
        assert_eq!(
            extract_json("```json\n[{\"text\":\"a\",\"support\":[1]},{\"text\":\"b\",\"support\":[2]}]\n```"),
            Some(serde_json::json!([{"text":"a","support":[1]},{"text":"b","support":[2]}]))
        );
        assert_eq!(extract_json("no json here"), None);
    }
}
