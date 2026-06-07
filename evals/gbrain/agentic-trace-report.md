# bt-gbrain Agentic Trace Audit

Date: 2026-06-07

This audit covers local OrgMemBench result JSON under
`/Users/pz/w/OrgMemBench/results` and source inspection in
`crates/roder-ext-gbrain`. It does not use live provider calls.

## Summary

The earlier bt-gbrain eval traces show fixed-loop self-answering, not
provider-tool agentic retrieval. Concise runs have no decomposed subqueries and
two LLM calls per row. Strict/thorough runs have exactly three decomposed
subqueries and four LLM calls per row. The model is not choosing bt-gbrain
tools during answer generation in those modes.

Phase 86 adds `roder-gbrain answer --agentic-tools`, which now runs a
provider-native read-only tool loop and can request provider-native parallel
tool calls. The first Sonnet medium + ZeroEntropy small runs prove the model is
querying, re-querying, and finishing through `respond_to_query`, but they also
show the next bottleneck: final answers are not sufficiently tied to observed
evidence ids or quote spans.

The fixed loop records useful counts and claim lists, but it does not record a
provider turn history, model-selected tool calls, tool observations, retrieval
notes, memory snapshot identifiers, final confidence, open questions, or the
route used to finish the answer. Those missing fields are the audit gap Task 2
and Task 3 must close.

## Completed Local Eval Metrics

| Result path | Tier | Accuracy | Faithfulness | Queries | Failure modes | LLM calls | Subqueries |
|---|---:|---:|---:|---:|---|---:|---:|
| `results/bitemporal-gbrain/helix-small.json` | small | 0.7265 | 0.1818 | 11 | hallucination 7, none 2, wrong_synthesis 2 | 2 | 0 |
| `results/bitemporal-gbrain/helix-medium.json` | medium | 0.6608 | 0.3699 | 73 | hallucination 32, none 15, retrieval_miss 10, wrong_abstain 1, wrong_synthesis 15 | 2 | 0 |
| `results/bitemporal-gbrain-opus/helix-small.json` | small | 0.7833 | 0.1818 | 11 | hallucination 9, none 1, retrieval_miss 1 | n/a | n/a |
| `results/bitemporal-gbrain-opus/helix-medium.json` | medium | 0.5805 | 0.1507 | 73 | hallucination 53, none 9, retrieval_miss 2, wrong_synthesis 9 | n/a | n/a |
| `results/bitemporal-gbrain-gpt55-strict/helix-medium.json` | medium | 0.3594 | 0.5205 | 73 | hallucination 26, retrieval_miss 28, wrong_abstain 16, wrong_synthesis 3 | 4 | 3 |
| `results/bitemporal-gbrain-gpt55-strict-high/helix-medium.json` | medium | 0.4009 | 0.4932 | 73 | hallucination 24, none 3, retrieval_miss 31, wrong_abstain 9, wrong_synthesis 6 | 4 | 3 |
| `results/bitemporal-gbrain-gpt55-medium-google-emb2-small-p6/helix-small.json` | small | 0.5530 | 0.7273 | 11 | hallucination 3, none 1, retrieval_miss 6, wrong_abstain 1 | 4 | 3 |
| `results/bitemporal-gbrain-sonnet-medium-google-emb2-small-p8/helix-small.json` | small | 0.6030 | 0.0909 | 11 | hallucination 8, none 1, wrong_synthesis 2 | 4 | 3 |
| `results/bitemporal-gbrain-phase84-small-p6-high/helix-small.json` | small | 0.4636 | 0.2727 | 11 | hallucination 3, none 1, retrieval_miss 6, wrong_synthesis 1 | 4 | 3 |
| `results/bitemporal-gbrain-sonnet-medium-zeroentropy-small-p8-obsidian/helix-small.json` | small | 0.6000 | 0.1818 | 11 | hallucination 9, retrieval_miss 1, wrong_synthesis 1 | 4 | 3 |
| `results/bitemporal-gbrain-sonnet-medium-zeroentropy-predream-small-p8-obsidian/helix-small.json` | small | 0.6341 | 0.2727 | 11 | hallucination 8, none 1, retrieval_miss 1, wrong_abstain 1 | 4 | 3 |
| `results/bitemporal-gbrain-sonnet-medium-zeroentropy-agentic-tools/helix-small.json` | small | 0.4735 | 0.1818 | 11 | hallucination 4, retrieval_miss 5, wrong_synthesis 2 | agentic | n/a |
| `results/bitemporal-gbrain-sonnet-medium-zeroentropy-agentic-parallel-tools/helix-small.json` | small | 0.4095 | 0.2727 | 11 | hallucination 3, retrieval_miss 7, wrong_synthesis 1 | agentic | n/a |

