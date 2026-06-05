//! Agentic decision loop for grounded, faithful answers.
//!
//! Motivated by the OrgMemBench failure data: retrieval recall is decent (~0.73)
//! but the single-pass answerer **over-generates** — 65/73 medium answers were
//! flagged hallucinated, polluting correct cores with fabricated dates, events,
//! artifact ids and figures. So this loop does not format-then-emit; it:
//!
//! 1. **decompose** the question into concrete evidence needs,
//! 2. run **multiple retrieval passes** (incl. event-cluster expansion),
//! 3. **draft** atomic claims, each REQUIRED to cite evidence records,
//! 4. **verify** every claim against its cited artifact text and **prune**
//!    unsupported ones (the anti-hallucination core; also classifies
//!    direct-vs-inferred for justification chains),
//! 5. run **temporal checks** (contradiction / supersession) when relevant,
//! 6. **synthesize** a final answer from VERIFIED claims only, with provenance.
//!
//! It is generic over [`Reasoner`] so it runs in the CLI/eval (Anthropic) today
//! and can wrap a registry `InferenceEngine` later. Hook points for future
//! agenticism are called out inline (budgets, progress events, sub-agent
//! dispatch, scratchpad persistence).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use roder_api::memory::MemoryScope;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::model::{AsOf, TemporalFact};
use crate::reason::{Reasoner, extract_json};
use crate::store::{GbrainStore, RecallParams};

/// Budgets + escalation knobs (hook for configurable cost/iteration policies).
#[derive(Debug, Clone, Copy)]
pub struct AgentBudget {
    pub max_subqueries: usize,
    pub retrieval_limit: usize,
    pub evidence_pool_cap: usize,
    pub max_claims: usize,
}

impl Default for AgentBudget {
    fn default() -> Self {
        Self {
            max_subqueries: 3,
            retrieval_limit: 8,
            evidence_pool_cap: 24,
            max_claims: 16,
        }
    }
}

/// Progress / status sink (hook for long-running decision agents — wire to an
/// event stream or task-ledger later). Default impl is silent.
pub trait ProgressSink: Send + Sync {
    fn step(&self, _stage: &str, _detail: &str) {}
}

/// No-op progress sink.
pub struct SilentProgress;
impl ProgressSink for SilentProgress {}

/// One numbered evidence record handed to the model.
#[derive(Debug, Clone, Serialize)]
pub struct EvidenceItem {
    pub index: usize,
    pub slug: String,
    pub date: String,
    pub source: String,
    pub status: String,
    /// Temporal note for as-of questions (e.g. "recorded after the as-of date").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub text: String,
}

/// A drafted claim with the evidence record numbers that supposedly support it.
#[derive(Debug, Clone, Deserialize)]
pub struct Claim {
    pub text: String,
    #[serde(default)]
    pub support: Vec<usize>,
}

/// A claim that survived verification, with resolved provenance slugs.
#[derive(Debug, Clone, Serialize)]
pub struct VerifiedClaim {
    pub text: String,
    pub provenance: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
}

/// The working scratchpad (serializable — persistence hook across passes).
#[derive(Debug, Clone, Serialize)]
pub struct WorkingContext {
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    pub subqueries: Vec<String>,
    pub evidence: Vec<EvidenceItem>,
    pub drafted: usize,
    pub verified: Vec<VerifiedClaim>,
    pub dropped: Vec<String>,
    pub contradictions: Vec<String>,
    pub llm_calls: usize,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Final grounded answer + provenance + trace.
#[derive(Debug, Clone, Serialize)]
pub struct AgentAnswer {
    pub answer: String,
    pub provenance: Vec<String>,
    pub context: WorkingContext,
}

/// The in-process decision agent. Generic over the LLM [`Reasoner`] and backed
/// by the existing [`GbrainStore`] for retrieval / temporal queries.
pub struct DecisionAgent<R: Reasoner> {
    store: Arc<GbrainStore>,
    reasoner: R,
    budget: AgentBudget,
    progress: Box<dyn ProgressSink>,
    scope: Option<MemoryScope>,
    tokens_in: AtomicU32,
    tokens_out: AtomicU32,
}

impl<R: Reasoner> DecisionAgent<R> {
    pub fn new(store: Arc<GbrainStore>, reasoner: R) -> Self {
        Self {
            store,
            reasoner,
            budget: AgentBudget::default(),
            progress: Box::new(SilentProgress),
            scope: None,
            tokens_in: AtomicU32::new(0),
            tokens_out: AtomicU32::new(0),
        }
    }

