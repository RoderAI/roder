//! `roder-gbrain` — a thin CLI over the bi-temporal gbrain store.
//!
//! Exists so external harnesses (notably the OrgMemBench `bitemporal-gbrain`
//! adapter) can ingest + recall against a bi-temporal store. All output is JSON.
//!
//! Subcommands:
//!   capture       (reads a JSON object on stdin) -> {"id": "..."}
//!   consolidate                                  -> {"supersession_links":N,"contradiction_links":M}
//!   recall  --query Q [--as-of D] [--limit N] [--scope S]
//!                                                -> {"results":[...],"contradictions":[...],"context":"..."}
//!   version                                      -> {"version":"...","commit":"..."}
//!
//! Storage path: --db <path> or $GBRAIN_DB (default: $TMPDIR/roder-gbrain.sqlite3).
//! Embeddings: OpenAI when $OPENAI_API_KEY is set, else deterministic local.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use roder_api::embeddings::EmbeddingProvider;
use roder_api::memory::MemoryScope;
use roder_ext_gbrain::model::{AsOf, parse_flexible};
use roder_ext_gbrain::render::render_recall;
use roder_ext_gbrain::store::{CaptureInput, GbrainStore, RecallParams};
use roder_ext_gbrain::tools::{fact_json, parse_scope};
use roder_ext_gbrain::{AgentBudget, DecisionAgent, Embedder, build_reasoner};
use roder_ext_openai_embeddings::OpenAiEmbeddingProvider;
use serde::Deserialize;
use serde_json::{Value, json};
use time::OffsetDateTime;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{}", json!({ "error": err.to_string() }));
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, flags) = parse_args(&args);

    if command == "version" {
        println!(
            "{}",
            json!({
                "version": env!("CARGO_PKG_VERSION"),
                "commit": option_env!("GIT_SHA").unwrap_or("dev"),
            })
        );
        return Ok(());
    }

    let store = open_store(&flags)?;

    match command.as_str() {
        "capture" => capture(&store).await,
        "consolidate" => consolidate(&store, &flags).await,
        "recall" => recall(&store, &flags).await,
        "answer" => answer_cmd(Arc::new(store), &flags).await,
        other => anyhow::bail!(
            "unknown command {other:?}; expected capture|consolidate|recall|answer|version"
        ),
    }
}

/// Agentic decision-loop answer (decompose -> retrieve -> draft -> verify/prune
/// -> finalize). Self-answers (grounded prose + provenance), unlike `recall`.
async fn answer_cmd(
    store: Arc<GbrainStore>,
    flags: &HashMap<String, String>,
) -> anyhow::Result<()> {
    let query = flags.get("query").cloned().unwrap_or_default();
    let scope = flags.get("scope").map(|s| parse_scope(s));
    let as_of = match flags.get("as-of").or_else(|| flags.get("as_of")) {
        Some(date) => Some(parse_flexible(date)?),
        None => None,
    };
    let reasoner = build_reasoner(flags.get("model").cloned())?;
    let mut budget = AgentBudget::default();
    if let Some(n) = flags.get("max-subqueries").and_then(|v| v.parse().ok()) {
        budget.max_subqueries = n;
    }
    if let Some(n) = flags.get("limit").and_then(|v| v.parse().ok()) {
        budget.retrieval_limit = n;
    }
    let agent = DecisionAgent::new(store, reasoner)
        .with_scope(scope)
        .with_budget(budget);
    // Default: token-light concise single-call synthesis. `--thorough` runs the
    // full decompose/draft/verify/finalize loop (more tokens, opt-in).
    let result = if flags.contains_key("thorough") {
        agent.answer(&query, as_of).await?
    } else {
        agent.answer_concise(&query, as_of).await?
    };
    println!(
        "{}",
        json!({
            "answer": result.answer,
            "cited_artifact_ids": result.provenance,
            "trace": result.context,
        })
    );
    Ok(())
}

