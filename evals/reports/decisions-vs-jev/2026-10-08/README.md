# Decisions optimization: 94.4% on the confirmation set

The improved visual Decisions backend passed **68/72 attempts (94.4%)**, up from
**51/72 (70.8%)** for the original adapter in the same run. Jev passed **62/72
(86.1%)**. On four new held-out tasks, visual Decisions, the original adapter,
and Jev each passed **12/12**; the improved text-only variant passed **10/12**.

This supports shipping the visual version as the opt-in OpenAI backend. Jev
remains Roder's default. The gain has a latency and token tradeoff, and this small
local fixture study does not establish superiority on arbitrary websites.

## What changed

- Native Decisions question wording with explicit observable completion criteria,
  current values, and containing card/row context.
- Recent action context and effects as evidence for the next decision.
- Fresh viewport images and candidate rectangles, with an explicit text-only option.
- Per-question refusal handling: ignore unused target refusals; stop on refused
  required operations, targets, or safety predicates and preserve billed usage.
- Per-request evidence isolation, provider-specific capability declarations, and
  cached-client refresh when the configured API key changes.

Screenshots are suppressed around recognized secret fields and previously typed
secrets. This is not universal image redaction. Images are not returned in normal
browser tool output. Native computer action selection remains an embedding API;
these optimization results concern the browser loop, not arbitrary computer tasks.

## Confirmation: 24 tasks, three repetitions, four variants

| Variant | Passed | Rate | Median wall time, all attempts | Input tokens, all attempts | DONE with failed outcome |
|---|---:|---:|---:|---:|---:|
| Original Decisions adapter | 51/72 | 70.8% | 2,308 ms | 274,951 | 8 |
| Native prompts + action effects, text only | 62/72 | 86.1% | 2,007.5 ms | 197,471 | 8 |
| Native prompts + effects + screenshots | **68/72** | **94.4%** | 2,312 ms | 524,102 | 4 |
| Jev (`jev-latest` → `jev-1.13.0`) | 62/72 | 86.1% | 1,387 ms | 386,396 | 3 |

The visual improvement is **+23.6 percentage points** over the original adapter
and **+8.3 points** over Jev in this run. Visual Decisions solved ten paired
attempts Jev missed; Jev solved four visual Decisions missed. These are repeated
task templates, not 72 independent website samples.

All-attempt medians mix different successful and failed paths. On the **58 pairs
both visual Decisions and Jev passed**, medians were **2,149 ms vs 1,448 ms**;
the median within-pair difference was **+614.5 ms** for visual Decisions. On the
49 successful pairs shared with the original adapter, the within-pair difference
was +172 ms. See [paired latency](paired-latency.json).

Visual Decisions used 1.91× the original adapter's input tokens and 2.65× the
text-only variant's input tokens. Tokenizers and billing differ across providers;
these counts are not dollar-cost comparisons. Counts include failed attempts.

The four visual failures were two repeated clicks below the fold, one repeated
cart addition (37 copies instead of one), and one framed-form completion claim
without the expected submitted result. Every failed attempt is retained. The
shared browser's inferred-completion heuristic remains a limitation: DONE status
can come from that heuristic or the model. The summary separates these sources.

## Held-out validation

Four new task templates were frozen after keyless scripted contract validation:
add exactly one Oak Bookend, leave an already-correct cart alone, submit a search
with Enter, and complete a partially populated preferences form while keeping an
already-checked setting enabled. No prompt tuning followed this model run.

| Variant | Passed | Median wall time | Input tokens |
|---|---:|---:|---:|
| Original Decisions | 12/12 | 1,358 ms | 26,622 |
| Improved text-only | 10/12 | 1,340 ms | 21,014 |
| Improved visual | **12/12** | 1,481 ms | 45,567 |
| Jev | 12/12 | 1,201.5 ms | 43,344 |

The text-only failures both added a second bookend. Visual Decisions tied the
controls on this small holdout; it did not demonstrate a holdout advantage.

## Experiments we measured and did not enable

First screening used one repetition of the same 24 development tasks:

| Variant | Passed |
|---|---:|
| Original adapter | 15/24 |
| Per-question refusals only | 17/24 |
| Native question wording | 20/24 |
| Joint operation + target choices | 18/24 |
| Sequential operation then target | 18/24 |
| Broad completion predicate at 0.90 | 11/24 |
| Native wording + action effects | **21/24** |
| Jev | 21/24 |

