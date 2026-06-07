use roder_api::memory::MemoryScope;
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolRegistry, ToolResult};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::agent::retriever::{ModelSelectedToolCall, collect_feedback_ids};
use crate::model::content_hash;
use crate::store::{MemorySnapshotReport, QueryFeedbackInput};
use crate::tools::is_read_only_tool;

use super::AgenticToolTrace;

impl AgenticToolTrace {
    pub fn to_query_feedback_input(
        &self,
        question: &str,
        answer: &str,
        scope: Option<MemoryScope>,
        question_kind: Option<String>,
        eval_result_id: Option<String>,
        duration_ms: Option<u64>,
    ) -> QueryFeedbackInput {
        let mut used_nodes = Vec::new();
        let mut used_cards = Vec::new();
        let mut used_events = Vec::new();
        for observation in &self.tool_observations {
            collect_feedback_ids(
                &observation.result.data,
                &mut used_nodes,
                &mut used_cards,
                &mut used_events,
            );
        }
        QueryFeedbackInput {
            scope,
            question: question.to_string(),
            question_kind,
            used_nodes,
            used_cards,
            used_events,
            duration_ms,
            tool_call_count: self.tool_calls.len(),
            stop_reason: self
                .stop_reason
                .clone()
                .or_else(|| (!self.responded_via.is_empty()).then(|| self.responded_via.clone())),
            answer_length: Some(answer.len()),
            response_hash: Some(content_hash(answer)),
            eval_result_id,
        }
    }

    pub fn record_query_feedback_id(&mut self, id: impl Into<String>) {
        self.query_feedback_id = Some(id.into());
    }

    pub fn record_memory_snapshot(&mut self, snapshot: MemorySnapshotReport) {
        self.raw_snapshot_high_watermark = snapshot.raw_snapshot_high_watermark.clone();
        self.selected_dream_run_id = snapshot.selected_dream_run_id.clone();
        self.selected_ontology_version = snapshot.selected_ontology_version.clone();
        self.derived_snapshot_version = snapshot.derived_snapshot_version.clone();
        self.memory_snapshot = Some(snapshot);
    }

    pub fn record_full_trace_path(&mut self, path: impl Into<String>) {
        self.full_trace_path = Some(path.into());
    }
}

pub(super) async fn execute_agentic_tool(
    registry: &ToolRegistry,
    call: &ModelSelectedToolCall,
    tool_index: usize,
) -> anyhow::Result<ToolResult> {
    if !is_read_only_tool(&call.name) {
        return Ok(ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            text: format!(
                "agentic retrieval rejected non-read-only tool {}",
                call.name
            ),
            data: json!({
                "readOnly": false,
                "rejected": true,
                "reason": "non_read_only_tool",
                "tool": call.name,
            }),
            is_error: true,
        });
    }
    let Some(executor) = registry.get(&call.name) else {
        return Ok(ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            text: format!("tool {} is not registered", call.name),
            data: json!({
                "readOnly": true,
                "rejected": true,
                "reason": "tool_not_registered",
                "tool": call.name,
            }),
            is_error: true,
        });
    };
    executor
        .execute(
            ToolExecutionContext::new(
                "gbrain-agentic-tools",
                format!("tool-{tool_index}"),
                PolicyMode::Default,
            ),
            ToolCall {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                raw_arguments: call.arguments.to_string(),
                thread_id: "gbrain-agentic-tools".to_string(),
                turn_id: format!("tool-{tool_index}"),
            },
        )
        .await
}

pub(super) fn parse_tool_arguments(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!({ "raw": raw }))
}

pub(super) fn apply_default_scope(
    name: &str,
    mut arguments: Value,
    scope: &Option<MemoryScope>,
) -> Value {
    let Some(scope) = scope else {
        return arguments;
    };
    if !name.starts_with("gbrain_") {
        return arguments;
    }
    if let Value::Object(map) = &mut arguments {
        map.entry("scope")
            .or_insert_with(|| Value::String(scope.stable_id()));
    }
    arguments
}

