//! `gbrain_*` agent tools over [`GbrainStore`]'s bi-temporal API.

use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use roder_api::memory::MemoryScope;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::model::{AsOf, TemporalFact, format_time, parse_flexible};
use crate::render::render_recall;
use crate::response_format::{self, ResponseFormat};
use crate::store::{CaptureInput, GbrainStore, RecallParams, RecallResult};

pub struct GbrainToolContributor {
    store: Arc<GbrainStore>,
}

impl GbrainToolContributor {
    pub fn new(store: Arc<GbrainStore>) -> Self {
        Self { store }
    }
}

impl ToolContributor for GbrainToolContributor {
    fn id(&self) -> ToolProviderId {
        "gbrain-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        for op in [
            Op::Capture,
            Op::Recall,
            Op::AsOf,
            Op::Supersede,
            Op::History,
            Op::Contradictions,
            Op::Consolidate,
        ] {
            registry.register(Arc::new(GbrainTool {
                store: self.store.clone(),
                op,
            }))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum Op {
    Capture,
    Recall,
    AsOf,
    Supersede,
    History,
    Contradictions,
    Consolidate,
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::Capture => "gbrain_capture",
            Op::Recall => "gbrain_recall",
            Op::AsOf => "gbrain_as_of",
            Op::Supersede => "gbrain_supersede",
            Op::History => "gbrain_history",
            Op::Contradictions => "gbrain_contradictions",
            Op::Consolidate => "gbrain_consolidate",
        }
    }
}

struct GbrainTool {
    store: Arc<GbrainStore>,
    op: Op,
}

#[async_trait::async_trait]
impl ToolExecutor for GbrainTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.op.name().to_string(),
            description: description(self.op).to_string(),
            parameters: schema(self.op),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let result = match self.op {
            Op::Capture => self.capture(&call).await,
            Op::Recall => self.recall(&call, false).await,
            Op::AsOf => self.recall(&call, true).await,
            Op::Supersede => self.supersede(&call).await,
            Op::History => self.history(&call).await,
            Op::Contradictions => self.contradictions(&call).await,
            Op::Consolidate => self.consolidate(&call).await,
        };
        match result {
            Ok((text, data)) => Ok(ToolResult {
                id: call.id,
                name: call.name,
                text,
                data,
                is_error: false,
            }),
            Err(err) => Ok(ToolResult {
                id: call.id,
                name: call.name,
                text: format!("gbrain error: {err}"),
                data: json!({ "error": err.to_string() }),
                is_error: true,
            }),
        }
    }
}

impl GbrainTool {
    async fn capture(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: CaptureArgs = parse(call)?;
        let mut input = CaptureInput::new(
            args.scope.as_deref().map(parse_scope).unwrap_or(MemoryScope::Global),
            args.text,
        );
        input.subject = args.subject;
        input.metadata = args.metadata;
        input.provenance = args.provenance;
        input.valid_at = parse_opt(args.valid_at.as_deref())?;
        input.invalid_at = parse_opt(args.invalid_at.as_deref())?;
        input.ingested_at = parse_opt(args.ingested_at.as_deref())?;
        let fact = self.store.capture(input).await?;
        Ok((
            format!("captured fact {} (valid {})", fact.id, fact.valid_at.date()),
            json!({ "id": fact.id, "fact": fact }),
        ))
    }