A second screen scored text effects 20/24, visual effects **23/24**, and a narrower
completion predicate 16/24. All variants in that second screen incurred image
capture overhead, but only the visual variant received images. Confirmation and
holdout use the production capability, capturing images only for visual Decisions.

Joint and sequential selection remain internal eval variants: neither improved
this screening enough to replace conditional target questions. Completion
predicates introduced too many false negatives. They remain disabled.

A retrospective confidence sweep also argues against a blanket threshold. For
visual Decisions, a 0.70 cutoff would reject all three incorrect model-DONE
answers in confirmation, but also **20 correct completions**. A 0.90 cutoff would
reject **37 correct completions**. This does not address inferred completion or
undo repeated actions. We recorded the distributions without treating a cutoff
selected on these fixtures as a calibrated production policy.

## Evidence and reproduction

Across both screens, confirmation, and holdout, this optimization recorded
**600 live attempts**. Screening selected the design; it is not pooled into the
confirmation success rate. The historical [October 6 report](../2026-10-06/README.md)
is a separate earlier baseline (47/72 Decisions, 63/72 Jev).

- [Protocol and limitations](method.md), including frozen holdout hashes.
- [Confirmation data](confirmation.jsonl.gz) and [summary](confirmation-summary.json).
- [Holdout data](holdout.jsonl.gz) and [summary](holdout-summary.json).
- [First screening](screening.jsonl.gz) and [summary](screening-summary.json).
- [Visual screening](visual-screening.jsonl.gz) and [summary](visual-screening-summary.json).
- [Paid live reproduction commands](reproduce.sh).

Raw rows contain decisions, effects, provider-reported usage and OpenAI wire
requests/responses; image traces contain only throwaway local fixtures. No API
keys or authorization headers are recorded. Jev wire payloads are not recorded.
Provider order rotates by task and repetition. Text values are fixture supplied,
and fallback/generative text assistance is disabled for every provider.

Regenerate the confirmation and holdout summaries from the repository root:

```sh
report=evals/reports/decisions-vs-jev/2026-10-08
python3 "$report/analyze.py" --tasks 24 --repeats 3 --providers original,effects,vision,jev "$report/confirmation.jsonl.gz"
python3 "$report/analyze.py" --tasks 4 --repeats 3 --providers original,effects,vision,jev "$report/holdout.jsonl.gz"
python3 "$report/paired.py" "$report/confirmation.jsonl.gz"
```

The analyzer rejects missing attempts, duplicate attempts, unexpected repetition
counts and provider sets. Wall times include browser work on a shared developer
machine; they are not isolated API latency measurements. Confirmation was run
before final error-path/test hardening; those changes do not change valid decision
selection. The holdout used the hardened implementation.

## Implementation validation

- Decisions-focused tests: **18 passed**, three live tests ignored by default. This includes real Chrome screenshot capture/secret suppression, native computer dispatch, holdout fixture contracts, refusal handling, concurrent evidence isolation, and summed usage on sequential errors.
- Agent-limit integration tests: **8 passed**. Doctest: **1 passed**.
- Full `roder-ext-jev` library suite: **476 passed, 1 failed, 12 ignored**. The failure was `fixture_harness::autoconsent_tests::autoconsent_and_consent_js_never_both_act_on_a_document`; its isolated retry passed. The full suite is not reported as clean.
- `cargo test --workspace` stopped at `roder-core --test remote_runner`, test `failed_runner_initialization_is_retryable_without_local_fallback`. One concurrent caller wrote `retry-b.txt` instead of receiving the shared initialization error. The isolated retry also failed. This unrelated remote-runner implementation was not modified. App-server library tests had passed 108/108 before that stop.
- Workspace Clippy completed with existing warnings. After fixing findings in this change, `cargo clippy -p roder-ext-jev --all-targets --no-deps -- -D warnings` passed.
- Changeset gate, scoped formatting/whitespace checks, and all four report-summary reproductions passed. The analyzer rejected deliberately missing and duplicate rows.
- All 600 live attempts completed collection. A successful eval process exit means collection completed, not that every task passed.

## Documentation basis

The [OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions)
recommends observable criteria, distinguishable choices, conditional independence
for batched questions, and threshold calibration with labeled examples. The
[API reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)
defines question-level refusals and image input. Those informed the variants;
which variant ships is based on the measured outcomes above. No installed
OpenAI Decisions-specific skill was found; the existing TypeSafe skill documents
the Jev side, not authoritative OpenAI prompting guidance.