pub(super) fn capture_trace_fields(
    trace: &mut AgenticToolTrace,
    call: &ModelSelectedToolCall,
    result: &ToolResult,
) {
    if call.name == "gbrain_retrieval_note" {
        trace.retrieval_notes.push(result.data.clone());
        if let Some(open) = result.data.get("openQuestions").and_then(Value::as_array) {
            for item in open {
                if let Some(text) = item.as_str()
                    && !trace.open_questions.iter().any(|existing| existing == text)
                {
                    trace.open_questions.push(text.to_string());
                }
            }
        }
    }
    if call.name == "respond_to_query" {
        trace.final_confidence = result
            .data
            .get("confidence")
            .and_then(Value::as_str)
            .map(str::to_string);
        trace.claims = result
            .data
            .get("claims")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        trace.rejected_claims = result
            .data
            .get("rejectedClaims")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        trace.unsupported_claim_count = trace
            .rejected_claims
            .iter()
            .filter(|claim| {
                claim
                    .get("confidence")
                    .or_else(|| claim.get("claimConfidence"))
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.eq_ignore_ascii_case("unsupported"))
                    || claim
                        .get("rejectionReason")
                        .or_else(|| claim.get("rejection_reason"))
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.to_ascii_lowercase().contains("unsupported"))
            })
            .count();
        trace.quote_span_coverage = quote_span_coverage(&trace.claims, &trace.rejected_claims);
        trace.citation_precision = citation_precision(trace, result);
        if let Some(open) = result.data.get("openQuestions").and_then(Value::as_array) {
            for item in open {
                if let Some(text) = item.as_str()
                    && !trace.open_questions.iter().any(|existing| existing == text)
                {
                    trace.open_questions.push(text.to_string());
                }
            }
        }
    }
}

pub(super) fn cited_evidence_ids(data: &Value) -> Vec<String> {
    data.get("citedEvidenceIds")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn quote_span_coverage(claims: &[Value], rejected_claims: &[Value]) -> Option<f32> {
    let total = claims.len() + rejected_claims.len();
    if total == 0 {
        return None;
    }
    let with_quote = claims
        .iter()
        .chain(rejected_claims)
        .filter(|claim| {
            claim
                .get("quoteSpans")
                .or_else(|| claim.get("quote_spans"))
                .and_then(Value::as_array)
                .is_some_and(|spans| !spans.is_empty())
        })
        .count();
    Some(with_quote as f32 / total as f32)
}

fn citation_precision(trace: &AgenticToolTrace, result: &ToolResult) -> Option<f32> {
    let cited = cited_evidence_ids(&result.data);
    if cited.is_empty() {
        return None;
    }
    let mut used_nodes = Vec::new();
    let mut used_cards = Vec::new();
    let mut used_events = Vec::new();
    for observation in &trace.tool_observations {
        if observation.call.name == "respond_to_query" {
            continue;
        }
        collect_feedback_ids(
            &observation.result.data,
            &mut used_nodes,
            &mut used_cards,
            &mut used_events,
        );
    }
    let matched = cited
        .iter()
        .filter(|id| {
            used_nodes.iter().any(|used| used == *id)
                || used_cards.iter().any(|used| used == *id)
                || used_events.iter().any(|used| used == *id)
        })
        .count();
    Some(matched as f32 / cited.len() as f32)
}

pub(super) fn agentic_user_prompt(
    question: &str,
    scope: Option<&MemoryScope>,
    as_of: Option<OffsetDateTime>,
) -> String {
    let mut prompt = format!("Question:\n{question}");
    if let Some(scope) = scope {
        prompt.push_str(&format!("\n\nDefault memory scope: {}", scope.stable_id()));
    }
    if let Some(as_of) = as_of {
        prompt.push_str(&format!("\nAs-of date: {}", as_of.date()));
    }
    prompt.push_str(
        "\n\nUse the available read-only gbrain tools until the answer or abstention is evidence-backed.",
    );
    prompt
}
