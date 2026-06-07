//! The one-shot LLM seam for the agentic decision loop.
//!
//! The loop needs a simple `complete(system, user) -> text` primitive for its
//! synthesize / strip / verify sub-calls. Rather than re-implement provider HTTP
//! clients, the concrete [`Reasoner`] is [`EngineReasoner`](crate::infer), which
//! drives roder's own [`InferenceEngine`](roder_api::inference::InferenceEngine)
//! — so gbrain inherits every provider roder ships (`roder-ext-anthropic`,
//! `roder-ext-openai-responses`, …) plus their reasoning / retry handling. This
//! module keeps the small [`Reasoner`] trait the loop is generic over and the
//! model-name → engine factory.

use std::sync::Arc;

use roder_api::inference::InferenceEngine;
use serde_json::Value;

/// A completion plus its token usage (for cost telemetry).
#[derive(Debug, Clone, Default)]
pub struct Completion {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl Completion {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }
}

/// Minimal LLM completion seam the decision loop is generic over.
#[async_trait::async_trait]
pub trait Reasoner: Send + Sync {
    /// Single-turn completion. Returns the concatenated text (thinking blocks
    /// excluded) + token usage. Implementations should be resilient to transient
    /// errors.
    async fn complete(&self, system: &str, user: &str) -> anyhow::Result<Completion>;

    /// Label for traces / provenance (e.g. the model id).
    fn label(&self) -> String {
        "reasoner".to_string()
    }
}

/// Boxed dynamic dispatch so the CLI can pick a reasoner (Anthropic vs OpenAI)
/// at runtime by model name while [`DecisionAgent`] stays generic.
#[async_trait::async_trait]
impl Reasoner for Box<dyn Reasoner> {
    async fn complete(&self, system: &str, user: &str) -> anyhow::Result<Completion> {
        (**self).complete(system, user).await
    }
    fn label(&self) -> String {
        (**self).label()
    }
}

/// Build a reasoner from a model id by selecting the matching roder inference
/// engine: `gpt-*` / `o[1-9]*` → OpenAI **Responses** (`roder-ext-openai-responses`,
/// `OPENAI_API_KEY`); everything else → Anthropic (`roder-ext-anthropic`,
/// `ANTHROPIC_API_KEY`). Reasoning effort comes from `GBRAIN_REASONING_EFFORT`
/// (default "medium"). No provider HTTP is re-implemented here — gbrain uses
/// roder's own primitives.
pub fn build_reasoner(model: Option<String>) -> anyhow::Result<Box<dyn Reasoner>> {
    let selected = build_inference_engine(model)?;
    Ok(Box::new(crate::infer::EngineReasoner::new(
        selected.engine,
        selected.provider,
        selected.model,
        selected.reasoning_level,
    )))
}

pub struct BuiltInferenceEngine {
    pub engine: Arc<dyn InferenceEngine>,
    pub provider: String,
    pub model: String,
    pub reasoning_level: Option<String>,
}

pub fn build_inference_engine(model: Option<String>) -> anyhow::Result<BuiltInferenceEngine> {
    use roder_api::catalog::{PROVIDER_ANTHROPIC, PROVIDER_OPENAI};

    let model = model.unwrap_or_else(|| "claude-sonnet-4-6".to_string());
    let lower = model.to_ascii_lowercase();
    let is_openai = lower.starts_with("gpt")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4");
    let effort = std::env::var("GBRAIN_REASONING_EFFORT")
        .ok()
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| ["minimal", "low", "medium", "high", "xhigh", "max"].contains(&e.as_str()))
        .unwrap_or_else(|| "medium".to_string());

    if is_openai {
        let key = std::env::var("OPENAI_API_KEY").map_err(|_| {
            anyhow::anyhow!("OPENAI_API_KEY not set (required for the GPT answerer)")
        })?;
        Ok(BuiltInferenceEngine {
            engine: Arc::new(roder_ext_openai_responses::OpenAiResponsesEngine::new(key)),
            provider: PROVIDER_OPENAI.to_string(),
            model,
            reasoning_level: Some(effort),
        })
    } else {
        let key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
            anyhow::anyhow!("ANTHROPIC_API_KEY not set (required for agentic mode)")
        })?;
        Ok(BuiltInferenceEngine {
            engine: Arc::new(roder_ext_anthropic::AnthropicEngine::new(key)),
            provider: PROVIDER_ANTHROPIC.to_string(),
            model,
            reasoning_level: Some(effort),
        })
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
            extract_json(
                "```json\n[{\"text\":\"a\",\"support\":[1]},{\"text\":\"b\",\"support\":[2]}]\n```"
            ),
            Some(serde_json::json!([{"text":"a","support":[1]},{"text":"b","support":[2]}]))
        );
        assert_eq!(extract_json("no json here"), None);
    }
}
