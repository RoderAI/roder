//! The model that drives the fallback: the one the session is on, or the
//! one `JEV_FALLBACK_MODEL` pins, reached through the same inference engine
//! Roder's turn uses.
//!
//! A tool is handed the calling turn's provider and model
//! (`ToolExecutionHandles::parent_model_selection`), not the engine itself,
//! so the Jev extension is given Roder's inference engines when it is
//! installed (as the subagent dispatcher is) and looks the engine up by the
//! turn's provider id. An engine that cannot take tool calls, and the
//! harnesses that run their own agent loop behind Roder's back (Claude Code,
//! Cursor), cannot drive it; then the fallback is handed over to the caller
//! instead. The turn's reasoning effort is not handed to tools, so the
//! effort is `JEV_FALLBACK_REASONING`'s (low by default).

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use roder_api::catalog::{
    PROVIDER_CLAUDE_CODE, PROVIDER_CODEX, PROVIDER_CURSOR, PROVIDER_OPENAI, lookup_model,
};
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints, TokenUsage,
    ToolCallCompleted,
};
use roder_api::tools::{ToolChoice, ToolSpec};
use roder_api::transcript::TranscriptItem;
use serde_json::{Value, json};

use super::settings::{FallbackSettings, PinnedModel};
use crate::text_model::Effort;

/// One model call's input.
pub(crate) struct Turn<'a> {
    /// This fallback's own conversation, apart from the calling turn's: a
    /// provider that keeps one connection per conversation (the Responses
    /// websocket) holds the calling turn's while it waits for the tool, so
    /// sharing it would wait forever.
    pub(crate) conversation: &'a str,
    pub(crate) instructions: &'a str,
    pub(crate) transcript: &'a [TranscriptItem],
    pub(crate) tools: &'a [ToolSpec],
    /// The page as the last look read it, for a scripted stand-in that
    /// picks its targets by label; a real model reads the transcript.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) last_page: Option<&'a Value>,
}

/// One model call's answer.
#[derive(Debug, Default)]
pub(crate) struct Reply {
    pub(crate) text: String,
    pub(crate) calls: Vec<ToolCallCompleted>,
    pub(crate) usage: Option<TokenUsage>,
    /// Provider output items (reasoning, calls) to replay with the next
    /// request, as the subagent loop keeps them.
    pub(crate) items: Vec<Value>,
}

/// What drives the fallback.
#[async_trait]
pub(crate) trait FallbackModel: Send + Sync {
    /// `provider/model (effort)`, for the result.
    fn label(&self) -> String;
    /// Whether screenshots can be shown to it.
    fn sees_images(&self) -> bool;
    async fn reply(&self, turn: Turn<'_>) -> anyhow::Result<Reply>;
}

/// A model reached through one of Roder's inference engines.
pub(crate) struct EngineModel {
    engine: Arc<dyn InferenceEngine>,
    selection: ModelSelection,
    effort: Effort,
    thread: String,
}

impl EngineModel {
    pub(crate) fn new(
        engine: Arc<dyn InferenceEngine>,
        selection: ModelSelection,
        effort: Effort,
        thread: &str,
    ) -> Self {
        Self {
            engine,
            selection,
            effort,
            thread: thread.to_string(),
        }
    }
}

#[async_trait]
impl FallbackModel for EngineModel {
    fn label(&self) -> String {
        format!(
            "{}/{} ({})",
            self.selection.provider,
            self.selection.model,
            self.effort.as_str()
        )
    }

    fn sees_images(&self) -> bool {
        self.engine.capabilities().image_input
    }