    pub fn with_budget(mut self, budget: AgentBudget) -> Self {
        self.budget = budget;
        self
    }

    pub fn with_progress(mut self, progress: Box<dyn ProgressSink>) -> Self {
        self.progress = progress;
        self
    }

    pub fn with_scope(mut self, scope: Option<MemoryScope>) -> Self {
        self.scope = scope;
        self
    }

    /// Run the full decision loop for one question.
    pub async fn answer(&self, question: &str, as_of: Option<OffsetDateTime>) -> anyhow::Result<AgentAnswer> {
        self.tokens_in.store(0, Ordering::Relaxed);
        self.tokens_out.store(0, Ordering::Relaxed);
        let mut calls = 0usize;
        let as_of_label = as_of.map(|d| d.date().to_string());

        // 1. Decompose into evidence needs.
        self.progress.step("decompose", question);
        let mut subqueries = self.decompose(question).await.unwrap_or_default();
        calls += 1;
        subqueries.truncate(self.budget.max_subqueries);

        // 2. Multi-pass retrieval -> deduped evidence pool (+ as-of-correct
        //    contradictions, and a current-state pass for as-of questions).
        self.progress.step("retrieve", &format!("{} sub-queries", subqueries.len()));
        let (evidence, contradictions) = self.gather_evidence(question, &subqueries, as_of).await?;

        // 3. Draft atomic, cited claims.
        self.progress.step("draft", &format!("{} evidence records", evidence.len()));
        let claims = self.draft(question, &evidence).await?;
        calls += 1;
        let drafted = claims.len();

        // 4. Verify each claim against its cited artifact text; prune unsupported.
        self.progress.step("verify", &format!("{drafted} claims"));
        let (verified, dropped) = self.verify(question, &claims, &evidence).await?;
        calls += 1;

        // 5. Synthesize the final answer from verified claims only.
        self.progress.step("finalize", &format!("{} verified claims", verified.len()));
        let answer = self
            .finalize(question, &verified, &contradictions, as_of_label.as_deref())
            .await?;
        calls += 1;

        let provenance = dedup_strings(verified.iter().flat_map(|c| c.provenance.clone()));
        let context = WorkingContext {
            question: question.to_string(),
            as_of: as_of_label,
            subqueries,
            evidence,
            drafted,
            verified,
            dropped,
            contradictions,
            llm_calls: calls,
            input_tokens: self.tokens_in.load(Ordering::Relaxed),
            output_tokens: self.tokens_out.load(Ordering::Relaxed),
        };
        Ok(AgentAnswer {
            answer,
            provenance,
            context,
        })
    }

    // ---- stages -------------------------------------------------------------

    /// One reasoner call, with optional raw-output tracing (`GBRAIN_AGENT_DEBUG`).
    async fn call(&self, stage: &str, system: &str, user: &str) -> anyhow::Result<String> {
        let completion = self.reasoner.complete(system, user).await?;
        self.tokens_in.fetch_add(completion.input_tokens, Ordering::Relaxed);
        self.tokens_out.fetch_add(completion.output_tokens, Ordering::Relaxed);
        if std::env::var("GBRAIN_AGENT_DEBUG").is_ok() {
            eprintln!(
                "── agent[{stage}] ──\n{}\n",
                completion.text.chars().take(2000).collect::<String>()
            );
        }
        Ok(completion.text)
    }