The Opus result family predates the current fixed-loop trace shape: its raw row
objects record `audit_replay`, `n_results`, and `n_contradictions`, but not
`llm_calls` or `subqueries`.

## Trace-Shape Facts

- Concise Sonnet family: `results/bitemporal-gbrain/*.json` records
  `raw.subqueries = []` on every row and `raw.llm_calls = 2` on every row.
- Strict/thorough families:
  `results/bitemporal-gbrain-gpt55-strict/helix-medium.json`,
  `results/bitemporal-gbrain-gpt55-strict-high/helix-medium.json`,
  `results/bitemporal-gbrain-gpt55-medium-google-emb2-small-p6/helix-small.json`,
  `results/bitemporal-gbrain-sonnet-medium-google-emb2-small-p8/helix-small.json`,
  `results/bitemporal-gbrain-phase84-small-p6-high/helix-small.json`, and the
  ZeroEntropy small runs record exactly three subqueries and four LLM calls on
  every row.
- Current raw traces include answer-time fields such as `as_of`, `subqueries`,
  `evidence`, `drafted`, `verified`, `dropped`, `n_dropped`, `llm_calls`,
  `agent_input_tokens`, `agent_output_tokens`, and `agent_cost_usd`.
- Current raw traces do not include `trace.provider_turns`, `trace.tool_calls`,
  `trace.tool_observations`, `trace.retrieval_notes`, `trace.responded_via`,
  `trace.final_confidence`, `trace.open_questions`, memory snapshot ids, quote
  span coverage, citation precision, or stop reason.

## Provider-Tool Agentic Runs

`results/bitemporal-gbrain-sonnet-medium-zeroentropy-agentic-tools/helix-small.json`
was the first Sonnet medium + ZeroEntropy provider-tool small run. It finished
all 11 rows through `respond_to_query`, averaged `59.73` tool calls and `52.55`
provider turns per question, and reached `0.4735` accuracy with `0.1818`
faithfulness. Citation precision and quote-span coverage were `0.0`.

`results/bitemporal-gbrain-sonnet-medium-zeroentropy-agentic-parallel-tools/helix-small.json`
ran the same small tier with `RODER_GBRAIN_PARALLEL_TOOL_CALLS=1`, dream before
eval, and OrgMemBench `parallel=8`. Ingest imported 126 artifacts, then ran
dream `603f2806-4f0a-4220-a510-d3faa31f120f`, producing 313 derived statements
and 279 derived events before answering.

Parallel trace stats:

| Metric | Pre-parallel | Parallel tools |
|---|---:|---:|
| Accuracy | 0.4735 | 0.4095 |
| Faithfulness | 0.1818 | 0.2727 |
| Hallucination rate | 0.8182 | 0.7273 |
| Cost per query | $2.3812 | $1.6163 |
| p50 latency | 212.4s | 134.9s |
| p95 latency | 460.1s | 363.5s |
| Average tool calls | 59.73 | 54.45 |
| Average provider turns | 52.55 | 31.91 |
| Max provider turns | 160 | 126 |
| Citation precision | 0.0 | 0.0 |
| Quote-span coverage | 0.0 | 0.0 |

Every parallel row recorded `parallel_tool_calls=true`. Across 11 full traces,
tool-call provider turns had average batch size `1.71` and max batch size `3`.
The top tools were `gbrain_recall` (`404` calls), `gbrain_search_raw` (`50`),
`gbrain_find_start_nodes` (`48`), and `gbrain_as_of` (`31`).

Conclusion: parallel tools improve execution shape and cost, but the
faithfulness problem remains an evidence-control problem. The next phase should
reject unsupported `respond_to_query` calls, normalize observed evidence ids,
require quote spans for final claims, and improve ontology/temporal navigation
tools before another medium run.

## Source Findings

- `crates/roder-ext-gbrain/src/infer.rs:61` builds the
  `AgentInferenceRequest` used by `EngineReasoner::complete`; lines 74-75 set
  `tools: Vec::new()` and `tool_choice: ToolChoice::None`. Live providers
  therefore cannot choose bt-gbrain tools through this path.
- `crates/roder-ext-gbrain/src/agent/retriever.rs:1` describes a read-only
  provider-style retrieval loop foundation that intentionally does not call a
  live provider yet.