    async fn reply(&self, turn: Turn<'_>) -> anyhow::Result<Reply> {
        let request = AgentInferenceRequest {
            model: self.selection.clone(),
            instructions: InstructionBundle {
                system: Some(turn.instructions.to_string()),
                developer: None,
                developer_context: None,
            },
            transcript: turn.transcript.to_vec(),
            tools: turn.tools.to_vec(),
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig {
                enabled: self.effort != Effort::None,
                level: (self.effort != Effort::None).then(|| self.effort.as_str().to_string()),
            },
            output: OutputConfig::default(),
            runtime: RuntimeHints {
                trace_id: Some(format!("{}:jev-fallback", self.thread)),
                ..RuntimeHints::default()
            },
            metadata: json!({"jev_fallback": true, "thread": self.thread}),
        };
        let context = InferenceTurnContext {
            thread_id: turn.conversation,
            turn_id: turn.conversation,
            tool_executor: None,
        };
        let mut stream = self.engine.stream_turn(context, request).await?;
        let mut reply = Reply::default();
        while let Some(event) = stream.next().await.transpose()? {
            match event {
                InferenceEvent::MessageDelta(delta) => reply.text.push_str(&delta.text),
                InferenceEvent::ToolCallCompleted(call) => reply.calls.push(call),
                InferenceEvent::Usage(usage) => match &mut reply.usage {
                    Some(total) => total.add_assign(&usage),
                    None => reply.usage = Some(usage),
                },
                InferenceEvent::OutputItemCompleted(item) => reply.items.push(item),
                InferenceEvent::Failed(failure) => {
                    anyhow::bail!("the fallback model failed: {}", failure.message)
                }
                _ => {}
            }
        }
        Ok(reply)
    }
}

/// Providers whose engine runs an agent of its own and cannot be driven
/// with Roder's tools.
const OWN_AGENT: [&str; 2] = [PROVIDER_CLAUDE_CODE, PROVIDER_CURSOR];

/// The engine and model the fallback runs on: the pinned one, else the
/// session's. `codex` says Roder holds a ChatGPT/Codex sign-in. `Err` says
/// why there is none, for the result.
pub(crate) fn resolve(
    settings: &FallbackSettings,
    turn: Option<&ModelSelection>,
    engines: &[Arc<dyn InferenceEngine>],
    thread: &str,
    codex: bool,
) -> Result<Arc<dyn FallbackModel>, String> {
    let selection = match &settings.model {
        Some(pinned) => pinned_selection(pinned, engines, codex)?,
        None => turn
            .cloned()
            .ok_or("the session's model is not known to the tool")?,
    };
    let engine = engines
        .iter()
        .find(|engine| engine.id() == selection.provider)
        .ok_or_else(|| {
            format!(
                "no inference engine serves {}/{} here",
                selection.provider, selection.model
            )
        })?;
    if OWN_AGENT.contains(&selection.provider.as_str()) || !engine.capabilities().tool_calls {
        return Err(format!(
            "{}/{} runs its own agent or takes no tool calls, so it cannot drive the browser tools",
            selection.provider, selection.model
        ));
    }
    Ok(Arc::new(EngineModel::new(
        engine.clone(),
        selection,
        settings.effort,
        thread,
    )))
}

/// A pinned model's provider: the one it names, else its catalog
/// provider's engine; an OpenAI model goes through the ChatGPT/Codex
/// sign-in when Roder holds one, as Jev's text helper does.
fn pinned_selection(
    pinned: &PinnedModel,
    engines: &[Arc<dyn InferenceEngine>],
    codex: bool,
) -> Result<ModelSelection, String> {
    let has = |id: &str| engines.iter().any(|engine| engine.id() == id);
    let provider = match &pinned.provider {
        Some(provider) => provider.clone(),
        None => {
            let entry = lookup_model(&pinned.model).ok_or_else(|| {
                format!(
                    "JEV_FALLBACK_MODEL {} is not in Roder's catalog; name it as provider/model",
                    pinned.model
                )
            })?;
            match entry.provider {
                PROVIDER_OPENAI if codex && has(PROVIDER_CODEX) => PROVIDER_CODEX.to_string(),
                provider if has(provider) => provider.to_string(),
                provider => {
                    return Err(format!(
                        "JEV_FALLBACK_MODEL {} needs the {provider} provider, which is not \
                         configured; name it as provider/model",
                        pinned.model
                    ));
                }
            }
        }
    };
    Ok(ModelSelection {
        provider,
        model: pinned.model.clone(),
    })
}

#[cfg(test)]
mod tests {
    use roder_api::inference::{
        InferenceCapabilities, InferenceEventStream, InferenceProviderContext, ModelDescriptor,
    };

