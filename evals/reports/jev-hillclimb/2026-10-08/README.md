# Jev browser hill-climb and complex fixture evaluation

The frozen Jev candidate passed **94/96 confirmation attempts (97.9%)**, versus
**81/96 (84.4%)** for original Jev and **90/96 (93.8%)** for visual Decisions.
That is a **13.5 percentage-point improvement** over original Jev in this run.

| Backend | Confirmation: 32 tasks × 3 | Median wall time, all attempts | Input tokens, all attempts | Failed outcomes reported done |
|---|---:|---:|---:|---:|
| Original Jev | 81/96 | 1,465.5 ms | 638,239 | 6 |
| **Improved Jev** | **94/96** | 1,510.5 ms | 859,230 | 2 |
| Visual Decisions | 90/96 | 1,892.5 ms | 728,498 | 4 |

The candidate passed 13 pairs original Jev missed, with no reverse losses. On their
81 jointly successful attempts, median wall times were 1,247 vs 1,238 ms; the
median paired saving was 6 ms, effectively unchanged in this setup. Against visual
Decisions it gained four pairs with no reverse losses. On their 90 shared successes,
medians were 1,496 vs 1,845 ms, with a 372 ms median paired saving for Jev. These
comparisons condition on shared successes; all-attempt medians mix failure paths.
The candidate used 34.6% more input tokens than original Jev, including its longer
successful paths. See [paired measurements](confirmation-paired.json).

Both candidate failures were `complex_allocation`: it recovered from the stock
conflict and chose the replacement, but reported DONE on the final review without
reserving. Its successful repetition executed the full recovery route. Baseline
Jev and visual Decisions failed all three attempts on that task, ending via the
shared inferred-completion heuristic before reaching recovery. The candidate did
not use inferred completion in confirmation. No forbidden mutations were observed
in any provider's 24 audited complex attempts; that is limited fixture evidence,
not a universal safety guarantee.

A retrospective 0.50 model-DONE confidence cutoff would reject both candidate
false completions, but also 12 correct completions. A 0.90 cutoff would reject 43
correct completions. Thresholds do not fix action errors or inferred completion;
no new threshold is enabled.

## Sealed holdout: six tasks, three repetitions

| Backend | Passed | Median wall time | Input tokens |
|---|---:|---:|---:|
| Original Jev | 15/18 | 1,788.5 ms | 242,304 |
| **Improved Jev** | **18/18** | 1,723.5 ms | 321,882 |
| Visual Decisions | **18/18** | 2,239 ms | 241,676 |

Original Jev failed all three repetitions of the longer bulletin-publication
workflow, claiming completion without a saved result. The candidate and visual
Decisions passed every held-out task, including the two 11-action scripted
workflows. The candidate therefore transfers beyond the development cases in this
small sample, but does not demonstrate a holdout accuracy advantage over visual
Decisions. No prompt tuning followed holdout inspection.

All 18 holdout attempts per provider had side-effect audits; none recorded a
forbidden mutation. On 15 jointly successful candidate/baseline pairs, median
paired latency saving was 4 ms, again effectively unchanged. Against visual
Decisions, all 18 pairs passed and the median paired saving was 547 ms. See
[holdout measurements](holdout-paired.json) and [raw attempts](holdout.jsonl.gz).

## Choice-order stress probe

Reversing the insertion order of every Jev choice criterion, without changing any
candidate IDs or meanings, produced **31/32** for the improved profile and
**28/32** for baseline. The candidate again failed only the equipment final-review
case. Baseline's booking outcome changed under reversed ordering. This single
stress pass supports checking order sensitivity, not claiming order invariance;
it is not pooled with normal-order confirmation. Safety predicate questions were
unchanged. See [stress data](choice-order.jsonl.gz) and [summary](choice-order-summary.json).

## What improved

The production Jev profile now supplies native form ownership, disabled controls,
and ten recent actions with row/section context. It keeps the original conditional
operation and target questions, with guidance for moving through staged forms,
handling validation or availability changes, and distinguishing review from saved
completion. It adds no model round trip, image input, confidence cutoff or fallback.
OpenAI Decisions retains its separate visual profile.

The fixture subagent added eight development and six sealed holdout tasks. These
cover dependent controls, similarly named records, partial forms, exact-once writes,
review/final confirmation, nested scrolling, async updates, and longer multi-page
workflows. The development scripts include 12-action booking and 10-action
allocation routes; the holdout contains two 11-action routes. Every new task grades
visible completion, exact final state, write counts and forbidden mutations.
Keyless negative browser traces prove that wrong-row actions, duplicate writes,
and stopping at review are rejected.

## Development screening

Each screen used one repetition. These are tuning results, never pooled into the
fresh confirmation or holdout score.

