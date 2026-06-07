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

use crate::dream::{DreamMode, DreamPolicy};
use crate::import::{DedupeMode, ImportBatchInput, ImportBatchParams};
use crate::model::{AsOf, TemporalFact, format_time, parse_flexible};
use crate::render::render_recall;
use crate::response_format::{self, ResponseFormat};
use crate::store::{CaptureInput, DreamParams, GbrainStore, RecallParams, RecallResult};

pub struct GbrainToolContributor {
    store: Arc<GbrainStore>,
}

impl GbrainToolContributor {
    pub fn new(store: Arc<GbrainStore>) -> Self {
        Self { store }
    }

    pub fn contribute_read_only(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        for op in Op::ALL.iter().copied().filter(|op| op.is_read_only()) {
            registry.register(Arc::new(GbrainTool {
                store: self.store.clone(),
                op,
            }))?;
        }
        Ok(())
    }
}

impl ToolContributor for GbrainToolContributor {
    fn id(&self) -> ToolProviderId {
        "gbrain-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        for op in Op::ALL {
            registry.register(Arc::new(GbrainTool {
                store: self.store.clone(),
                op: *op,
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
    Import,
    Dream,
    DreamStatus,
    SearchRaw,
    FindContradictions,
    FindStartNodes,
    ExpandNeighbors,
    FindPaths,
    ExplainNode,
    GetCommunity,
    RetrievalNote,
    RespondToQuery,
}

impl Op {
    const ALL: &'static [Op] = &[
        Op::Capture,
        Op::Recall,
        Op::AsOf,
        Op::Supersede,
        Op::History,
        Op::Contradictions,
        Op::Consolidate,
        Op::Import,
        Op::Dream,
        Op::DreamStatus,
        Op::SearchRaw,
        Op::FindContradictions,
        Op::FindStartNodes,
        Op::ExpandNeighbors,
        Op::FindPaths,
        Op::ExplainNode,
        Op::GetCommunity,
        Op::RetrievalNote,
        Op::RespondToQuery,
    ];

    fn name(self) -> &'static str {
        match self {
            Op::Capture => "gbrain_capture",
            Op::Recall => "gbrain_recall",
            Op::AsOf => "gbrain_as_of",
            Op::Supersede => "gbrain_supersede",
            Op::History => "gbrain_history",
            Op::Contradictions => "gbrain_contradictions",
            Op::Consolidate => "gbrain_consolidate",
            Op::Import => "gbrain_import",
            Op::Dream => "gbrain_dream",
            Op::DreamStatus => "gbrain_dream_status",
            Op::SearchRaw => "gbrain_search_raw",
            Op::FindContradictions => "gbrain_find_contradictions",
            Op::FindStartNodes => "gbrain_find_start_nodes",
            Op::ExpandNeighbors => "gbrain_expand_neighbors",
            Op::FindPaths => "gbrain_find_paths",
            Op::ExplainNode => "gbrain_explain_node",
            Op::GetCommunity => "gbrain_get_community",
            Op::RetrievalNote => "gbrain_retrieval_note",
            Op::RespondToQuery => "respond_to_query",
        }
    }

    fn is_read_only(self) -> bool {
        !matches!(
            self,
            Op::Capture | Op::Supersede | Op::Consolidate | Op::Import | Op::Dream
        )
    }
}

pub fn read_only_tool_names() -> &'static [&'static str] {
    &[
        "gbrain_recall",
        "gbrain_as_of",
        "gbrain_history",
        "gbrain_contradictions",
        "gbrain_dream_status",
        "gbrain_search_raw",
        "gbrain_find_contradictions",
        "gbrain_find_start_nodes",
        "gbrain_expand_neighbors",
        "gbrain_find_paths",
        "gbrain_explain_node",
        "gbrain_get_community",
        "gbrain_retrieval_note",
        "respond_to_query",
    ]
}

pub fn is_read_only_tool(name: &str) -> bool {
    read_only_tool_names().contains(&name)
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
            Op::Import => self.import(&call).await,
            Op::Dream => self.dream(&call).await,
            Op::DreamStatus => self.dream_status(&call).await,
            Op::SearchRaw => self.search_raw(&call).await,
            Op::FindContradictions => self.find_contradictions(&call).await,
            Op::FindStartNodes => self.find_start_nodes(&call).await,
            Op::ExpandNeighbors => self.not_yet_dreamed(&call, "neighbors").await,
            Op::FindPaths => self.not_yet_dreamed(&call, "paths").await,
            Op::ExplainNode => self.not_yet_dreamed(&call, "node").await,
            Op::GetCommunity => self.not_yet_dreamed(&call, "community").await,
            Op::RetrievalNote => self.retrieval_note(&call).await,
            Op::RespondToQuery => self.respond_to_query(&call).await,
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
            args.scope
                .as_deref()
                .map(parse_scope)
                .unwrap_or(MemoryScope::Global),
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
        Ok((
            args.response_format.bound(&text),
            json!({ "contradictions": data }),
        ))
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

    async fn import(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: ImportArgs = parse(call)?;
        let input = if let Some(payload) = args.payload {
            ImportBatchInput::JsonlString(payload)
        } else if let Some(path) = args.input {
            ImportBatchInput::Path(path.into())
        } else {
            anyhow::bail!("gbrain_import requires `input` or `payload`");
        };
        let result = self
            .store
            .import_batch(ImportBatchParams {
                input,
                format: args.format.unwrap_or_else(|| "jsonl".to_string()),
                scope: args
                    .scope
                    .as_deref()
                    .map(parse_scope)
                    .unwrap_or(MemoryScope::Global),
                source: args.source,
                dedupe: parse_dedupe(args.dedupe.as_deref())?,
                dream_after_import: args.dream_after_import,
                metadata: args.metadata,
            })
            .await?;
        Ok((
            format!(
                "import {} completed: {} inserted, {} duplicate(s) skipped",
                result.run_id, result.inserted, result.skipped_duplicates
            ),
            json!({ "readOnly": false, "result": result }),
        ))
    }

    async fn dream(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: DreamArgs = parse(call)?;
        let result = self
            .store
            .dream(DreamParams {
                mode: parse_dream_mode(args.mode.as_deref().unwrap_or("enrich"))?,
                scope: args
                    .scope
                    .as_deref()
                    .map(parse_scope)
                    .unwrap_or(MemoryScope::Global),
                since: parse_opt(args.since.as_deref())?,
                run_policy: parse_dream_policy(
                    args.run_policy.as_deref().unwrap_or("maintenance"),
                )?,
                workers: args.workers.unwrap_or(1),
                dry_run: args.dry_run.unwrap_or(false),
                cancellation_token: args.cancellation_token,
                reasoner_model: args.reasoner_model,
            })
            .await?;
        Ok((
            format!("dream {} {}", result.id, result.status.as_str()),
            json!({ "readOnly": false, "run": result }),
        ))
    }

    async fn dream_status(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: DreamStatusArgs = parse(call)?;
        let status = self
            .store
            .dream_status(&args.run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("dream run not found: {}", args.run_id))?;
        Ok((
            format!("dream {} {}", status.id, status.status.as_str()),
            json!({ "readOnly": true, "run": status }),
        ))
    }

    async fn search_raw(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: SearchRawArgs = parse(call)?;
        let result = self
            .store
            .recall(RecallParams {
                query: args.query,
                as_of: parse_as_of(args.as_of.as_deref())?,
                scope: args.scope.as_deref().map(parse_scope),
                include_global: args.include_global.unwrap_or(true),
                limit: args.limit.unwrap_or(10),
                expand: false,
            })
            .await?;
        let hits: Vec<Value> = result
            .hits
            .iter()
            .map(|hit| {
                json!({
                    "observationType": "raw_fact",
                    "fact": fact_json(&hit.fact, Some(hit.score), result.now),
                    "trace": {
                        "source": "gbrain_facts",
                        "readOnly": true,
                    }
                })
            })
            .collect();
        Ok((
            format!("{} raw fact observation(s)", hits.len()),
            json!({
                "tool": call.name,
                "readOnly": true,
                "observations": hits,
                "contradictions": result.contradictions.iter().map(|p| json!({ "a": p.a.id, "b": p.b.id })).collect::<Vec<_>>(),
            }),
        ))
    }

    async fn find_contradictions(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: FindContradictionsArgs = parse(call)?;
        let pairs = self
            .store
            .contradictions(
                args.scope.as_deref().map(parse_scope),
                args.entity_id
                    .as_deref()
                    .or(args.subject.as_deref())
                    .or(args.predicate_family.as_deref()),
                args.limit.unwrap_or(20),
            )
            .await?;
        let observations: Vec<Value> = pairs
            .iter()
            .map(|pair| {
                json!({
                    "observationType": "contradiction",
                    "a": fact_json(&pair.a, None, OffsetDateTime::now_utc()),
                    "b": fact_json(&pair.b, None, OffsetDateTime::now_utc()),
                    "trace": {
                        "source": "gbrain_contradictions",
                        "readOnly": true,
                    }
                })
            })
            .collect();
        Ok((
            format!("{} contradiction observation(s)", observations.len()),
            json!({
                "tool": call.name,
                "readOnly": true,
                "timeWindow": args.time_window,
                "observations": observations,
            }),
        ))
    }

    async fn find_start_nodes(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: FindStartNodesArgs = parse(call)?;
        let result = self
            .store
            .recall(RecallParams {
                query: args.query,
                as_of: parse_as_of(args.as_of.as_deref())?,
                scope: args.scope.as_deref().map(parse_scope),
                include_global: true,
                limit: args.limit.unwrap_or(8),
                expand: false,
            })
            .await?;
        let requested_node_kinds = args.node_kinds.clone();
        let observations: Vec<Value> = result
            .hits
            .iter()
            .map(|hit| {
                json!({
                    "observationType": "start_node",
                    "node": {
                        "id": hit.fact.id,
                        "kind": "raw_fact",
                        "label": hit.fact.subject.clone().unwrap_or_else(|| hit.fact.text.chars().take(80).collect()),
                        "sourceFactId": hit.fact.id,
                        "score": hit.score,
                    },
                    "fact": fact_json(&hit.fact, Some(hit.score), result.now),
                    "trace": {
                        "source": "gbrain_facts",
                        "readOnly": true,
                        "requestedNodeKinds": requested_node_kinds.clone(),
                        "fallback": "raw_until_dream_graph_schema_lands",
                    }
                })
            })
            .collect();
        Ok((
            format!("{} start node observation(s)", observations.len()),
            json!({
                "tool": call.name,
                "readOnly": true,
                "observations": observations,
            }),
        ))
    }

    async fn not_yet_dreamed(
        &self,
        call: &ToolCall,
        observation_type: &'static str,
    ) -> anyhow::Result<(String, Value)> {
        Ok((
            format!("{observation_type} graph observations are not yet dreamed"),
            json!({
                "tool": call.name,
                "readOnly": true,
                "observations": [],
                "status": "not_yet_dreamed",
                "trace": {
                    "source": "dream_graph_schema_pending",
                    "readOnly": true,
                    "arguments": call.arguments,
                }
            }),
        ))
    }

    async fn retrieval_note(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: RetrievalNoteArgs = parse(call)?;
        Ok((
            "recorded retrieval note".to_string(),
            json!({
                "tool": call.name,
                "readOnly": true,
                "observationType": "retrieval_note",
                "note": args.note,
                "evidenceIds": args.evidence_ids,
                "openQuestions": args.open_questions,
                "trace": {
                    "source": "agent_scratchpad",
                    "readOnly": true,
                }
            }),
        ))
    }

    async fn respond_to_query(&self, call: &ToolCall) -> anyhow::Result<(String, Value)> {
        let args: RespondToQueryArgs = parse(call)?;
        Ok((
            args.message.clone(),
            json!({
                "tool": call.name,
                "readOnly": true,
                "observationType": "final_response",
                "message": args.message,
                "citedEvidenceIds": args.cited_evidence_ids,
                "confidence": args.confidence,
                "openQuestions": args.open_questions,
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

fn parse_as_of(value: Option<&str>) -> anyhow::Result<AsOf> {
    Ok(value
        .map(parse_flexible)
        .transpose()?
        .map(AsOf::at)
        .unwrap_or_else(AsOf::now))
}

fn parse_dedupe(value: Option<&str>) -> anyhow::Result<DedupeMode> {
    value.unwrap_or("both").parse()
}

fn parse_dream_mode(value: &str) -> anyhow::Result<DreamMode> {
    match value {
        "enrich" => Ok(DreamMode::Enrich),
        "refine" => Ok(DreamMode::Refine),
        "compact" => Ok(DreamMode::Compact),
        "full" => Ok(DreamMode::Full),
        other => anyhow::bail!("unknown dream mode {other:?}; expected enrich|refine|compact|full"),
    }
}

fn parse_dream_policy(value: &str) -> anyhow::Result<DreamPolicy> {
    match value {
        "interactive" => Ok(DreamPolicy::Interactive),
        "eval" => Ok(DreamPolicy::Eval),
        "import" => Ok(DreamPolicy::Import),
        "maintenance" => Ok(DreamPolicy::Maintenance),
        other => anyhow::bail!(
            "unknown dream policy {other:?}; expected interactive|eval|import|maintenance"
        ),
    }
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportArgs {
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    payload: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    dedupe: Option<String>,
    #[serde(default)]
    dream_after_import: Option<String>,
    #[serde(default)]
    metadata: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DreamArgs {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    since: Option<String>,
    #[serde(default)]
    run_policy: Option<String>,
    #[serde(default)]
    workers: Option<usize>,
    #[serde(default)]
    dry_run: Option<bool>,
    #[serde(default)]
    cancellation_token: Option<String>,
    #[serde(default)]
    reasoner_model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DreamStatusArgs {
    run_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchRawArgs {
    query: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    as_of: Option<String>,
    #[serde(default)]
    include_global: Option<bool>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FindContradictionsArgs {
    #[serde(default)]
    entity_id: Option<String>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    predicate_family: Option<String>,
    #[serde(default)]
    time_window: Option<Value>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FindStartNodesArgs {
    query: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    as_of: Option<String>,
    #[serde(default)]
    node_kinds: Vec<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RetrievalNoteArgs {
    note: String,
    #[serde(default)]
    evidence_ids: Vec<String>,
    #[serde(default)]
    open_questions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RespondToQueryArgs {
    message: String,
    #[serde(default)]
    cited_evidence_ids: Vec<String>,
    #[serde(default)]
    confidence: Option<String>,
    #[serde(default)]
    open_questions: Vec<String>,
}

// --------------------------------------------------------------------------- //
// Specs
// --------------------------------------------------------------------------- //

fn description(op: Op) -> &'static str {
    match op {
        Op::Capture => {
            "Capture a bi-temporal organizational fact (valid/transaction time, optional subject + provenance)."
        }
        Op::Recall => {
            "Hybrid recall of current-belief facts; optionally pin valid/transaction time for an as-of query."
        }
        Op::AsOf => {
            "Reconstruct what the organization believed AS OF a past date (bi-temporal date-travel), flagging what has since changed."
        }
        Op::Supersede => {
            "Replace a fact with a corrected one, recording the supersession link and an explicit reason."
        }
        Op::History => {
            "Return the full timeline for a subject or fact, including superseded/invalidated versions."
        }
        Op::Contradictions => {
            "Detect facts that conflict (same subject, overlapping validity, not superseding)."
        }
        Op::Consolidate => {
            "Rebuild the supersession + contradiction link graph (the extract/dream pass)."
        }
        Op::Import => {
            "Mutating batch import of JSONL raw memory facts from a local path or supplied payload; explicit maintenance only."
        }
        Op::Dream => {
            "Mutating explicit dream maintenance run. Creates a ledger row and runs deterministic consolidation for refine/full modes."
        }
        Op::DreamStatus => "Read-only inspection of an explicit dream maintenance run by id.",
        Op::SearchRaw => {
            "Read-only raw fact search fallback. Use after ontology/evidence-card navigation is insufficient."
        }
        Op::FindContradictions => {
            "Read-only contradiction search over existing fact links; never consolidates or mutates memory."
        }
        Op::FindStartNodes => {
            "Read-only entrypoint discovery for graph navigation. Returns dreamed graph nodes when available, otherwise traceable raw-fact start nodes."
        }
        Op::ExpandNeighbors => {
            "Read-only graph neighbor expansion for dreamed nodes. Returns an empty not-yet-dreamed observation until graph schema lands."
        }
        Op::FindPaths => {
            "Read-only path search between dreamed graph nodes. Returns an empty not-yet-dreamed observation until graph schema lands."
        }
        Op::ExplainNode => {
            "Read-only node explanation with provenance, aliases, and community context when dreamed graph schema is available."
        }
        Op::GetCommunity => {
            "Read-only community lookup for dreamed graph neighborhoods. Returns an empty not-yet-dreamed observation until graph schema lands."
        }
        Op::RetrievalNote => {
            "Record an agent scratchpad note for the current retrieval trace without mutating memory."
        }
        Op::RespondToQuery => {
            "Return the final free-form answer once evidence, temporal state, and contradiction checks are sufficient."
        }
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
        Op::Import => json!({
            "type": "object",
            "properties": {
                "input": { "type": "string", "description": "Local JSONL file path. Use payload when passing inline JSONL." },
                "payload": { "type": "string", "description": "Inline JSONL payload supplied by the caller." },
                "format": { "type": "string", "enum": ["jsonl"], "default": "jsonl" },
                "scope": scope_schema(),
                "source": { "type": "string" },
                "dedupe": { "type": "string", "enum": ["source_id", "content_hash", "both"], "default": "both" },
                "dreamAfterImport": { "type": "string", "enum": ["enrich", "refine", "compact", "full"] },
                "metadata": { "type": "object" }
            },
            "additionalProperties": false
        }),
        Op::Dream => json!({
            "type": "object",
            "properties": {
                "mode": { "type": "string", "enum": ["enrich", "refine", "compact", "full"], "default": "enrich" },
                "scope": scope_schema(),
                "since": { "type": "string" },
                "runPolicy": { "type": "string", "enum": ["interactive", "eval", "import", "maintenance"], "default": "maintenance" },
                "workers": { "type": "integer", "minimum": 1, "maximum": 64 },
                "dryRun": { "type": "boolean" },
                "cancellationToken": { "type": "string" },
                "reasonerModel": { "type": "string" }
            },
            "additionalProperties": false
        }),
        Op::DreamStatus => json!({
            "type": "object",
            "properties": {
                "runId": { "type": "string" }
            },
            "required": ["runId"],
            "additionalProperties": false
        }),
        Op::SearchRaw => json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "scope": scope_schema(),
                "asOf": { "type": "string", "description": "Optional ISO date for read-only as-of recall." },
                "includeGlobal": { "type": "boolean" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        Op::FindContradictions => json!({
            "type": "object",
            "properties": {
                "entityId": { "type": "string" },
                "subject": { "type": "string" },
                "predicateFamily": { "type": "string" },
                "timeWindow": { "type": "object" },
                "scope": scope_schema(),
                "limit": { "type": "integer", "minimum": 1, "maximum": 100 }
            },
            "additionalProperties": false
        }),
        Op::FindStartNodes => json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "scope": scope_schema(),
                "asOf": { "type": "string" },
                "nodeKinds": { "type": "array", "items": { "type": "string" } },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        Op::ExpandNeighbors => json!({
            "type": "object",
            "properties": {
                "nodeId": { "type": "string" },
                "edgeKinds": { "type": "array", "items": { "type": "string" } },
                "depth": { "type": "integer", "minimum": 1, "maximum": 4 },
                "asOf": { "type": "string" }
            },
            "required": ["nodeId"],
            "additionalProperties": false
        }),
        Op::FindPaths => json!({
            "type": "object",
            "properties": {
                "sourceNodeId": { "type": "string" },
                "targetNodeId": { "type": "string" },
                "relationFilter": { "type": "array", "items": { "type": "string" } },
                "confidenceFilter": { "type": "array", "items": { "type": "string" } },
                "asOf": { "type": "string" },
                "budget": { "type": "integer", "minimum": 1, "maximum": 100 }
            },
            "required": ["sourceNodeId", "targetNodeId"],
            "additionalProperties": false
        }),
        Op::ExplainNode => json!({
            "type": "object",
            "properties": {
                "nodeId": { "type": "string" },
                "includeSources": { "type": "boolean" },
                "includeAliases": { "type": "boolean" },
                "includeCommunity": { "type": "boolean" }
            },
            "required": ["nodeId"],
            "additionalProperties": false
        }),
        Op::GetCommunity => json!({
            "type": "object",
            "properties": {
                "nodeId": { "type": "string" },
                "communityId": { "type": "string" },
                "includeMembers": { "type": "boolean" },
                "includeHubs": { "type": "boolean" }
            },
            "additionalProperties": false
        }),
        Op::RetrievalNote => json!({
            "type": "object",
            "properties": {
                "note": { "type": "string" },
                "evidenceIds": { "type": "array", "items": { "type": "string" } },
                "openQuestions": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["note"],
            "additionalProperties": false
        }),
        Op::RespondToQuery => json!({
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "Free-form final answer. Cite evidence ids naturally when useful." },
                "citedEvidenceIds": { "type": "array", "items": { "type": "string" } },
                "confidence": { "type": "string", "enum": ["high", "medium", "low", "unsupported"] },
                "openQuestions": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["message"],
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
        let capture = GbrainTool {
            store: store.clone(),
            op: Op::Capture,
        };
        let recall = GbrainTool {
            store: store.clone(),
            op: Op::Recall,
        };
        let supersede = GbrainTool {
            store: store.clone(),
            op: Op::Supersede,
        };
        let as_of = GbrainTool {
            store: store.clone(),
            op: Op::AsOf,
        };

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
        let past = run(
            &as_of,
            "gbrain_as_of",
            json!({"date": "2023-06-01", "query": "who owns acme"}),
        )
        .await;
        assert!(past.text.contains("Maya"), "as-of recall: {}", past.text);
    }
}
