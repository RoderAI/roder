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

use crate::ground::{
    GroundIndex, GroundingAudit, audit_grounding, build_ground_index, is_walkthrough_question,
    safe_specifics,
};
use crate::model::{AsOf, TemporalFact};
use crate::reason::{Reasoner, extract_json};
use crate::store::{GbrainStore, RecallParams};

pub mod claims;

use self::claims::{
    ClaimConfidence, ClaimTemporalScope, ClaimType, EvidenceRecord, LedgerClaim, QuoteSpan,
    validate_claim_ledger,
};

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
            // Retrieval is the real faithfulness+coverage bottleneck: at limit 8
            // only ~0.58-0.75 of ground-truth evidence reaches the answerer, so it
            // must fabricate the rest. limit 16 (+ expand) lifts recall to ~0.87.
            retrieval_limit: 16,
            evidence_pool_cap: 40,
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
    /// Verbatim span from a cited record that proves the claim (forces grounding).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub quote: String,
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
    strict_faithfulness: bool,
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
            strict_faithfulness: false,
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

    pub fn with_strict_faithfulness(mut self, strict: bool) -> Self {
        self.strict_faithfulness = strict;
        self
    }

    /// Run the full decision loop for one question.
    pub async fn answer(
        &self,
        question: &str,
        as_of: Option<OffsetDateTime>,
    ) -> anyhow::Result<AgentAnswer> {
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
        self.progress
            .step("retrieve", &format!("{} sub-queries", subqueries.len()));
        let (evidence, contradictions) = self.gather_evidence(question, &subqueries, as_of).await?;

        // 3. Draft atomic, cited claims.
        self.progress
            .step("draft", &format!("{} evidence records", evidence.len()));
        let claims = self.draft(question, &evidence).await?;
        calls += 1;
        let drafted = claims.len();

        // 4. Verify each claim against its cited artifact text; prune unsupported.
        self.progress.step("verify", &format!("{drafted} claims"));
        let (verified, dropped) = self.verify(question, &claims, &evidence).await?;
        calls += 1;

        // 5. Synthesize the final answer from verified claims only.
        self.progress
            .step("finalize", &format!("{} verified claims", verified.len()));
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

    /// Token-LIGHT path: retrieve (no LLM) then a SINGLE concise+faithful
    /// synthesis call. Same call count as a plain answerer, but we own the
    /// prompt — so the answer is focused and grounded instead of an info-dump.
    /// This is the default; the multi-pass [`answer`] loop is opt-in.
    pub async fn answer_concise(
        &self,
        question: &str,
        as_of: Option<OffsetDateTime>,
    ) -> anyhow::Result<AgentAnswer> {
        self.tokens_in.store(0, Ordering::Relaxed);
        self.tokens_out.store(0, Ordering::Relaxed);
        let as_of_label = as_of.map(|d| d.date().to_string());

        // Retrieval only (no decompose call — keep it cheap).
        self.progress.step("retrieve", question);
        let (evidence, contradictions) = self.gather_evidence(question, &[], as_of).await?;

        self.progress
            .step("synthesize", &format!("{} records", evidence.len()));
        let evidence_empty = evidence.is_empty();
        let answer = if evidence_empty {
            "The available records do not contain enough evidence to answer this.".to_string()
        } else {
            let synth_prompt =
                concise_prompt(question, &evidence, &contradictions, as_of_label.as_deref());
            // Self-consistency (opt-in, GBRAIN_SELF_CONSISTENCY=N>=2): synthesize N
            // independent drafts and keep only facts a MAJORITY assert. Stochastic
            // fabrications vary run-to-run so they drop out; stable grounded facts
            // survive in every run, preserving coverage — a frontier-shifter the
            // single-draft strip can't be.
            let n_self = std::env::var("GBRAIN_SELF_CONSISTENCY")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|n| (2..=4).contains(n))
                .unwrap_or(1);
            let draft = if n_self >= 2 {
                let mut drafts = Vec::with_capacity(n_self);
                for i in 0..n_self {
                    self.progress
                        .step("synthesize", &format!("sample {}/{n_self}", i + 1));
                    drafts.push(self.call("synthesize", CONCISE_SYS, &synth_prompt).await?);
                }
                self.progress
                    .step("consensus", &format!("{n_self} samples"));
                self.call(
                    "consensus",
                    CONSENSUS_SYS,
                    &consensus_prompt(question, &drafts),
                )
                .await?
            } else {
                self.call("synthesize", CONCISE_SYS, &synth_prompt).await?
            };
            // Free deterministic guard: drop slug-shaped cites not in the pool.
            let draft = strip_phantom_cites(&draft, &evidence);
            let idx = build_ground_index(&evidence);
            let walk = is_walkthrough_question(question);
            let audit_disabled = std::env::var("GBRAIN_NO_GROUNDING_AUDIT").is_ok();
            let audit = |a: &str| {
                if audit_disabled {
                    GroundingAudit::default()
                } else {
                    audit_grounding(question, a, &idx, walk)
                }
            };

            // Pass 1: lenient, deterministic-flag-driven specific strip.
            self.progress.step("strip", "audit");
            let stripped = self
                .call(
                    "strip",
                    STRIP_SYS,
                    &strip_prompt(question, &draft, &evidence, &audit(&draft)),
                )
                .await?;
            let stripped = strip_phantom_cites(&stripped, &evidence);

            // Pass 2 (opt-in via GBRAIN_FAITHFUL_VERIFY=1): adversarial DELETE-ONLY
            // concept/causal verify. Default OFF — on the reliable medium tier it
            // was net-negative (dropped rubric-credited content, overall 0.62 < the
            // 0.65 bar) despite the one-way-ratchet guard; the small-tier gain was
            // n=11 noise. Kept available for experimentation.
            let verify_on = std::env::var("GBRAIN_FAITHFUL_VERIFY")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("on"))
                .unwrap_or(false);
            if !verify_on {
                stripped
            } else {
                self.progress.step("verify-faithful", "adversarial");
                let verified = self
                    .call(
                        "verify-faithful",
                        VERIFY_FAITHFUL_SYS,
                        &faithful_verify_prompt(question, &stripped, &evidence, &audit(&stripped)),
                    )
                    .await?;
                let verified = strip_phantom_cites(&verified, &evidence);
                if accept_faithful_pass(question, &stripped, &verified, &idx, walk) {
                    verified
                } else {
                    stripped
                }
            }
        };

        // Provenance = the cited records that actually appear in the answer.
        let provenance: Vec<String> = evidence
            .iter()
            .filter(|e| answer.contains(&e.slug))
            .map(|e| e.slug.clone())
            .collect();
        let provenance = if provenance.is_empty() {
            // No inline cites surfaced — fall back to the retrieved set (capped).
            evidence.iter().take(6).map(|e| e.slug.clone()).collect()
        } else {
            dedup_strings(provenance)
        };

        let context = WorkingContext {
            question: question.to_string(),
            as_of: as_of_label,
            subqueries: Vec::new(),
            evidence,
            drafted: 0,
            verified: Vec::new(),
            dropped: Vec::new(),
            contradictions,
            llm_calls: if evidence_empty {
                0
            } else {
                let n_self = std::env::var("GBRAIN_SELF_CONSISTENCY")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .filter(|n| (2..=4).contains(n))
                    .unwrap_or(1);
                let synth = if n_self >= 2 { n_self + 1 } else { 1 }; // samples + consensus
                let verify = usize::from(
                    std::env::var("GBRAIN_FAITHFUL_VERIFY")
                        .map(|v| v == "1" || v.eq_ignore_ascii_case("on"))
                        .unwrap_or(false),
                );
                synth + 1 + verify // + strip
            },
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
        self.tokens_in
            .fetch_add(completion.input_tokens, Ordering::Relaxed);
        self.tokens_out
            .fetch_add(completion.output_tokens, Ordering::Relaxed);
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
        // Always expand the event cluster: it lifts ground-truth recall ~0.58->0.75
        // at limit 8 and the grounding audit + concise prompt handle precision. The
        // retrieval pool is the bottleneck, so favour recall.
        let contra = is_contradiction_question(question);
        let expand = true;
        // For a clash/dispute, add a per-named-party sub-query so EACH side is
        // surfaced even when the opposing statement lives on a different record
        // than the agreement (the C6 root cause: only one side was retrieved).
        let mut subs: Vec<String> = subqueries.to_vec();
        if contra {
            for name in contradiction_parties(question) {
                subs.push(format!("{name} position statement"));
            }
        }
        let mut pool: Vec<EvidenceItem> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut contradictions = std::collections::BTreeSet::new();

        // Primary pass: the as-of snapshot (or current).
        let primary = as_of.map(AsOf::at).unwrap_or_else(AsOf::now);
        self.retrieve_into(
            question,
            &subs,
            primary,
            expand,
            "",
            &mut pool,
            &mut seen,
            &mut contradictions,
        )
        .await?;

        // As-of questions also need the CURRENT state to answer "what has SINCE
        // changed" (C4 audit replay). Records only present now are flagged.
        if let Some(d) = as_of {
            let note = format!(
                "NOT on record as of {} — recorded later / current state",
                d.date()
            );
            self.retrieve_into(
                question,
                &subs,
                AsOf::now(),
                expand,
                &note,
                &mut pool,
                &mut seen,
                &mut contradictions,
            )
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
                    slug: hit
                        .fact
                        .provenance
                        .first()
                        .cloned()
                        .unwrap_or_else(|| hit.fact.id.clone()),
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
            .call(
                "verify",
                VERIFY_SYS,
                &verify_prompt(question, claims, evidence),
            )
            .await?;
        let verdicts = extract_json(&out)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let mut kept = Vec::new();
        let mut dropped = Vec::new();
        let evidence_records = if self.strict_faithfulness {
            Some(
                evidence
                    .iter()
                    .map(evidence_record_from_item)
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        for (i, claim) in claims.iter().enumerate() {
            let verdict = verdicts
                .iter()
                .find(|v| v.get("id").and_then(Value::as_u64).map(|x| x as usize) == Some(i + 1));
            let supported = verdict
                .and_then(|v| v.get("supported"))
                .and_then(Value::as_bool)
                // No verdict for a claim => conservatively drop it.
                .unwrap_or(false);
            let quote = verdict
                .and_then(|v| v.get("quote"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            // Keep a claim if the adversarial verifier marked it supported. The
            // quote is captured for telemetry/grounding but is NOT a hard gate —
            // requiring a verbatim quote over-pruned true paraphrased facts and
            // caused false abstention (C2/C3 regressions).
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
                if let Some(records) = evidence_records.as_ref() {
                    let ledger_claim = ledger_claim_from_verdict(
                        i + 1,
                        question,
                        claim,
                        evidence,
                        quote.as_str(),
                        classification.as_deref(),
                    );
                    let trace = validate_claim_ledger(&[ledger_claim], records);
                    if let Some(reason) = trace
                        .rejected
                        .first()
                        .and_then(|claim| claim.rejection_reason.clone())
                    {
                        dropped.push(format!("{} ({reason})", claim.text));
                        continue;
                    }
                    if trace.verified.is_empty() {
                        dropped.push(format!("{} (strict ledger rejected claim)", claim.text));
                        continue;
                    }
                }
                kept.push(VerifiedClaim {
                    text: claim.text.clone(),
                    provenance,
                    classification,
                    quote,
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
            return Ok(
                "The available records do not contain enough evidence to answer this.".to_string(),
            );
        }
        self.call(
            "finalize",
            FINALIZE_SYS,
            &finalize_prompt(question, verified, contradictions, as_of),
        )
        .await
    }
}

fn dedup_strings(iter: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    iter.into_iter()
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

fn evidence_record_from_item(item: &EvidenceItem) -> EvidenceRecord {
    EvidenceRecord::new(item.index, item.slug.clone(), item.text.clone())
        .with_date(item.date.clone())
        .with_status(item.status.clone())
        .with_note(item.note.clone())
}

fn ledger_claim_from_verdict(
    id: usize,
    question: &str,
    claim: &Claim,
    evidence: &[EvidenceItem],
    quote: &str,
    classification: Option<&str>,
) -> LedgerClaim {
    let supporting_record_numbers: Vec<usize> = claim
        .support
        .iter()
        .copied()
        .filter(|n| evidence.get(n.saturating_sub(1)).is_some())
        .collect();
    let supporting_artifact_ids = dedup_strings(
        supporting_record_numbers
            .iter()
            .filter_map(|n| evidence.get(n.saturating_sub(1)).map(|e| e.slug.clone())),
    );
    let quote_spans = quote_spans_for(quote, &supporting_record_numbers, evidence);
    LedgerClaim {
        claim_id: format!("claim-{id}"),
        claim_text: claim.text.clone(),
        claim_type: claim_type_from(question, classification, supporting_record_numbers.len()),
        supporting_artifact_ids,
        supporting_record_numbers,
        quote_spans,
        temporal_scope: temporal_scope_from_question(question),
        confidence: ClaimConfidence::Rejected,
        rejection_reason: None,
    }
}

fn quote_spans_for(
    quote: &str,
    supporting_record_numbers: &[usize],
    evidence: &[EvidenceItem],
) -> Vec<QuoteSpan> {
    let quote = quote.trim();
    if quote.is_empty() {
        return Vec::new();
    }
    supporting_record_numbers
        .iter()
        .filter_map(|n| evidence.get(n.saturating_sub(1)))
        .filter(|item| normalized_contains_light(&item.text, quote))
        .map(|item| QuoteSpan {
            artifact_id: item.slug.clone(),
            record_number: item.index,
            quote: quote.to_string(),
        })
        .take(1)
        .collect()
}

fn claim_type_from(
    question: &str,
    classification: Option<&str>,
    support_count: usize,
) -> ClaimType {
    let ql = question.to_ascii_lowercase();
    if is_contradiction_question(question) {
        ClaimType::Contradiction
    } else if ql.contains("as of")
        || ql.contains("changed")
        || ql.contains("change")
        || ql.contains("current")
        || ql.contains("now")
        || ql.contains("superseded")
        || ql.contains("replaced")
    {
        ClaimType::TemporalStatus
    } else if support_count > 1 {
        ClaimType::Derived
    } else if classification == Some("direct") {
        ClaimType::Direct
    } else {
        ClaimType::Derived
    }
}

fn temporal_scope_from_question(question: &str) -> ClaimTemporalScope {
    let ql = question.to_ascii_lowercase();
    if ql.contains("as of")
        && (ql.contains("since")
            || ql.contains("changed")
            || ql.contains("change")
            || ql.contains("now")
            || ql.contains("current")
            || ql.contains("superseded")
            || ql.contains("replaced"))
    {
        ClaimTemporalScope::SinceAsOf
    } else if ql.contains("as of") {
        ClaimTemporalScope::AsOf
    } else if ql.contains("current") || ql.contains("now") {
        ClaimTemporalScope::Current
    } else {
        ClaimTemporalScope::Unknown
    }
}

fn normalized_contains_light(haystack: &str, needle: &str) -> bool {
    let needle = normalize_light(needle);
    !needle.is_empty() && normalize_light(haystack).contains(&needle)
}

fn normalize_light(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = true;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
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

const VERIFY_SYS: &str = "You are an ADVERSARIAL fact-checker. Assume each claim is UNSUPPORTED until proven. \
For each claim you get its cited record text (with source type). Output a JSON array of {\"id\": <claim \
number>, \"supported\": <true|false>, \"quote\": <a short VERBATIM span from a cited record that proves the \
claim, or \"\">, \"classification\": <\"direct\"|\"inferred\">, \"reason\": <short>}. RULE: supported=true \
ONLY if EVERY specific in the claim — each name, exact date, number, quoted phrase, and causal link — appears \
EXPLICITLY in a cited record (paste it in \"quote\"). If the record is vague, only implies it, or any single \
specific (a date, a number, a name, a 'because') is missing or different, supported=false. Plausibility, \
inference, and 'close enough' are NOT support. CLASSIFICATION: \"direct\" = the cited record is a FIRST-HAND \
account by the actor themselves (a Slack/chat message or email WRITTEN BY the person who decided/witnessed \
it). \"inferred\" = derived from a SECONDARY record — meeting notes, incident report, post-mortem, summary, \
document, or third-party mention. Meeting notes and reports are \"inferred\", not direct.";

const FINALIZE_SYS: &str = "Write the final answer using ONLY the verified claims (each with provenance and \
any classification). Add NOTHING beyond them — no extra dates, names, events, or figures. Be specific and \
COMPLETE: address EVERY part the question asks (e.g. who, when, what, alternatives, rationale, current \
status). If the question asks to walk through or justify the evidence, enumerate each evidence item with its \
provenance id and label it direct vs inferred using the provided classification. If the question is 'as of \
<date>', state what was known THEN and, for records marked 'recorded later / current state', say for each \
whether it is still current or has SINCE changed/been replaced. If contradictions are listed, report both \
sides. Do not pad with tangential records. Cite provenance ids inline.";

const CONCISE_SYS: &str = "You answer organizational-memory questions ONLY from the retrieved records below; \
you have NO background knowledge of this company — if a fact is not in a listed record, it does not exist. \
RULES: 1) RELEVANT & COMPLETE — include EVERY piece of information the question asks for (who AND when AND \
what AND why AND which alternatives AND current/resolution status) when the records support it, and EXCLUDE \
everything else. Tangential or 'maybe relevant' facts are WRONG, not thorough — every sentence must earn its \
place by answering part of the question. Length should match what the question needs: don't pad, don't drop a \
relevant part. 2) FAITHFUL — every specific (proper name, exact date, clock time, number, percentage, money/ \
credit figure, version number, quoted phrase, causal link) MUST be copyable from a record; write the [SLUG] of \
the ONE record that contains a specific immediately after THAT specific. If a sentence mixes specifics from \
two records, split it so each specific sits beside the record that states it. If a specific is not in the \
records, OMIT it — never guess or infer. Do NOT add clock timestamps (HH:MM), severity labels, penalty/credit/ \
revenue figures, or attendee names not written in a record, and never carry a person or date from one record \
onto a fact stated by a different record. 3) If the records \
genuinely do not answer, say so in one sentence. 4) 'now'/'currently' => give the most recent/current fact; \
'as of <date>' => state what was known THEN, then note what has SINCE changed. Report a change ONLY where a \
record EXPLICITLY states that a specific earlier fact was replaced/revised/superseded — do NOT treat every \
later record as a change, do NOT infer or enumerate a sequence of changes the records do not state, and do \
NOT invent dates/IDs for changes. Words like 'subsequently / later / then / a further change / phase / \
superseded' are FORBIDDEN unless a single record's text literally states the earlier fact was replaced (and \
that record's [SLUG] must carry the statement). If no record explicitly states a change to a fact, say it is \
unchanged / still in effect. 5) ONLY when explicitly \
asked to 'walk through' or 'justify with evidence', list each supporting record with its [SLUG], labeled \
direct (a first-hand message/email by the actor) or inferred (meeting notes, report, summary, third-party). \
The records belong to several distinct events (event id = the slug without its final -NNN segment); ONE \
conclusion is established by ONE event's artifacts — list only those, never add records from other events even \
if topically similar, and never invent an extra case/incident. Otherwise do NOT enumerate records. 6) CONFLICT/DISPUTE questions: report EACH named party's own position \
separately with its date and that party's [SLUG]; state whether the positions actually conflict; give \
resolution status as exactly one of resolved / superseded / still unresolved with the resolving record's \
[SLUG], or 'no retrieved record resolves it'. NEVER conclude 'no contradiction exists' merely because the \
records you read first agree. If a named party's position is genuinely absent from the records, SAY it is not \
in the retrieved records — do NOT invent it, and do NOT assume the parties agreed unless a record explicitly \
shows agreement.";

const VERIFY_FAITHFUL_SYS: &str = "You are an ADVERSARIAL FAITHFULNESS VERIFIER running a final DELETE-ONLY \
pass. You receive numbered RECORDS (the ONLY source of truth) and a DRAFT whose specific values were already \
checked. Your sole job: DELETE every clause whose CONTENT cannot be traced to a record — even when it names no \
date or number — while KEEPING every clause that paraphrases record content. METHOD — read ONE clause at a \
time and ask: does at least one record actually state this concept / event / relationship? A faithful \
paraphrase preserves a record's meaning and IS supported — keep it; you are NOT matching words and you must \
NEVER require a verbatim quote. Treat as GUILTY-until-proven and delete unless you can point to a record \
stating its content: (a) a FRAMING NOUN or distinctive named thing the records never introduce — a tool, \
system, document, channel, market, meeting, audit, review, thread, prototype, anomaly, or product/tool name \
(e.g. 'a Slack thread', 'investor pressure', 'the freight-forwarding market', 'a mid-morning engineering-lead \
meeting', 'Figma prototype', 'Confluence path', 'Snowflake'); (b) a CAUSAL or motive claim ('because', 'due \
to', 'driven by', 'as a result of', 'in order to', 'the reason was'); (c) an ATTRIBUTED event / quote / \
participant LIST (someone said / flagged / confirmed / attended / proposed X) — a record of that exact event \
must exist; (d) any date, time, number, percentage, money/credit/rate figure, version, or person not in the \
records. MISATTRIBUTION is ungrounded too: if a value / event / link IS in some record but the draft attaches \
it to a DIFFERENT actor, date, record, or event than that record states, delete the misattached part. If \
deleting a fragment leaves a broken sentence, delete the whole sentence. HARD CONSTRAINTS — you are an EDITOR, \
not an author: ONLY delete; NEVER add, rephrase, reorder, merge, or 'correct' anything; every word you keep \
must be copied verbatim from the DRAFT; NEVER introduce a date, name, number, [SLUG], or word not already in \
the draft; keep correct [SLUG] cites on their facts and NEVER move a [SLUG]. Do NOT delete supported content \
that answers the question — removing a grounded fact is as wrong as keeping a fabrication; when a clause is \
grounded, keep it untouched. Do NOT invent or assert a supersession, a resolution, or 'no contradiction'; for \
conflict questions keep only each party's stated position and say a position is 'not in the retrieved records' \
if no record states it. The prompt may list GROUNDING-AUDIT spans already proven absent or misattributed — \
delete each and any clause depending on it. If after deleting nothing grounded remains, output exactly: The \
available records do not contain enough evidence to answer this. Otherwise output ONLY the edited answer text \
— no preamble, no notes, no claim list.";

const STRIP_SYS: &str = "You are a STRICT faithfulness editor. You receive numbered RECORDS and a DRAFT \
answer. Return a CORRECTED answer that keeps every well-supported statement VERBATIM but removes anything the \
records do not support. RULES: 1) Check EVERY specific in the draft — each proper name, exact date, clock time \
(HH:MM), number, percentage, money/credit figure, version number, quoted phrase, severity label, [SLUG] \
citation, and causal link ('because'/'due to'/'driven by') — against the record text. If a specific does not \
appear in any record, or appears there with a DIFFERENT value, DELETE that specific; if removing it leaves the \
sentence unsupported, delete the whole sentence. Never carry a name or date from one record onto a fact stated \
by a different record. Also DELETE any asserted change / 'superseding event' / replacement that no record \
EXPLICITLY states — treating a later record as a 'change' to an earlier fact is fabrication; keep only \
changes a record spells out. 2) Do NOT delete a statement merely because it is paraphrased — KEEP it if every \
specific it asserts is present in some record. Remove only CLEAR fabrications and value mismatches, not \
faithful summaries. 3) NEVER add a fact, name, date, number, or [SLUG] not already in the draft; never \
rephrase supported content; keep correct [SLUG] cites attached to their facts. 4) CONFLICT/DISPUTE answers: if \
the draft claims 'no contradiction exists' or invents one party's position or a resolution the records do not \
state, correct it — state only what the records show, and say a party's position is 'not in the retrieved \
records' if it is genuinely absent; never invent the missing side or a resolution. 5) AUTOMATED GROUNDING \
AUDIT: the prompt may list spans that were string-checked against the records. The 'FABRICATED (absent from \
every record)' list is AUTHORITATIVE — remove each listed span and any clause or sentence that depends on it; \
never re-add or relabel it. EXCEPTION: keep a span only if it is plainly the SAME value in a trivially \
different surface form already present in a record. For the 'MISATTRIBUTED' list, re-read the EXACT [SLUG] \
beside the span; if that record does not contain the value, delete the value (it belongs to a different \
record) — do not move the [SLUG]. For 'OFF-CLUSTER records' in a walk-through answer, drop those records' \
bullets unless their own text directly states the conclusion being justified. 6) Keep it relevant and output \
ONLY the corrected answer text, no preamble or notes.";

const CONSENSUS_SYS: &str = "You are given several INDEPENDENT answers to the SAME question, each written from \
the SAME records. Produce ONE consolidated answer containing ONLY facts asserted by a MAJORITY of the inputs. \
A specific (name, date, number, percentage, event, causal claim, attribution, quote) that appears in only ONE \
input is unreliable — DROP it. Keep every fact the inputs agree on (preserve coverage), stay concise, and copy \
[SLUG] citations exactly as they appear. NEVER add a fact, name, date, number, or [SLUG] not present in a \
majority of the inputs; never invent a reconciliation. If the inputs broadly agree, return their shared \
content; if they mostly disagree, return only the few facts they share. For 'as of <date>' / change questions \
keep only changes a majority state. Output ONLY the consolidated answer, no preamble.";

fn consensus_prompt(question: &str, drafts: &[String]) -> String {
    let mut s = format!(
        "Question: {question}\n\nYou are given {} independent answers from the same records. Keep ONLY what a \
         MAJORITY agree on; drop anything in just one.\n",
        drafts.len()
    );
    for (i, d) in drafts.iter().enumerate() {
        s.push_str(&format!("\n--- ANSWER {} ---\n{}\n", i + 1, d));
    }
    s.push_str("\nReturn the consolidated answer.");
    s
}

fn concise_prompt(
    question: &str,
    evidence: &[EvidenceItem],
    contradictions: &[String],
    as_of: Option<&str>,
) -> String {
    let mut s = String::new();
    if let Some(d) = as_of {
        s.push_str(&format!("This question is about knowledge AS OF {d}.\n"));
    }
    s.push_str(&format!("Question: {question}\n\nRecords:\n"));
    for e in evidence {
        let note = if e.note.is_empty() {
            String::new()
        } else {
            format!(", {}", e.note)
        };
        s.push_str(&format!(
            "[{}] ({}, {}, status={}{note}) {}\n",
            e.slug,
            e.date,
            if e.source.is_empty() {
                "source?"
            } else {
                &e.source
            },
            e.status,
            truncate(&e.text, 6000),
        ));
    }
    if !contradictions.is_empty() {
        s.push_str("\nConflicting records to report if relevant:\n");
        for c in contradictions {
            s.push_str(&format!("  - {c}\n"));
        }
    }
    if is_contradiction_question(question) {
        let parties = contradiction_parties(question);
        if !parties.is_empty() {
            s.push_str(
                "\nThis is a CONFLICT/DISPUTE question. Attribute EACH party's own position \
                 separately (with its date + [SLUG]), say whether they actually conflict, and give \
                 the resolution status. Parties asked about:\n",
            );
            for p in &parties {
                s.push_str(&format!("  - {p}\n"));
            }
        }
    }
    s.push_str("\nWrite the relevant, faithful, directly-responsive answer.");
    s
}

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
            if e.source.is_empty() {
                "source?"
            } else {
                &e.source
            },
            e.status,
            truncate(&e.text, 6000),
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
/// Clash/dispute question naming >=2 people — triggers conflict retrieval +
/// scaffold. EXCLUDES supersession/audit phrasings to protect the C4 win.
fn is_contradiction_question(question: &str) -> bool {
    let ql = question.to_lowercase();
    const CLASH: &[&str] = &[
        "clash",
        "dispute",
        "disagree",
        "differing position",
        "differing positions",
        "conflicting position",
        "what position did",
        "the conflict",
        "objected",
        "opposed",
        "contradict",
        " versus ",
        " vs ",
        "both sides",
        "differing",
    ];
    CLASH.iter().any(|m| ql.contains(m)) && proper_name_phrases(question).len() >= 2
}

fn contradiction_parties(question: &str) -> Vec<String> {
    proper_name_phrases(question)
}

/// Adjacent Capitalized tokens => a "First Last" proper-name span.
fn proper_name_phrases(question: &str) -> Vec<String> {
    let ws: Vec<&str> = question.split_whitespace().collect();
    // Drop a trailing possessive ("Torres's" -> "Torres") then trim punctuation.
    let clean = |w: &str| {
        w.split('\'')
            .next()
            .unwrap_or(w)
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_string()
    };
    let cap = |w: &str| {
        w.chars().next().is_some_and(char::is_uppercase)
            && w.chars().skip(1).any(|c| c.is_lowercase())
    };
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 1 < ws.len() {
        let (a, b) = (clean(ws[i]), clean(ws[i + 1]));
        if a.len() > 1 && b.len() > 1 && cap(&a) && cap(&b) {
            out.push(format!("{a} {b}"));
            i += 2;
        } else {
            i += 1;
        }
    }
    dedup_strings(out)
}

/// Drop bracketed cites that look like an artifact slug (have '-' and a digit)
/// but are NOT in the retrieved pool — invented ids. Zero LLM cost.
fn strip_phantom_cites(answer: &str, evidence: &[EvidenceItem]) -> String {
    let real: std::collections::HashSet<&str> = evidence.iter().map(|e| e.slug.as_str()).collect();
    let mut out = String::with_capacity(answer.len());
    let mut rest = answer;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        if let Some(rel) = rest[open + 1..].find(']') {
            let inner = &rest[open + 1..open + 1 + rel];
            let slug_shaped = inner.contains('-')
                && inner.chars().any(|c| c.is_ascii_digit())
                && inner
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if slug_shaped && !real.contains(inner) {
                if out.ends_with(' ') {
                    out.pop();
                }
            } else {
                out.push_str(&rest[open..open + 1 + rel + 1]);
            }
            rest = &rest[open + 1 + rel + 1..];
        } else {
            out.push_str(&rest[open..]);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

fn strip_prompt(
    question: &str,
    draft: &str,
    evidence: &[EvidenceItem],
    audit: &GroundingAudit,
) -> String {
    audit_edit_prompt(
        question,
        draft,
        evidence,
        audit,
        "DRAFT ANSWER TO AUDIT (correct it; keep supported parts verbatim):",
    )
}

/// Accept the extra delete-only verify pass ONLY if it cannot have hurt coverage:
/// it stayed delete-only, did not collapse, preserved EVERY correctly-attributed
/// grounded specific, and introduced no new deterministic fabrication. Otherwise
/// the caller keeps the strip output — so the extra pass is a one-way faithfulness
/// ratchet that can never regress overall below the v4 strip's coverage floor.
fn accept_faithful_pass(
    question: &str,
    before: &str,
    after: &str,
    idx: &GroundIndex,
    walk: bool,
) -> bool {
    let nz = |s: &str| s.chars().filter(|c| !c.is_whitespace()).count();
    let (a, b) = (nz(before), nz(after));
    if b == 0 || b > a {
        return false; // emptied, or grew => not delete-only
    }
    if a >= 80 && (b as f32) < 0.45 * (a as f32) {
        return false; // removed >55% => over-prune
    }
    let after_lc = after.to_lowercase();
    for g in safe_specifics(question, before, idx, walk) {
        if !after_lc.contains(&g.to_lowercase()) {
            return false; // dropped a correctly-attributed grounded fact
        }
    }
    let f_before = audit_grounding(question, before, idx, walk)
        .fabricated
        .len();
    let f_after = audit_grounding(question, after, idx, walk).fabricated.len();
    f_after <= f_before
}

/// The adversarial DELETE-ONLY faithfulness verify prompt (call #3).
fn faithful_verify_prompt(
    question: &str,
    draft: &str,
    evidence: &[EvidenceItem],
    audit: &GroundingAudit,
) -> String {
    audit_edit_prompt(
        question,
        draft,
        evidence,
        audit,
        "DRAFT TO VERIFY (delete every clause whose content is not stated by a record; keep grounded paraphrases verbatim):",
    )
}

/// Shared record + grounding-audit-block renderer for both edit passes.
fn audit_edit_prompt(
    question: &str,
    draft: &str,
    evidence: &[EvidenceItem],
    audit: &GroundingAudit,
    draft_header: &str,
) -> String {
    let mut s = format!("Question: {question}\n\nRecords:\n");
    for e in evidence {
        s.push_str(&format!(
            "[{}] ({}, status={}) {}\n",
            e.slug,
            e.date,
            e.status,
            truncate(&e.text, 6000),
        ));
    }
    s.push('\n');
    s.push_str(draft_header);
    s.push('\n');
    s.push_str(draft);
    if !audit.fabricated.is_empty() {
        s.push_str(
            "\n\nGROUNDING AUDIT — FABRICATED (string-checked: absent from EVERY record). \
             Remove each span and any clause that depends on it:\n",
        );
        for f in &audit.fabricated {
            s.push_str(&format!("  - \"{}\" ({})\n", f.span, f.kind));
        }
    }
    if !audit.misattributed.is_empty() {
        s.push_str(
            "\nGROUNDING AUDIT — MISATTRIBUTED (present in some record but NOT in the [SLUG] cited \
             beside it). Re-check each against its exact cited record; delete the value if absent there:\n",
        );
        for f in &audit.misattributed {
            s.push_str(&format!(
                "  - \"{}\" ({}) cited to [{}]\n",
                f.span,
                f.kind,
                f.cite.as_deref().unwrap_or("?")
            ));
        }
    }
    if !audit.off_cluster.is_empty() {
        s.push_str(
            "\nGROUNDING AUDIT — OFF-CLUSTER records (different event than the conclusion). \
             Drop their bullets unless their text states the conclusion:\n",
        );
        for c in &audit.off_cluster {
            s.push_str(&format!("  - [{c}]\n"));
        }
    }
    s.push_str("\n\nReturn ONLY the edited answer.");
    s
}

fn verify_prompt(question: &str, claims: &[Claim], evidence: &[EvidenceItem]) -> String {
    let mut s =
        format!("Question: {question}\n\nClaims to verify (with their cited record text):\n");
    for (i, c) in claims.iter().enumerate() {
        s.push_str(&format!("\nClaim {}: {}\nCited records:\n", i + 1, c.text));
        for n in &c.support {
            if let Some(e) = evidence.get(n.saturating_sub(1)) {
                s.push_str(&format!(
                    "  [{}] ({}, {}) {}\n",
                    e.index,
                    e.date,
                    e.source,
                    truncate(&e.text, 6000)
                ));
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
        s.push_str(&format!(
            "This question is about the organization's knowledge AS OF {d}.\n"
        ));
    }
    s.push_str(&format!(
        "Question: {question}\n\nVerified claims (use ONLY these):\n"
    ));
    for (i, c) in verified.iter().enumerate() {
        let cls = c
            .classification
            .as_deref()
            .map(|x| format!(" [{x}]"))
            .unwrap_or_default();
        s.push_str(&format!(
            "{}. {}{} (sources: {})\n",
            i + 1,
            c.text,
            cls,
            c.provenance.join(", ")
        ));
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
                "[{\"id\":1,\"supported\":true,\"quote\":\"owner is Maya Patel\",\"classification\":\"direct\"},{\"id\":2,\"supported\":false,\"quote\":\"\",\"reason\":\"figure not in record\"}]"
            } else {
                "Maya owns the Acme account (direct)."
            };
            Ok(Completion::text(out))
        }
    }

    async fn store_with_fact() -> Arc<GbrainStore> {
        let store = Arc::new(GbrainStore::open_in_memory(Embedder::new(None)).unwrap());
        let mut input = CaptureInput::new(
            MemoryScope::Project("p".into()),
            "Acme account owner is Maya Patel",
        );
        input.provenance = vec!["ART-1".into()];
        store.capture(input).await.unwrap();
        store
    }

    #[test]
    fn proper_names_and_contradiction_detection() {
        assert_eq!(
            proper_name_phrases(
                "Please provide Miguel Torres's statement and Diego Alvarez's view"
            ),
            vec!["Miguel Torres".to_string(), "Diego Alvarez".to_string()]
        );
        // clash phrasing + 2 named parties => contradiction question
        assert!(is_contradiction_question(
            "Outline the differing positions of Luis Ramirez and Marco Rossi and whether resolved"
        ));
        // audit/supersession phrasing must NOT trigger (protects the C4 win)
        assert!(!is_contradiction_question(
            "What is the data retention policy now and what changed since the 180-day version?"
        ));
        // single party, no clash => not a contradiction question
        assert!(!is_contradiction_question("Who decided the migration?"));
    }

    #[test]
    fn phantom_cite_strip_drops_invented_slugs_only() {
        let ev = vec![EvidenceItem {
            index: 1,
            slug: "ART-EV-2023-001".into(),
            date: "2023".into(),
            source: "".into(),
            status: "current".into(),
            note: "".into(),
            text: "x".into(),
        }];
        let out = strip_phantom_cites(
            "Maya decided it [ART-EV-2023-001] then [ART-EV-2099-999] later, see [note].",
            &ev,
        );
        assert!(out.contains("[ART-EV-2023-001]")); // real cite kept
        assert!(!out.contains("ART-EV-2099-999")); // invented slug dropped
        assert!(out.contains("[note]")); // non-slug bracket kept
    }

    #[tokio::test]
    async fn loop_prunes_unsupported_claims_and_keeps_grounded_ones() {
        let agent = DecisionAgent::new(store_with_fact().await, MockReasoner);
        let ans = agent
            .answer("Who owns the Acme account?", None)
            .await
            .unwrap();
        // The fabricated revenue claim is dropped; the grounded ownership claim survives.
        assert_eq!(ans.context.verified.len(), 1);
        assert_eq!(ans.context.dropped.len(), 1);
        assert!(ans.context.dropped[0].contains("70M"));
        assert_eq!(
            ans.context.verified[0].provenance,
            vec!["ART-1".to_string()]
        );
        assert!(ans.answer.contains("Maya"));
        assert_eq!(ans.provenance, vec!["ART-1".to_string()]);
        assert_eq!(ans.context.llm_calls, 4);
    }

    #[tokio::test]
    async fn strict_faithfulness_rejects_supported_claim_without_quote_span() {
        struct NoQuoteVerifier;
        #[async_trait::async_trait]
        impl Reasoner for NoQuoteVerifier {
            async fn complete(&self, system: &str, _user: &str) -> anyhow::Result<Completion> {
                let out: &str = if system == DECOMPOSE_SYS {
                    "[\"who owns acme\"]"
                } else if system == DRAFT_SYS {
                    "[{\"text\":\"Maya Patel owns Acme\",\"support\":[1]}]"
                } else if system == VERIFY_SYS {
                    "[{\"id\":1,\"supported\":true,\"quote\":\"\",\"classification\":\"direct\"}]"
                } else {
                    "Maya Patel owns Acme."
                };
                Ok(Completion::text(out))
            }
        }

        let agent = DecisionAgent::new(store_with_fact().await, NoQuoteVerifier)
            .with_strict_faithfulness(true);
        let ans = agent
            .answer("Who owns the Acme account?", None)
            .await
            .unwrap();
        assert!(ans.context.verified.is_empty());
        assert_eq!(ans.context.dropped.len(), 1);
        assert!(ans.context.dropped[0].contains("no quote"));
        assert!(
            ans.answer
                .to_lowercase()
                .contains("not contain enough evidence")
        );
    }

    #[tokio::test]
    async fn empty_evidence_yields_honest_abstention() {
        let store = Arc::new(GbrainStore::open_in_memory(Embedder::new(None)).unwrap());
        struct NoClaims;
        #[async_trait::async_trait]
        impl Reasoner for NoClaims {
            async fn complete(&self, _system: &str, _u: &str) -> anyhow::Result<Completion> {
                Ok(Completion::text("[]"))
            }
        }
        let agent = DecisionAgent::new(store, NoClaims);
        let ans = agent.answer("anything?", None).await.unwrap();
        assert!(ans.context.verified.is_empty());
        assert!(
            ans.answer
                .to_lowercase()
                .contains("not contain enough evidence")
        );
    }
}