    async fn decompose(&self, question: &str) -> anyhow::Result<Vec<String>> {
        let out = self
            .call(
                "decompose",
                DECOMPOSE_SYS,
                &format!("Question: {question}\n\nReturn the JSON array of sub-queries."),
            )
            .await?;
        let arr = extract_json(&out)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let mut subs: Vec<String> = arr
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .filter(|s| !s.trim().is_empty())
            .collect();
        if subs.is_empty() {
            subs.push(question.to_string());
        }
        Ok(subs)
    }

    async fn gather_evidence(
        &self,
        question: &str,
        subqueries: &[String],
        as_of: Option<OffsetDateTime>,
    ) -> anyhow::Result<(Vec<EvidenceItem>, Vec<String>)> {
        // Event-cluster expansion only for evidence-enumeration / provenance /
        // contradiction questions; focused retrieval for "what is X now" questions
        // (over-broad evidence dilutes those and the answer enumerates noise).
        let expand = is_evidence_question(question);
        let mut pool: Vec<EvidenceItem> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut contradictions = std::collections::BTreeSet::new();

        // Primary pass: the as-of snapshot (or current).
        let primary = as_of.map(AsOf::at).unwrap_or_else(AsOf::now);
        self.retrieve_into(question, subqueries, primary, expand, "", &mut pool, &mut seen, &mut contradictions)
            .await?;

        // As-of questions also need the CURRENT state to answer "what has SINCE
        // changed" (C4 audit replay). Records only present now are flagged.
        if let Some(d) = as_of {
            let note = format!("NOT on record as of {} — recorded later / current state", d.date());
            self.retrieve_into(question, subqueries, AsOf::now(), expand, &note, &mut pool, &mut seen, &mut contradictions)
                .await?;
        }
        Ok((pool, contradictions.into_iter().collect()))
    }

    #[allow(clippy::too_many_arguments)]
    async fn retrieve_into(
        &self,
        question: &str,
        subqueries: &[String],
        as_of: AsOf,
        expand: bool,
        note: &str,
        pool: &mut Vec<EvidenceItem>,
        seen: &mut std::collections::HashSet<String>,
        contradictions: &mut std::collections::BTreeSet<String>,
    ) -> anyhow::Result<()> {
        let queries = std::iter::once(question.to_string()).chain(subqueries.iter().cloned());
        for q in queries {
            if pool.len() >= self.budget.evidence_pool_cap {
                break;
            }
            let result = self
                .store
                .recall(RecallParams {
                    query: q,
                    as_of,
                    scope: self.scope.clone(),
                    include_global: true,
                    limit: self.budget.retrieval_limit,
                    expand,
                })
                .await?;
            let now = result.now;
            // Reuse the recall's as-of-correct contradiction set (not wall-clock).
            for pair in &result.contradictions {
                contradictions.insert(format!(
                    "\"{}\" conflicts with \"{}\"",
                    pair.a.text.trim(),
                    pair.b.text.trim()
                ));
            }
            for hit in result.hits {
                if !seen.insert(hit.fact.id.clone()) {
                    continue;
                }
                if pool.len() >= self.budget.evidence_pool_cap {
                    break;
                }
                pool.push(EvidenceItem {
                    index: pool.len() + 1,
                    slug: hit.fact.provenance.first().cloned().unwrap_or_else(|| hit.fact.id.clone()),
                    date: hit.fact.valid_at.date().to_string(),
                    source: source_label(&hit.fact),
                    status: crate::store::status_label(&hit.fact, now).to_string(),
                    note: note.to_string(),
                    text: hit.fact.text.clone(),
                });
            }
        }
        Ok(())
    }