    async fn recall(&self, call: &ToolCall, as_of_tool: bool) -> anyhow::Result<(String, Value)> {
        let args: RecallArgs = parse(call)?;
        let as_of = if as_of_tool {
            let date = args
                .date
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("gbrain_as_of requires `date`"))?;
            AsOf::at(parse_flexible(date)?)
        } else if let Some(date) = args.as_of.as_deref() {
            AsOf::at(parse_flexible(date)?)
        } else {
            AsOf {
                transaction_time: parse_opt(args.transaction_time.as_deref())?,
                valid_time: parse_opt(args.valid_time.as_deref())?,
            }
        };
        let result = self
            .store
            .recall(RecallParams {
                query: args.query.unwrap_or_default(),
                as_of,
                scope: args.scope.as_deref().map(parse_scope),
                include_global: args.include_global.unwrap_or(true),
                limit: args.limit.unwrap_or(10),
                expand: false,
            })
            .await?;
        Ok(render_result(&result, args.response_format))
    }

    async fn supersede(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: SupersedeArgs = parse(call)?;
        let valid_at = parse_opt(args.valid_at.as_deref())?;
        let fact = self
            .store
            .supersede(&args.old_id, args.text, args.reason, valid_at)
            .await?;
        Ok((
            format!("superseded {} with {}", args.old_id, fact.id),
            json!({ "id": fact.id, "supersedes": args.old_id, "fact": fact }),
        ))
    }

    async fn history(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: HistoryArgs = parse(call)?;
        let facts = self
            .store
            .history(
                args.id.as_deref(),
                args.subject.as_deref(),
                args.scope.as_deref().map(parse_scope),
            )
            .await?;
        let now = OffsetDateTime::now_utc();
        let mut text = format!("{} version(s) in timeline:", facts.len());
        for fact in &facts {
            text.push_str(&format!(
                "\n- [{}] {} (valid {})",
                crate::store::status_label(fact, now),
                fact.text.trim(),
                fact.valid_at.date()
            ));
        }
        Ok((
            args.response_format.bound(&text),
            json!({ "facts": facts.iter().map(|f| fact_json(f, None, now)).collect::<Vec<_>>() }),
        ))
    }

    async fn contradictions(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: ContradictionsArgs = parse(call)?;
        let pairs = self
            .store
            .contradictions(
                args.scope.as_deref().map(parse_scope),
                args.subject.as_deref(),
                args.limit.unwrap_or(20),
            )
            .await?;
        let mut text = format!("{} contradiction(s):", pairs.len());
        let mut data = Vec::new();
        for pair in &pairs {
            text.push_str(&format!(
                "\n- \"{}\" ⟂ \"{}\"",
                pair.a.text.trim(),
                pair.b.text.trim()
            ));
            data.push(json!({ "a": pair.a.id, "b": pair.b.id }));
        }
        Ok((args.response_format.bound(&text), json!({ "contradictions": data })))
    }

    async fn consolidate(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: ConsolidateArgs = parse(call)?;
        let stats = self
            .store
            .consolidate(args.scope.as_deref().map(parse_scope))
            .await?;
        Ok((
            format!(
                "consolidated: +{} supersession link(s), +{} contradiction link(s)",
                stats.supersession_links, stats.contradiction_links
            ),
            json!({
                "supersessionLinks": stats.supersession_links,
                "contradictionLinks": stats.contradiction_links
            }),
        ))
    }
}

// --------------------------------------------------------------------------- //
// Shared helpers
// --------------------------------------------------------------------------- //

fn render_result(result: &RecallResult, format: ResponseFormat) -> (String, Value) {
    let context = render_recall(result);
    let hits: Vec<Value> = result
        .hits
        .iter()
        .map(|s| fact_json(&s.fact, Some(s.score), result.now))
        .collect();
    let contradictions: Vec<Value> = result
        .contradictions
        .iter()
        .map(|p| json!({ "a": p.a.id, "b": p.b.id }))
        .collect();
    (
        format.bound(&context),
        json!({
            "context": context,
            "results": hits,
            "contradictions": contradictions,
            "responseFormat": format.as_str(),
        }),
    )
}

/// Structured JSON for one fact (used by tools + the CLI `recall` output).
pub fn fact_json(fact: &TemporalFact, score: Option<f32>, now: OffsetDateTime) -> Value {
    json!({
        "id": fact.id,
        "slug": fact.provenance.first().cloned().unwrap_or_else(|| fact.id.clone()),
        "subject": fact.subject,
        "text": fact.text,
        "score": score,
        "status": crate::store::status_label(fact, now),
        "validAt": format_time(fact.valid_at),
        "invalidAt": fact.invalid_at.map(format_time),
        "ingestedAt": format_time(fact.ingested_at),
        "expiredAt": fact.expired_at.map(format_time),
        "supersedes": fact.supersedes,
        "supersededBy": fact.superseded_by,
        "supersessionReason": fact.supersession_reason,
        "provenance": fact.provenance,
    })
}