    use super::*;

    struct Engine {
        id: &'static str,
        tools: bool,
    }

    #[async_trait]
    impl InferenceEngine for Engine {
        fn id(&self) -> String {
            self.id.into()
        }
        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities {
                tool_calls: self.tools,
                ..InferenceCapabilities::text_only()
            }
        }
        async fn list_models(
            &self,
            _ctx: InferenceProviderContext<'_>,
        ) -> anyhow::Result<Vec<ModelDescriptor>> {
            Ok(Vec::new())
        }
        async fn stream_turn(
            &self,
            _ctx: InferenceTurnContext<'_>,
            _request: AgentInferenceRequest,
        ) -> anyhow::Result<InferenceEventStream> {
            anyhow::bail!("not in unit tests")
        }
    }

    fn engines() -> Vec<Arc<dyn InferenceEngine>> {
        vec![
            Arc::new(Engine {
                id: "codex",
                tools: true,
            }),
            Arc::new(Engine {
                id: "openai",
                tools: true,
            }),
            Arc::new(Engine {
                id: "claude-code",
                tools: true,
            }),
            Arc::new(Engine {
                id: "plain",
                tools: false,
            }),
        ]
    }

    fn selection(provider: &str, model: &str) -> ModelSelection {
        ModelSelection {
            provider: provider.into(),
            model: model.into(),
        }
    }

    #[test]
    fn the_sessions_model_drives_it_unless_one_is_pinned() {
        let settings = FallbackSettings::default();
        let turn = selection("openai", "gpt-6-luna");
        let model = resolve(&settings, Some(&turn), &engines(), "t", true).unwrap();
        assert_eq!(model.label(), "openai/gpt-6-luna (low)");
        let pinned = FallbackSettings {
            model: Some(PinnedModel {
                provider: None,
                model: "gpt-6-sol".into(),
            }),
            ..FallbackSettings::default()
        };
        // Bare, an OpenAI model goes through the Codex sign-in when Roder
        // holds one, else the OpenAI key.
        let model = resolve(&pinned, Some(&turn), &engines(), "t", true).unwrap();
        assert_eq!(model.label(), "codex/gpt-6-sol (low)");
        let model = resolve(&pinned, Some(&turn), &engines(), "t", false).unwrap();
        assert_eq!(model.label(), "openai/gpt-6-sol (low)");
        let named = FallbackSettings {
            model: Some(PinnedModel {
                provider: Some("openai".into()),
                model: "gpt-6-sol".into(),
            }),
            ..FallbackSettings::default()
        };
        assert_eq!(
            resolve(&named, None, &engines(), "t", true)
                .unwrap()
                .label(),
            "openai/gpt-6-sol (low)"
        );
    }

    #[test]
    fn a_model_that_cannot_drive_tools_is_refused_with_the_reason() {
        let settings = FallbackSettings::default();
        let error = |turn: Option<ModelSelection>| {
            resolve(&settings, turn.as_ref(), &engines(), "t", true)
                .err()
                .unwrap()
        };
        assert!(error(None).contains("not known"));
        assert!(error(Some(selection("claude-code", "claude-sonnet"))).contains("own agent"));
        assert!(error(Some(selection("plain", "m"))).contains("no tool calls"));
        assert!(error(Some(selection("gemini", "g"))).contains("no inference engine"));
        let unknown = FallbackSettings {
            model: Some(PinnedModel {
                provider: None,
                model: "no-such-model".into(),
            }),
            ..FallbackSettings::default()
        };
        let why = resolve(&unknown, None, &engines(), "t", true)
            .err()
            .unwrap();
        assert!(why.contains("provider/model"), "{why}");
    }
}
