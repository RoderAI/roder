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

use roder_api::memory::MemoryScope;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::model::AsOf;
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
}

impl<R: Reasoner> DecisionAgent<R> {
    pub fn new(store: Arc<GbrainStore>, reasoner: R) -> Self {
        Self {
            store,
            reasoner,
            budget: AgentBudget::default(),
            progress: Box::new(SilentProgress),
            scope: None,
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
        let mut calls = 0usize;
        let as_of_label = as_of.map(|d| d.date().to_string());

        // 1. Decompose into evidence needs.
        self.progress.step("decompose", question);
        let mut subqueries = self.decompose(question).await.unwrap_or_default();
        calls += 1;
        subqueries.truncate(self.budget.max_subqueries);

        // 2. Multi-pass retrieval -> deduped evidence pool.
        self.progress.step("retrieve", &format!("{} sub-queries", subqueries.len()));
        let evidence = self.gather_evidence(question, &subqueries, as_of).await?;

        // 3. Draft atomic, cited claims.
        self.progress.step("draft", &format!("{} evidence records", evidence.len()));
        let claims = self.draft(question, &evidence).await?;
        calls += 1;
        let drafted = claims.len();

        // 4. Verify each claim against its cited artifact text; prune unsupported.
        self.progress.step("verify", &format!("{drafted} claims"));
        let (verified, dropped) = self.verify(question, &claims, &evidence).await?;
        calls += 1;

        // 5. Temporal checks (contradiction / supersession) when relevant.
        let contradictions = self.temporal_checks(&evidence).await;

        // 6. Synthesize the final answer from verified claims only.
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
        let out = self.reasoner.complete(system, user).await?;
        if std::env::var("GBRAIN_AGENT_DEBUG").is_ok() {
            eprintln!(
                "── agent[{stage}] ──\n{}\n",
                out.chars().take(2000).collect::<String>()
            );
        }
        Ok(out)
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
    ) -> anyhow::Result<Vec<EvidenceItem>> {
        let as_of = as_of.map(AsOf::at).unwrap_or_else(AsOf::now);
        let mut seen = std::collections::HashSet::new();
        let mut pool: Vec<EvidenceItem> = Vec::new();
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
                    expand: true, // evidence gathering wants the full cluster
                })
                .await?;
            let now = result.now;
            for hit in result.hits {
                let id = hit.fact.id.clone();
                if !seen.insert(id) {
                    continue;
                }
                if pool.len() >= self.budget.evidence_pool_cap {
                    break;
                }
                let slug = hit.fact.provenance.first().cloned().unwrap_or_else(|| hit.fact.id.clone());
                let source = hit
                    .fact
                    .metadata
                    .get("source_type")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_default();
                let author = hit
                    .fact
                    .metadata
                    .get("author")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let source = if author.is_empty() {
                    source
                } else if source.is_empty() {
                    author.to_string()
                } else {
                    format!("{source} / {author}")
                };
                pool.push(EvidenceItem {
                    index: pool.len() + 1,
                    slug,
                    date: hit.fact.valid_at.date().to_string(),
                    source,
                    status: crate::store::status_label(&hit.fact, now).to_string(),
                    text: hit.fact.text.clone(),
                });
            }
        }
        Ok(pool)
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

    async fn temporal_checks(&self, evidence: &[EvidenceItem]) -> Vec<String> {
        // Surface store-detected contradictions among the retrieved subjects so
        // the synthesizer can report conflicts faithfully (C6) rather than
        // silently picking one side.
        let scope = self.scope.clone();
        let Ok(pairs) = self.store.contradictions(scope, None, 8).await else {
            return Vec::new();
        };
        let slugs: std::collections::HashSet<&str> =
            evidence.iter().map(|e| e.slug.as_str()).collect();
        pairs
            .into_iter()
            .filter(|p| {
                let a = p.a.provenance.first().map(String::as_str).unwrap_or("");
                let b = p.b.provenance.first().map(String::as_str).unwrap_or("");
                slugs.contains(a) || slugs.contains(b)
            })
            .map(|p| format!("\"{}\" conflicts with \"{}\"", p.a.text.trim(), p.b.text.trim()))
            .collect()
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

const DRAFT_SYS: &str = "You answer ONLY from the numbered evidence records provided. Output a JSON array \
of atomic claims. Each claim is an object {\"text\": <one factual statement>, \"support\": [<record numbers>]}. \
HARD RULES: every claim MUST cite at least one record number that actually states it; never include a \
name, date, number, or causal link that is not present in a cited record; if you are unsure, omit the claim. \
Prefer fewer, well-supported claims over many speculative ones. Output ONLY the JSON array.";

const VERIFY_SYS: &str = "You are a strict grounding verifier. For each claim you are given its full cited \
record text. Decide if those records DIRECTLY state the claim. Output a JSON array of \
{\"id\": <claim number>, \"supported\": <true|false>, \"classification\": <\"direct\"|\"inferred\">, \
\"reason\": <short>}. Mark supported=false if the specific detail (exact date, name, figure, or causal \
link) is not explicitly in the cited records — plausibility is NOT support. \"direct\" = the cited record \
is first-hand testimony/the document itself; \"inferred\" = the claim is deduced from indirect evidence.";

const FINALIZE_SYS: &str = "You write the final answer using ONLY the verified claims provided (each with its \
provenance). Add NOTHING beyond them — no extra dates, names, events, or figures. Be specific and complete. \
If the question asks to walk through or justify with evidence, enumerate each piece and label it direct vs \
inferred. If contradictions are listed, report both sides rather than choosing one. Cite provenance ids \
inline where natural.";

fn draft_prompt(question: &str, evidence: &[EvidenceItem]) -> String {
    let mut s = format!("Question: {question}\n\nEvidence records:\n");
    for e in evidence {
        s.push_str(&format!(
            "[{}] ({}, {}, status={}) {}\n",
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
    use crate::store::CaptureInput;
    use roder_api::memory::MemoryScope;

    /// Deterministic Reasoner that returns canned JSON per stage (matched on the
    /// system prompt) so the loop's plumbing is testable without an LLM.
    struct MockReasoner;

    #[async_trait::async_trait]
    impl Reasoner for MockReasoner {
        async fn complete(&self, system: &str, _user: &str) -> anyhow::Result<String> {
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
            Ok(out.to_string())
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
            async fn complete(&self, system: &str, _u: &str) -> anyhow::Result<String> {
                Ok(if system == DECOMPOSE_SYS { "[]" } else { "[]" }.to_string())
            }
        }
        let agent = DecisionAgent::new(store, NoClaims);
        let ans = agent.answer("anything?", None).await.unwrap();
        assert!(ans.context.verified.is_empty());
        assert!(ans.answer.to_lowercase().contains("not contain enough evidence"));
    }
}