pub fn parse_scope(scope: &str) -> MemoryScope {
    match scope {
        "global" => MemoryScope::Global,
        v if v.starts_with("global") => MemoryScope::Global,
        v if v.starts_with("project:") => MemoryScope::Project(v["project:".len()..].to_string()),
        v if v.starts_with("workspace:") => {
            MemoryScope::Workspace(v["workspace:".len()..].to_string())
        }
        v if v.starts_with("thread:") => MemoryScope::Thread(v["thread:".len()..].to_string()),
        v if v.starts_with("user:") => MemoryScope::User(v["user:".len()..].to_string()),
        v => MemoryScope::Project(v.to_string()),
    }
}

fn parse<T: serde::de::DeserializeOwned>(call: &ToolCall) -> anyhow::Result<T> {
    Ok(serde_json::from_value(call.arguments.clone())?)
}

fn parse_opt(value: Option<&str>) -> anyhow::Result<Option<OffsetDateTime>> {
    value.map(parse_flexible).transpose()
}

// --------------------------------------------------------------------------- //
// Argument structs
// --------------------------------------------------------------------------- //

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureArgs {
    text: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    valid_at: Option<String>,
    #[serde(default)]
    invalid_at: Option<String>,
    #[serde(default)]
    ingested_at: Option<String>,
    #[serde(default)]
    provenance: Vec<String>,
    #[serde(default)]
    metadata: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecallArgs {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    as_of: Option<String>,
    #[serde(default)]
    valid_time: Option<String>,
    #[serde(default)]
    transaction_time: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    include_global: Option<bool>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    response_format: ResponseFormat,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SupersedeArgs {
    old_id: String,
    text: String,
    reason: String,
    #[serde(default)]
    valid_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryArgs {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    response_format: ResponseFormat,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContradictionsArgs {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    response_format: ResponseFormat,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConsolidateArgs {
    #[serde(default)]
    scope: Option<String>,
}

// --------------------------------------------------------------------------- //
// Specs
// --------------------------------------------------------------------------- //

fn description(op: Op) -> &'static str {
    match op {
        Op::Capture => "Capture a bi-temporal organizational fact (valid/transaction time, optional subject + provenance).",
        Op::Recall => "Hybrid recall of current-belief facts; optionally pin valid/transaction time for an as-of query.",
        Op::AsOf => "Reconstruct what the organization believed AS OF a past date (bi-temporal date-travel), flagging what has since changed.",
        Op::Supersede => "Replace a fact with a corrected one, recording the supersession link and an explicit reason.",
        Op::History => "Return the full timeline for a subject or fact, including superseded/invalidated versions.",
        Op::Contradictions => "Detect facts that conflict (same subject, overlapping validity, not superseding).",
        Op::Consolidate => "Rebuild the supersession + contradiction link graph (the extract/dream pass).",
    }
}

fn scope_schema() -> Value {
    json!({ "type": "string", "description": "global | project:<id> | workspace:<id> | thread:<id> | user:<id>" })
}

fn schema(op: Op) -> Value {
    match op {
        Op::Capture => json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "The fact statement." },
                "scope": scope_schema(),
                "subject": { "type": "string", "description": "Entity/key the fact is about (groups supersession + contradiction)." },
                "validAt": { "type": "string", "description": "ISO date the fact became true (defaults to now)." },
                "invalidAt": { "type": "string", "description": "ISO date the fact stopped being true, if known." },
                "ingestedAt": { "type": "string", "description": "ISO date the org recorded it (defaults to now)." },
                "provenance": { "type": "array", "items": { "type": "string" }, "description": "Source artifact ids/slugs." },
                "metadata": { "type": "object" }
            },
            "required": ["text"],
            "additionalProperties": false
        }),
        Op::Recall => json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "asOf": { "type": "string", "description": "ISO date — pin BOTH timelines to this instant (as-of belief)." },
                "validTime": { "type": "string", "description": "ISO date — what was true in the world at this instant." },
                "transactionTime": { "type": "string", "description": "ISO date — what was on record at this instant." },
                "scope": scope_schema(),
                "includeGlobal": { "type": "boolean" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50 },
                "response_format": response_format::schema()
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        Op::AsOf => json!({
            "type": "object",
            "properties": {
                "date": { "type": "string", "description": "ISO date to travel to." },
                "query": { "type": "string" },
                "scope": scope_schema(),
                "limit": { "type": "integer", "minimum": 1, "maximum": 50 },
                "response_format": response_format::schema()
            },
            "required": ["date"],
            "additionalProperties": false
        }),
        Op::Supersede => json!({
            "type": "object",
            "properties": {
                "oldId": { "type": "string" },
                "text": { "type": "string", "description": "The new (replacement) fact." },
                "reason": { "type": "string", "description": "Why the change happened." },
                "validAt": { "type": "string", "description": "ISO date the new fact became true (defaults to now)." }
            },
            "required": ["oldId", "text", "reason"],
            "additionalProperties": false
        }),
        Op::History => json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "subject": { "type": "string" },
                "scope": scope_schema(),
                "response_format": response_format::schema()
            },
            "additionalProperties": false
        }),
        Op::Contradictions => json!({
            "type": "object",
            "properties": {
                "scope": scope_schema(),
                "subject": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 100 },
                "response_format": response_format::schema()
            },
            "additionalProperties": false
        }),
        Op::Consolidate => json!({
            "type": "object",
            "properties": { "scope": scope_schema() },
            "additionalProperties": false
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::policy_mode::PolicyMode;

    fn store() -> Arc<GbrainStore> {
        Arc::new(GbrainStore::open_in_memory(crate::embed::Embedder::new(None)).unwrap())
    }

    fn ctx() -> ToolExecutionContext {
        ToolExecutionContext::new("t", "u", PolicyMode::Default)
    }

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            raw_arguments: args.to_string(),
            arguments: args,
            thread_id: "t".into(),
            turn_id: "u".into(),
        }
    }

    async fn run(tool: &GbrainTool, name: &str, args: Value) -> ToolResult {
        tool.execute(ctx(), call(name, args)).await.unwrap()
    }

    #[tokio::test]
    async fn capture_recall_and_as_of_roundtrip() {
        let store = store();
        let capture = GbrainTool { store: store.clone(), op: Op::Capture };
        let recall = GbrainTool { store: store.clone(), op: Op::Recall };
        let supersede = GbrainTool { store: store.clone(), op: Op::Supersede };
        let as_of = GbrainTool { store: store.clone(), op: Op::AsOf };

        let v1 = run(
            &capture,
            "gbrain_capture",
            json!({"text": "Acme account owner is Maya", "subject": "acme-owner", "validAt": "2022-01-01"}),
        )
        .await;
        let v1_id = v1.data["id"].as_str().unwrap().to_string();

        run(
            &supersede,
            "gbrain_supersede",
            json!({"oldId": v1_id, "text": "Acme account owner is Daniel", "reason": "Maya left", "validAt": "2024-01-01"}),
        )
        .await;

        // Current belief -> Daniel.
        let now = run(&recall, "gbrain_recall", json!({"query": "who owns acme"})).await;
        assert!(now.text.contains("Daniel"), "current recall: {}", now.text);

        // As of 2023 -> Maya.
        let past = run(&as_of, "gbrain_as_of", json!({"date": "2023-06-01", "query": "who owns acme"})).await;
        assert!(past.text.contains("Maya"), "as-of recall: {}", past.text);
    }
}