- `crates/roder-ext-gbrain/src/agent/retriever.rs:77` defines the local
  `ToolPlanner` trait, and lines 164-182 define `FakeToolPlanner`. The loop can
  execute model-shaped tool calls against a read-only registry, but the caller is
  still a fake/local planner rather than a provider-native tool bridge.
- `crates/roder-ext-gbrain/src/agent/retriever.rs:117` rejects non-read-only
  tool names before execution. This is a useful safety primitive for the future
  provider-tool runner.

## Hallucination Despite Multiple Verified Claims

These rows were judged as `hallucination` while the raw trace admitted at least
five verified claims. They show that local lexical/quote support is not enough
to prove the semantic relation, temporal status, conflict state, or causal link
required by the benchmark.

| Result path | Question | Score | Verified | Dropped | Evidence pool | Calls | Subqueries |
|---|---|---:|---:|---:|---:|---:|---:|
| `results/bitemporal-gbrain-gpt55-strict/helix-medium.json` | Q-0001 | 0.1875 | 8 | 3 | 31 | 4 | 3 |
| `results/bitemporal-gbrain-gpt55-strict/helix-medium.json` | Q-0014 | 0.2250 | 5 | 5 | 39 | 4 | 3 |
| `results/bitemporal-gbrain-gpt55-strict/helix-medium.json` | Q-0015 | 0.0625 | 7 | 2 | 37 | 4 | 3 |
| `results/bitemporal-gbrain-gpt55-strict/helix-medium.json` | Q-0020 | 0.5000 | 6 | 1 | 30 | 4 | 3 |
| `results/bitemporal-gbrain-gpt55-strict/helix-medium.json` | Q-0028 | 0.8750 | 9 | 0 | 29 | 4 | 3 |
| `results/bitemporal-gbrain-sonnet-medium-google-emb2-small-p8/helix-small.json` | Q-0004 | 0.7833 | 9 | 2 | 40 | 4 | 3 |
| `results/bitemporal-gbrain-sonnet-medium-google-emb2-small-p8/helix-small.json` | Q-0005 | 0.4667 | 10 | 2 | 37 | 4 | 3 |
| `results/bitemporal-gbrain-sonnet-medium-google-emb2-small-p8/helix-small.json` | Q-0010 | 0.3000 | 12 | 4 | 28 | 4 | 3 |

## Retrieval Miss Or Wrong Abstain Despite Large Evidence Pool

These rows were judged as `retrieval_miss` or `wrong_abstain` while the raw
trace had at least 20 evidence items available. They show that larger pools and
fixed subquery counts do not guarantee benchmark-relevant evidence selection.

| Result path | Question | Failure mode | Score | Verified | Dropped | Evidence pool | Calls | Subqueries |
|---|---|---|---:|---:|---:|---:|---:|---:|
| `results/bitemporal-gbrain/helix-medium.json` | Q-0002 | wrong_abstain | 0.0000 | 0 | 0 | 21 | 2 | 0 |
| `results/bitemporal-gbrain/helix-medium.json` | Q-0007 | retrieval_miss | 0.8625 | 0 | 0 | 21 | 2 | 0 |
| `results/bitemporal-gbrain/helix-medium.json` | Q-0029 | retrieval_miss | 0.8750 | 0 | 0 | 20 | 2 | 0 |
| `results/bitemporal-gbrain/helix-medium.json` | Q-0065 | retrieval_miss | 0.4333 | 0 | 0 | 21 | 2 | 0 |
| `results/bitemporal-gbrain-gpt55-medium-google-emb2-small-p6/helix-small.json` | Q-0001 | retrieval_miss | 0.6750 | 7 | 1 | 27 | 4 | 3 |
| `results/bitemporal-gbrain-gpt55-medium-google-emb2-small-p6/helix-small.json` | Q-0004 | retrieval_miss | 0.7333 | 12 | 4 | 40 | 4 | 3 |
| `results/bitemporal-gbrain-gpt55-medium-google-emb2-small-p6/helix-small.json` | Q-0010 | wrong_abstain | 0.0000 | 0 | 6 | 28 | 4 | 3 |
| `results/bitemporal-gbrain-gpt55-strict/helix-medium.json` | Q-0007 | retrieval_miss | 0.3375 | 4 | 4 | 39 | 4 | 3 |

## Fixture

The compact fixture for this audit is
`evals/gbrain/agentic-trace-fixtures/current-runs.json`. It records the current
run summaries, source findings, and representative failure rows above. It is
intended as an audit fixture for future trace-shape work, not as a replacement
for the full OrgMemBench result JSON.