fn open_store(flags: &HashMap<String, String>) -> anyhow::Result<GbrainStore> {
    let db = flags
        .get("db")
        .map(PathBuf::from)
        .or_else(|| std::env::var("GBRAIN_DB").ok().map(PathBuf::from))
        .unwrap_or_else(|| std::env::temp_dir().join("roder-gbrain.sqlite3"));
    let provider: Option<Arc<dyn EmbeddingProvider>> = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .map(|key| Arc::new(OpenAiEmbeddingProvider::new(Some(key))) as Arc<dyn EmbeddingProvider>);
    GbrainStore::open(db, Embedder::new(provider))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CapturePayload {
    text: String,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    valid_at: Option<String>,
    #[serde(default)]
    invalid_at: Option<String>,
    #[serde(default)]
    ingested_at: Option<String>,
    #[serde(default)]
    provenance: Vec<String>,
    #[serde(default)]
    supersedes: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    metadata: Value,
}

async fn capture(store: &GbrainStore) -> anyhow::Result<()> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    let payload: CapturePayload = serde_json::from_str(buf.trim())
        .map_err(|e| anyhow::anyhow!("invalid capture JSON on stdin: {e}"))?;

    let mut provenance = payload.provenance;
    if let Some(slug) = &payload.slug
        && !provenance.iter().any(|p| p == slug) {
            provenance.insert(0, slug.clone());
        }

    let mut input = CaptureInput::new(
        payload
            .scope
            .as_deref()
            .map(parse_scope)
            .unwrap_or(MemoryScope::Global),
        payload.text,
    );
    input.subject = payload.subject;
    input.metadata = payload.metadata;
    input.provenance = provenance;
    input.valid_at = parse_opt(payload.valid_at.as_deref())?;
    input.invalid_at = parse_opt(payload.invalid_at.as_deref())?;
    input.ingested_at = parse_opt(payload.ingested_at.as_deref())?;
    input.supersedes = payload.supersedes;
    input.supersession_reason = payload.reason;

    let fact = store.capture(input).await?;
    println!("{}", json!({ "id": fact.id, "slug": fact.provenance.first() }));
    Ok(())
}

async fn consolidate(
    store: &GbrainStore,
    flags: &HashMap<String, String>,
) -> anyhow::Result<()> {
    let scope = flags.get("scope").map(|s| parse_scope(s));
    let stats = store.consolidate(scope).await?;
    println!(
        "{}",
        json!({
            "supersession_links": stats.supersession_links,
            "contradiction_links": stats.contradiction_links,
        })
    );
    Ok(())
}

async fn recall(store: &GbrainStore, flags: &HashMap<String, String>) -> anyhow::Result<()> {
    let query = flags.get("query").cloned().unwrap_or_default();
    let limit = flags
        .get("limit")
        .and_then(|l| l.parse::<usize>().ok())
        .unwrap_or(10);
    let scope = flags.get("scope").map(|s| parse_scope(s));
    let as_of = match flags.get("as-of").or_else(|| flags.get("as_of")) {
        Some(date) => AsOf::at(parse_flexible(date)?),
        None => AsOf::now(),
    };
    // --expand pulls in the top hits' event cluster (for evidence-enumeration
    // questions); the caller (e.g. the OrgMemBench adapter) decides per question.
    let expand = flags.contains_key("expand");

    let result = store
        .recall(RecallParams {
            query,
            as_of,
            scope,
            include_global: true,
            limit,
            expand,
        })
        .await?;

    let now = OffsetDateTime::now_utc();
    let results: Vec<Value> = result
        .hits
        .iter()
        .map(|s| fact_json(&s.fact, Some(s.score), now))
        .collect();
    let contradictions: Vec<Value> = result
        .contradictions
        .iter()
        .map(|p| {
            json!({
                "a": { "id": p.a.id, "text": p.a.text },
                "b": { "id": p.b.id, "text": p.b.text },
            })
        })
        .collect();

    println!(
        "{}",
        json!({
            "answer": Value::Null,
            "results": results,
            "contradictions": contradictions,
            "context": render_recall(&result),
        })
    );
    Ok(())
}

fn parse_opt(value: Option<&str>) -> anyhow::Result<Option<OffsetDateTime>> {
    value.map(parse_flexible).transpose()
}

/// Minimal arg parser: first non-flag token is the command; `--key value` and
/// `--key=value` become flags; bare `--flag` becomes `flag=""`.
fn parse_args(args: &[String]) -> (String, HashMap<String, String>) {
    let mut command = String::new();
    let mut flags = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if let Some(rest) = arg.strip_prefix("--") {
            if let Some((k, v)) = rest.split_once('=') {
                flags.insert(k.to_string(), v.to_string());
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                flags.insert(rest.to_string(), args[i + 1].clone());
                i += 1;
            } else {
                flags.insert(rest.to_string(), String::new());
            }
        } else if command.is_empty() {
            command = arg.clone();
        }
        i += 1;
    }
    (command, flags)
}
