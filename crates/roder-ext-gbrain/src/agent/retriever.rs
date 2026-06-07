//! Read-only provider-style retrieval loop foundation.
//!
//! This module intentionally does not call a live provider yet. It models the
//! contract agentic retrieval needs: the model sees tools, chooses calls, gets
//! structured observations, and continues until it emits a final free-form
//! response.

use async_trait::async_trait;
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolRegistry, ToolResult, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tools::is_read_only_tool;

#[derive(Debug, Clone)]
pub struct AgenticRetrieverConfig {
    pub max_tool_calls: Option<usize>,
}

impl Default for AgenticRetrieverConfig {
    fn default() -> Self {
        Self {
            max_tool_calls: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelSelectedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

impl ModelSelectedToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProviderTurn {
    ToolCalls(Vec<ModelSelectedToolCall>),
    FinalResponse(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolObservation {
    pub call: ModelSelectedToolCall,
    pub result: ToolResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetrievalTrace {
    pub question: String,
    pub tool_specs: Vec<ToolSpec>,
    pub observations: Vec<ToolObservation>,
    pub final_response: Option<String>,
}

impl RetrievalTrace {
    fn new(question: impl Into<String>, tool_specs: Vec<ToolSpec>) -> Self {
        Self {
            question: question.into(),
            tool_specs,
            observations: Vec::new(),
            final_response: None,
        }
    }
}

#[async_trait]
pub trait ToolPlanner {
    async fn next_turn(&mut self, trace: &RetrievalTrace) -> anyhow::Result<ProviderTurn>;
}

pub struct AgenticRetriever {
    registry: ToolRegistry,
    config: AgenticRetrieverConfig,
}

impl AgenticRetriever {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            config: AgenticRetrieverConfig::default(),
        }
    }

    pub fn with_config(mut self, config: AgenticRetrieverConfig) -> Self {
        self.config = config;
        self
    }

    pub async fn run<P: ToolPlanner + Send>(
        &self,
        question: &str,
        planner: &mut P,
    ) -> anyhow::Result<RetrievalTrace> {
        let mut trace = RetrievalTrace::new(question, self.registry.specs());
        let mut executed_tool_calls = 0usize;
        loop {
            match planner.next_turn(&trace).await? {
                ProviderTurn::FinalResponse(message) => {
                    trace.final_response = Some(message);
                    return Ok(trace);
                }
                ProviderTurn::ToolCalls(calls) => {
                    if calls.is_empty() {
                        anyhow::bail!("provider returned an empty tool-call turn");
                    }
                    for call in calls {
                        if !is_read_only_tool(&call.name) {
                            anyhow::bail!(
                                "agentic retrieval rejected non-read-only tool {}",
                                call.name
                            );
                        }
                        executed_tool_calls += 1;
                        if self
                            .config
                            .max_tool_calls
                            .is_some_and(|max_tool_calls| executed_tool_calls > max_tool_calls)
                        {
                            anyhow::bail!(
                                "agentic retrieval exceeded max_tool_calls={}",
                                self.config.max_tool_calls.unwrap_or_default()
                            );
                        }
                        let executor = self.registry.get(&call.name).ok_or_else(|| {
                            anyhow::anyhow!("tool {} is not registered", call.name)
                        })?;
                        let tool_call = ToolCall {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            raw_arguments: call.arguments.to_string(),
                            arguments: call.arguments.clone(),
                            thread_id: "agentic-retriever".to_string(),
                            turn_id: format!("tool-{executed_tool_calls}"),
                        };
                        let result = executor
                            .execute(
                                ToolExecutionContext::new(
                                    "agentic-retriever",
                                    format!("tool-{executed_tool_calls}"),
                                    PolicyMode::Default,
                                ),
                                tool_call,
                            )
                            .await?;
                        trace.observations.push(ToolObservation { call, result });
                    }
                }
            }
        }
    }
}

pub struct FakeToolPlanner {
    turns: std::collections::VecDeque<ProviderTurn>,
}

impl FakeToolPlanner {
    pub fn new(turns: impl IntoIterator<Item = ProviderTurn>) -> Self {
        Self {
            turns: turns.into_iter().collect(),
        }
    }
}

#[async_trait]
impl ToolPlanner for FakeToolPlanner {
    async fn next_turn(&mut self, _trace: &RetrievalTrace) -> anyhow::Result<ProviderTurn> {
        self.turns
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("fake planner exhausted before final response"))
    }
}
