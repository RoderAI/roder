# bt-gbrain Faithfulness Report

Date: 2026-06-06

## Benchmark State

PR #30 comments establish the current validated sequence:

| Run | Overall | Faithfulness | Hallucinations | Notes |
|---|---:|---:|---:|---|
| Sonnet neutral medium baseline | 0.605 | 0.041 | 68/73 | strong C3/C6, severe over-elaboration |
| Opus 4.8 answerer A/B | 0.581 | 0.151 | 53/73 | more disciplined, lower rubric coverage |
| concise-v3 | 0.639 | 0.315 | 40/73 | strip + C6 retrieval + strict C4 |
| concise-v4 grounding audit | 0.675 | 0.397 | 31/73 | best validated branch state |
| supersession-support heuristic | 0.612 | 0.301 | 42/73 | reverted; over-pruned real supersessions |

Claude's broadened-retrieval Sonnet medium pass finished at n=73:
`overall = 0.6608`, `faithfulness = 0.3699`, `hallucination = 0.6301`.
Category scores were C1 `0.6542`, C2 `0.7058`, C3 `0.7405`, C4 `0.7127`,
C5 `0.4800`, C6 `0.7944`. This confirms the retrieval lever improved coverage
enough to clear the `>=0.65` overall target, but generation remains near the
0.40 faithfulness plateau without stricter claim admission.

Local final result files currently present:

| Result path | Overall | Faithfulness | Hallucination | Questions |
|---|---:|---:|---:|---:|
| `results/bitemporal-gbrain/helix-small.json` | 0.727 | 0.182 | 0.818 | 11 |
| `results/bitemporal-gbrain/helix-medium.json` | 0.661 | 0.370 | 0.630 | 73 |
| `results/bitemporal-gbrain-opus/helix-small.json` | 0.783 | 0.182 | 0.818 | 11 |
| `results/bitemporal-gbrain-opus/helix-medium.json` | 0.581 | 0.151 | 0.849 | 73 |
| `results/bitemporal-gbrain-gpt55-strict/helix-medium.json` | 0.359 | 0.521 | 0.479 | 73 |

See `evals/gbrain/agentic-trace-report.md` for the 2026-06-07 trace audit that
adds the newer Google Embeddings, ZeroEntropy, strict-high, and phase84 small
runs. That audit records the fixed-loop trace shape: concise rows use `0`
subqueries and `2` LLM calls, while strict/thorough rows use exactly `3`
subqueries and `4` LLM calls.

The gpt-5.5 medium strict run is not equivalent to the Claude/Sonnet broadened
medium run. It improves faithfulness over Sonnet's `0.3699` plateau, but it
collapses overall accuracy from `0.6608` to `0.3594`. This is a deletion-heavy
faithfulness gain rather than a better answerer.

## Diagnosis

The bottleneck has moved from retrieval coverage to claim admission. The branch
already fixed large recall misses through event-cluster expansion and broader
retrieval. The residual hallucinations are mostly:

- copied values attached to the wrong record or date
- as-of answers inventing a change sequence where records only show later state
- evidence-chain answers pulling artifacts from a neighboring event
- direct/inferred classification errors for meeting notes and summaries
- conflict answers that state a resolution without both sides and a resolving record

Representative failing question IDs from the current broadened-retrieval medium
partial:

- `Q-0003`: correct SLA direction but wrong change date
- `Q-0008`: correct supersession core but unsupported names
- `Q-0015`: wrong announcer, wrong date, fabricated figures
- `Q-0020`: wrong event cluster and wrong decision rationale
- `Q-0038`: residual misattributed-date supersession class noted in PR comments
- `Q-0066` / `Q-0067`: C5 evidence-chain failures rescued by v4 off-cluster fixes

Prompt-only or broad heuristic fixes are now risky. The reverted
supersession-support check is the proof: it targeted a real residual class but
deleted legitimate supersession content, lowering both rubric and faithfulness.

## Implemented Next Step

Phase 83 now starts with a deterministic claim ledger under
`crates/roder-ext-gbrain/src/agent/claims.rs`.

The strict ledger validates:

- every supporting artifact id and record number exists in the evidence pool
- every admitted claim has at least one quote span
- quote spans are normalized substrings of the cited record text
- obvious specifics in the claim are present in cited evidence
- direct claims cannot combine multiple records
- since-as-of claims require explicit change, replacement, or unchanged support

`roder-gbrain answer --faithfulness strict` routes through the thorough
draft/verify/finalize path and enables the deterministic ledger gate.

## Strict gpt-5.5 Medium Result

The full strict run completed in tmux session `gpt55-bt-gbrain-strict`.

- result path: `/Users/pz/w/OrgMemBench/results/bitemporal-gbrain-gpt55-strict/helix-medium.json`
- partial path: `/Users/pz/w/OrgMemBench/results/bitemporal-gbrain-gpt55-strict/helix-medium.partial.jsonl`
- config: `answer_model=gpt-5.5`, `faithfulness=strict`, `GBRAIN_REASONING_EFFORT=medium`
- measured answer/judge cost: `$15.0166`
- p50 latency: `111.57s`
- p95 latency: `222.88s`

Final metrics:

| Run | Overall | Faithfulness | Hallucination | Questions |
|---|---:|---:|---:|---:|
| Claude/Sonnet broadened medium | 0.6608 | 0.3699 | 0.6301 | 73 |
| gpt-5.5 medium strict ledger | 0.3594 | 0.5205 | 0.4795 | 73 |

Strict mode crossed the `>=0.50` faithfulness target, but failed the `>=0.65`
overall target by a wide margin. Failure modes were:

| Failure mode | Count |
|---|---:|
| `hallucination` | 26 |
| `retrieval_miss` | 28 |
| `wrong_synthesis` | 3 |
| `wrong_abstain` | 16 |

Category scores:

| Category | Score |
|---|---:|
| supersession | 0.3333 |
| decision_provenance | 0.5132 |
| bitemporal | 0.4738 |
| audit_replay | 0.3511 |
| justification_chain | 0.2447 |
| contradiction | 0.2139 |

The strict ledger exposed two separate bottlenecks:

- Over-pruning: many rows became faithful but low-scoring abstentions with all
  drafted claims dropped, especially temporal reconstruction, support-chain, and
  contradiction questions.
- Under-validation: rows with many verified claims still failed as
  hallucinations, which means the current validator proves local lexical support
  but not the semantic relation, temporal status, conflict state, or causal link
  required by the question.

Claim-drop diagnostics from the partial JSONL:

- 16/73 rows had `0` verified claims.
- 69/73 rows had at least one dropped claim.
- The run admitted 306 verified claims and dropped 520 claims.
- Average per row was `4.19` verified claims and `7.12` dropped claims.

## Next Implementation Direction

Do not spend another full medium run on prompt tuning or broader retrieval. The
next slice should make strict mode extractive before it is generative:

1. Build the answer from quote-anchored evidence cards first, not from free-form
   draft claims that are dropped afterward.
2. Add semantic relation validators for replacement, conflict resolution,
   direct-vs-inferred support, actor/action/date binding, and causal language.
3. Add a fixture runner for the existing `evals/gbrain/faithfulness-fixtures`
   cases and include abstention, quote coverage, and unsupported-claim metrics.
4. Emit the accepted and rejected claim trace in CLI JSON so failed eval rows can
   be audited without re-running live providers.
5. Re-run a small or targeted subset before another full medium run; the gate is
   faithfulness above `0.50` while moving overall back toward the Sonnet
   `0.6608` coverage level.