    async fn draft(&self, question: &str, evidence: &[EvidenceItem]) -> anyhow::Result<Vec<Claim>> {
        let out = self
            .call("draft", DRAFT_SYS, &draft_prompt(question, evidence))
            .await?;
        let arr = extract_json(&out)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let mut claims: Vec<Claim> = arr
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Claim>(v).ok())
            .filter(|c| !c.text.trim().is_empty() && !c.support.is_empty())
            .collect();
        claims.truncate(self.budget.max_claims);
        Ok(claims)
    }

    async fn verify(
        &self,
        question: &str,
        claims: &[Claim],
        evidence: &[EvidenceItem],
    ) -> anyhow::Result<(Vec<VerifiedClaim>, Vec<String>)> {
        if claims.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let out = self
            .call("verify", VERIFY_SYS, &verify_prompt(question, claims, evidence))
            .await?;
        let verdicts = extract_json(&out)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let mut kept = Vec::new();
        let mut dropped = Vec::new();
        for (i, claim) in claims.iter().enumerate() {
            let verdict = verdicts.iter().find(|v| {
                v.get("id").and_then(Value::as_u64).map(|x| x as usize) == Some(i + 1)
            });
            let supported = verdict
                .and_then(|v| v.get("supported"))
                .and_then(Value::as_bool)
                // No verdict for a claim => conservatively drop it.
                .unwrap_or(false);
            if supported {
                let provenance = dedup_strings(
                    claim
                        .support
                        .iter()
                        .filter_map(|n| evidence.get(n.saturating_sub(1)).map(|e| e.slug.clone())),
                );
                let classification = verdict
                    .and_then(|v| v.get("classification"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                kept.push(VerifiedClaim {
                    text: claim.text.clone(),
                    provenance,
                    classification,
                });
            } else {
                dropped.push(claim.text.clone());
            }
        }
        Ok((kept, dropped))
    }


    async fn finalize(
        &self,
        question: &str,
        verified: &[VerifiedClaim],
        contradictions: &[String],
        as_of: Option<&str>,
    ) -> anyhow::Result<String> {
        if verified.is_empty() {
            return Ok("The available records do not contain enough evidence to answer this.".to_string());
        }
        self.call("finalize", FINALIZE_SYS, &finalize_prompt(question, verified, contradictions, as_of))
            .await
    }
}

fn dedup_strings(iter: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    iter.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

// --------------------------------------------------------------------------- //
// Prompts — each targets a specific failure mode from the eval data.
// --------------------------------------------------------------------------- //

const DECOMPOSE_SYS: &str = "You break an organizational-memory question into the specific pieces of \
evidence needed to answer it faithfully (who decided, when, what was decided, what alternatives, what \
changed, which documents support it). Output ONLY a JSON array of 2-4 short search sub-queries (strings). \
No prose.";

const DRAFT_SYS: &str = "You answer ONLY from the numbered evidence records. Output a JSON array of atomic \
claims {\"text\": <one factual statement>, \"support\": [<record numbers>]}. HARD RULES: every claim MUST \
cite >=1 record that actually states it; never include a name/date/number/causal link not present in a \
cited record; if unsure, omit it. Answer the SPECIFIC question asked — do NOT summarize every record. For \
'walk me through the evidence' questions, include a record ONLY if it directly establishes the specific \
conclusion the question is about; ignore tangential records from the same event. But DO cover every part \
the question asks (who / when / what was decided / what alternatives / why / what changed / current status) \
whenever a record supports it — completeness on the asked facets matters. Output ONLY the JSON array.";

const VERIFY_SYS: &str = "You are a strict grounding verifier. For each claim you get its cited record text \
(with each record's source type). Decide if the records DIRECTLY state the claim. Output a JSON array of \
{\"id\": <claim number>, \"supported\": <true|false>, \"classification\": <\"direct\"|\"inferred\">, \
\"reason\": <short>}. supported=false if the specific detail (exact date, name, figure, or causal link) is \
not explicitly in the cited records — plausibility is NOT support. CLASSIFICATION (important): \"direct\" = \
the cited record is a FIRST-HAND account by the actor themselves — a Slack/chat message or email WRITTEN BY \
the person who decided or witnessed it. \"inferred\" = the claim is derived from a SECONDARY record — meeting \
notes, an incident report, a post-mortem, a summary, a document, or a third party's mention — NOT the actor's \
own words. Meeting notes and reports are \"inferred\", not direct.";

const FINALIZE_SYS: &str = "Write the final answer using ONLY the verified claims (each with provenance and \
any classification). Add NOTHING beyond them — no extra dates, names, events, or figures. Be specific and \
COMPLETE: address EVERY part the question asks (e.g. who, when, what, alternatives, rationale, current \
status). If the question asks to walk through or justify the evidence, enumerate each evidence item with its \
provenance id and label it direct vs inferred using the provided classification. If the question is 'as of \
<date>', state what was known THEN and, for records marked 'recorded later / current state', say for each \
whether it is still current or has SINCE changed/been replaced. If contradictions are listed, report both \
sides. Do not pad with tangential records. Cite provenance ids inline.";

fn draft_prompt(question: &str, evidence: &[EvidenceItem]) -> String {
    let mut s = format!("Question: {question}\n\nEvidence records:\n");
    for e in evidence {
        let note = if e.note.is_empty() {
            String::new()
        } else {
            format!(", {}", e.note)
        };
        s.push_str(&format!(
            "[{}] ({}, {}, status={}{note}) {}\n",
            e.index,
            e.date,
            if e.source.is_empty() { "source?" } else { &e.source },
            e.status,
            truncate(&e.text, 1200),
        ));
    }
    s.push_str("\nReturn the JSON array of cited claims.");
    s
}

/// `source_type / author` label from a fact's metadata, for attribution +
/// direct-vs-inferred classification.
fn source_label(fact: &TemporalFact) -> String {
    let source = fact
        .metadata
        .get("source_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    let author = fact
        .metadata
        .get("author")
        .and_then(Value::as_str)
        .unwrap_or("");
    match (source.is_empty(), author.is_empty()) {
        (false, false) => format!("{source} / {author}"),
        (false, true) => source.to_string(),
        (true, false) => author.to_string(),
        (true, true) => String::new(),
    }
}

/// Whether a question wants the full evidence cluster (justification / provenance
/// / contradiction / audit) vs a focused current-fact answer.
fn is_evidence_question(question: &str) -> bool {
    let q = question.to_lowercase();
    const MARKERS: &[&str] = &[
        "walk me through",
        "conversation turns",
        "evidence",
        "justif",
        "supporting",
        "supports",
        "which document",
        "which record",
        "which message",
        "step by step",
        "enumerate",
        "both sides",
        "contradict",
        "conflict",
        "who decided",
        "who chose",
        "alternatives",
        "what changed",
        "since changed",
        "as of",
    ];
    MARKERS.iter().any(|m| q.contains(m))
}

fn verify_prompt(question: &str, claims: &[Claim], evidence: &[EvidenceItem]) -> String {
    let mut s = format!("Question: {question}\n\nClaims to verify (with their cited record text):\n");
    for (i, c) in claims.iter().enumerate() {
        s.push_str(&format!("\nClaim {}: {}\nCited records:\n", i + 1, c.text));
        for n in &c.support {
            if let Some(e) = evidence.get(n.saturating_sub(1)) {
                s.push_str(&format!("  [{}] ({}, {}) {}\n", e.index, e.date, e.source, truncate(&e.text, 1200)));
            }
        }
    }
    s.push_str("\nReturn the JSON verdict array.");
    s
}

fn finalize_prompt(
    question: &str,
    verified: &[VerifiedClaim],
    contradictions: &[String],
    as_of: Option<&str>,
) -> String {
    let mut s = String::new();
    if let Some(d) = as_of {
        s.push_str(&format!("This question is about the organization's knowledge AS OF {d}.\n"));
    }
    s.push_str(&format!("Question: {question}\n\nVerified claims (use ONLY these):\n"));
    for (i, c) in verified.iter().enumerate() {
        let cls = c.classification.as_deref().map(|x| format!(" [{x}]")).unwrap_or_default();
        s.push_str(&format!("{}. {}{} (sources: {})\n", i + 1, c.text, cls, c.provenance.join(", ")));
    }
    if !contradictions.is_empty() {
        s.push_str("\nUnresolved contradictions to report:\n");
        for c in contradictions {
            s.push_str(&format!("  - {c}\n"));
        }
    }
    s.push_str("\nWrite the final grounded answer.");
    s
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push_str(" …");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Embedder;
    use crate::reason::Completion;
    use crate::store::CaptureInput;
    use roder_api::memory::MemoryScope;

    /// Deterministic Reasoner that returns canned JSON per stage (matched on the
    /// system prompt) so the loop's plumbing is testable without an LLM.
    struct MockReasoner;

    #[async_trait::async_trait]
    impl Reasoner for MockReasoner {
        async fn complete(&self, system: &str, _user: &str) -> anyhow::Result<Completion> {
            let out: &str = if system == DECOMPOSE_SYS {
                "[\"who owns acme\", \"acme owner history\"]"
            } else if system == DRAFT_SYS {
                // claim 1 cited+true; claim 2 cited but the verifier will reject it.
                "[{\"text\":\"Maya owns Acme\",\"support\":[1]},{\"text\":\"Acme revenue was $70M\",\"support\":[1]}]"
            } else if system == VERIFY_SYS {
                "[{\"id\":1,\"supported\":true,\"classification\":\"direct\"},{\"id\":2,\"supported\":false,\"reason\":\"figure not in record\"}]"
            } else {
                "Maya owns the Acme account (direct)."
            };
            Ok(Completion::text(out))
        }
    }

    async fn store_with_fact() -> Arc<GbrainStore> {
        let store = Arc::new(GbrainStore::open_in_memory(Embedder::new(None)).unwrap());
        let mut input = CaptureInput::new(MemoryScope::Project("p".into()), "Acme account owner is Maya Patel");
        input.provenance = vec!["ART-1".into()];
        store.capture(input).await.unwrap();
        store
    }

    #[tokio::test]
    async fn loop_prunes_unsupported_claims_and_keeps_grounded_ones() {
        let agent = DecisionAgent::new(store_with_fact().await, MockReasoner);
        let ans = agent.answer("Who owns the Acme account?", None).await.unwrap();
        // The fabricated revenue claim is dropped; the grounded ownership claim survives.
        assert_eq!(ans.context.verified.len(), 1);
        assert_eq!(ans.context.dropped.len(), 1);
        assert!(ans.context.dropped[0].contains("70M"));
        assert_eq!(ans.context.verified[0].provenance, vec!["ART-1".to_string()]);
        assert!(ans.answer.contains("Maya"));
        assert_eq!(ans.provenance, vec!["ART-1".to_string()]);
        assert_eq!(ans.context.llm_calls, 4);
    }

    #[tokio::test]
    async fn empty_evidence_yields_honest_abstention() {
        let store = Arc::new(GbrainStore::open_in_memory(Embedder::new(None)).unwrap());
        struct NoClaims;
        #[async_trait::async_trait]
        impl Reasoner for NoClaims {
            async fn complete(&self, system: &str, _u: &str) -> anyhow::Result<Completion> {
                Ok(Completion::text(if system == DECOMPOSE_SYS { "[]" } else { "[]" }))
            }
        }
        let agent = DecisionAgent::new(store, NoClaims);
        let ans = agent.answer("anything?", None).await.unwrap();
        assert!(ans.context.verified.is_empty());
        assert!(ans.answer.to_lowercase().contains("not contain enough evidence"));
    }
}
