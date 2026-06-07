//! Read-only provider-style retrieval loop foundation.
//!
//! This module intentionally does not call a live provider yet. It models the
//! contract agentic retrieval needs: the model sees tools, chooses calls, gets
//! structured observations, and continues until it emits a final free-form
//! response.

use async_trait::async_trait;
use roder_api::memory::MemoryScope;
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolRegistry, ToolResult, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::content_hash;
use crate::store::QueryFeedbackInput;
use crate::tools::is_read_only_tool;

#[derive(Debug, Clone, Default)]
pub struct AgenticRetrieverConfig {
    pub max_tool_calls: Option<usize>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueryFeedbackTraceMetadata {
    pub question_kind: Option<String>,
    pub eval_result_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetrievalTrace {
    pub question: String,
    pub question_kind: Option<String>,
    pub eval_result_id: Option<String>,
    pub tool_specs: Vec<ToolSpec>,
    pub observations: Vec<ToolObservation>,
    pub stop_reason: Option<String>,
    pub final_response: Option<String>,
}

impl RetrievalTrace {
    fn new(
        question: impl Into<String>,
        tool_specs: Vec<ToolSpec>,
        metadata: QueryFeedbackTraceMetadata,
    ) -> Self {
        Self {
            question: question.into(),
            question_kind: metadata.question_kind,
            eval_result_id: metadata.eval_result_id,
            tool_specs,
            observations: Vec::new(),
            stop_reason: None,
            final_response: None,
        }
    }

    pub fn to_query_feedback_input(
        &self,
        scope: Option<MemoryScope>,
        duration_ms: Option<u64>,
    ) -> QueryFeedbackInput {
        let (used_nodes, used_cards, used_events) = self.used_evidence_ids();
        QueryFeedbackInput {
            scope,
            question: self.question.clone(),
            question_kind: self.question_kind.clone(),
            used_nodes,
            used_cards,
            used_events,
            duration_ms,
            tool_call_count: self.observations.len(),
            stop_reason: self.stop_reason.clone(),
            answer_length: self.final_response.as_ref().map(|answer| answer.len()),
            response_hash: self
                .final_response
                .as_ref()
                .map(|answer| content_hash(answer)),
            eval_result_id: self.eval_result_id.clone(),
        }
    }

    pub fn used_evidence_ids(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        let mut used_nodes = Vec::new();
        let mut used_cards = Vec::new();
        let mut used_events = Vec::new();
        for observation in &self.observations {
            collect_feedback_ids(
                &observation.result.data,
                &mut used_nodes,
                &mut used_cards,
                &mut used_events,
            );
        }
        (used_nodes, used_cards, used_events)
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
        self.run_with_query_feedback_metadata(
            question,
            planner,
            QueryFeedbackTraceMetadata::default(),
        )
        .await
    }

    pub async fn run_with_query_feedback_metadata<P: ToolPlanner + Send>(
        &self,
        question: &str,
        planner: &mut P,
        metadata: QueryFeedbackTraceMetadata,
    ) -> anyhow::Result<RetrievalTrace> {
        let mut trace = RetrievalTrace::new(question, self.registry.specs(), metadata);
        let mut executed_tool_calls = 0usize;
        loop {
            match planner.next_turn(&trace).await? {
                ProviderTurn::FinalResponse(message) => {
                    trace.stop_reason = Some("final_response".to_string());
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

pub(crate) fn collect_feedback_ids(
    value: &Value,
    used_nodes: &mut Vec<String>,
    used_cards: &mut Vec<String>,
    used_events: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            if map.get("observationType").and_then(Value::as_str) == Some("raw_fact")
                && let Some(fact_id) = map
                    .get("fact")
                    .and_then(|fact| fact.get("id"))
                    .and_then(Value::as_str)
            {
                push_unique(used_nodes, fact_id);
            }
            if map.contains_key("text")
                && map.contains_key("validAt")
                && let Some(fact_id) = map.get("id").and_then(Value::as_str)
            {
                push_unique(used_nodes, fact_id);
            }
            if let Some(node_id) = map
                .get("node")
                .and_then(|node| node.get("id"))
                .and_then(Value::as_str)
            {
                push_unique(used_nodes, node_id);
            }
            for (key, child) in map {
                match key.as_str() {
                    "nodeId" | "sourceFactId" => push_string_or_array(used_nodes, child),
                    "cardId" | "evidenceCardId" => push_string_or_array(used_cards, child),
                    "eventId" | "temporalEventId" => push_string_or_array(used_events, child),
                    "evidenceIds" | "citedEvidenceIds" => {
                        for evidence_id in strings(child) {
                            classify_evidence_id(&evidence_id, used_nodes, used_cards, used_events);
                        }
                    }
                    _ => collect_feedback_ids(child, used_nodes, used_cards, used_events),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_feedback_ids(item, used_nodes, used_cards, used_events);
            }
        }
        _ => {}
    }
}

fn push_string_or_array(target: &mut Vec<String>, value: &Value) {
    for item in strings(value) {
        push_unique(target, &item);
    }
}

fn strings(value: &Value) -> Vec<String> {
    match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn classify_evidence_id(
    evidence_id: &str,
    used_nodes: &mut Vec<String>,
    used_cards: &mut Vec<String>,
    used_events: &mut Vec<String>,
) {
    let normalized = evidence_id
        .split_once(':')
        .map(|(prefix, _)| prefix)
        .unwrap_or_default();
    match normalized {
        "node" => push_unique(used_nodes, evidence_id),
        "card" | "evidence_card" => push_unique(used_cards, evidence_id),
        "event" | "temporal_event" => push_unique(used_events, evidence_id),
        _ => {}
    }
}

fn push_unique(target: &mut Vec<String>, value: &str) {
    if !target.iter().any(|existing| existing == value) {
        target.push(value.to_string());
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