| Screen | Tasks | Variant scores |
|---|---:|---|
| 1 | 24 | Baseline 21; literal wording 21; verbose action effects 21; operation criteria 19 |
| 2 | 24 | Baseline 21; effects 21; available targets in operation criteria 21; joint action choice 20 |
| 3 | 24 | Baseline 21; original wording + effects 21; available targets + original wording 21; form evidence 23 |
| 4 | 32 | Baseline 27; form + effects 30; form + concise context 30; visual Decisions 31 |
| 5 | 32 | Form + concise context 30; **staged-workflow guidance 32** |

Generic literal rewrites, larger criteria, and joint operation/target choices did
not improve enough to ship. Richer evidence was more useful. Form ownership fixed
choosing an unrelated Go button instead of Enter; disabled-control evidence helped
inner-panel scrolling; contextual history helped avoid a second deletion. Verbose
history hurt completion on the ambiguous legacy edit task. Concise context retained
the useful history without those effects. The final workflow guidance fixed the
development booking and changed-availability failures.

## Why this approach

TypeSafe documents structured text state, independent conditional questions and
literal interpretation limits. Those favor supplying observable relationships and
explicit workflow semantics rather than copying the screenshot-based Decisions
strategy. The current Jev model takes text, not images. Its confidence reflects
concentration among alternatives, not a calibrated probability that an entire
browser task succeeded. We retain confidence distributions for analysis and do not
turn an in-sample cutoff into a production safety claim.

Sources: [state](https://docs.typesafe.ai/concepts/state),
[conditional fan-out](https://docs.typesafe.ai/patterns/fan-out),
[Jev jaggedness](https://docs.typesafe.ai/model-jaggedness/jev-1.13),
[confidence](https://docs.typesafe.ai/confidence), and
[API contract](https://docs.typesafe.ai/api). The repository's
[TypeSafe skill](../../../../.agents/skills/typesafe-ai/SKILL.md) guided this work.

## Evidence and reproduction

The [method](method.md) records the separation between tuning and holdout, grader
limitations and exact frozen hashes. `jev` always means the original Jev profile
in this harness; production Jev uses `jev_workflow`. `vision` is the improved visual
Decisions profile from the earlier [Decisions study](../../decisions-vs-jev/2026-10-08/README.md).

Both providers receive identical fixture goals, supplied text values and graders.
Fallback and generative text assistance are disabled. Attempts use fresh pages,
rotating provider order and a 60-second timeout. No parallel build or Chrome test
was run by this task during measurement. Wall time includes browser work on a
shared developer machine. Input tokens are provider-reported usage, not a dollar
cost comparison. Repeated local templates are not independent real-world websites.

Across five screens, confirmation, holdout and choice-order probing, this study
retained **886 live attempts**. Jev requested `jev-latest` and returned
`jev-1.13.0`; Decisions used `gpt-6-luna`. Future alias resolution can change.

All screening rows and summaries are retained as `screening-{1..5}.jsonl.gz` and
`screening-{1..5}-summary.json`. Wire traces include model requests and responses,
not authorization headers. Images contain only throwaway local fixtures. Failed
attempts remain in every denominator. Transport records do not split HTTP retries.

Run paid evaluations from the repository root with both provider keys configured:

```sh
report=evals/reports/jev-hillclimb/2026-10-08
bash "$report/reproduce.sh" confirmation
bash "$report/reproduce.sh" holdout
bash "$report/reproduce.sh" order
python3 "$report/analyze.py" --tasks 32 --repeats 3 --providers jev,jev_workflow,vision "$report/confirmation.jsonl.gz"
python3 "$report/analyze.py" --tasks 6 --repeats 3 --providers jev,jev_workflow,vision "$report/holdout.jsonl.gz"
python3 "$report/paired.py" "$report/confirmation.jsonl.gz"
```

A successful harness process means complete collection, not universal model success.
The holdout is now public; future tuning needs a newly sealed set.

## Validation

- `cargo test -p roder-ext-jev --lib -- --test-threads=1`: **486 passed, zero failed, 12 ignored**. Includes real Chrome fixture contracts, negative side-effect traces, native form reassociation, unchanged safety questions, secret handling and Decisions profile isolation.
- `cargo test -p roder-ext-jev --test agent_limits`: **8 passed**. Doctest: **1 passed**.
- `cargo clippy -p roder-ext-jev --all-targets --no-deps -- -D warnings`: passed.
- `cargo test --workspace`: Jev passed again (**486/486**, 12 ignored), then the workspace stopped at `roder-ext-roder-cloud::engine::tests::list_models_without_key_falls_back_to_catalog` (`src/engine/tests.rs:426`): expected four models, got three. The unrelated model catalog has concurrent local changes and was not modified or staged by this work. The isolated catalog-test retry also failed with the same three-versus-four assertion. The previously failing remote-runner initialization test passed in this run.
- Scoped Rust formatting, whitespace and changeset checks passed. All eight retained dataset summaries reproduce; missing and duplicate attempts are rejected. Frozen candidate/holdout hashes still match, and raw traces contain local fixture data without authorization headers.
- All **886 live attempts** completed collection. Failed model outcomes remain in the reports; process success does not mean every task passed.
