# Native browser recovery evaluation — October 28, 2026

The previous USPS trial completed through one parent delegation but both native
drivers stopped early and Terra completed the research. This follow-up adds:

- Stale-decision diagnostics naming changed fields without recording their
  values. A navigation may be retried once without another model call only
  after a fresh observation proves the exact node, label, destination, context,
  document, URL and form state unchanged. Other changes require a fresh decision.
- One local replan after a low-confidence repeated link. The duplicate is
  suppressed and excluded from the recovery choices. Buttons, commitment labels
  and high irreversible probabilities keep the stop. Budgets are not reset.
- Native `READ_DOCUMENT` operations using the same rendered-text extractor as
  direct tools. Complete text is scrubbed before 6,000-character/100-line pages
  enter bounded retained evidence. Document reads leave the viewport unchanged.
- An optional caller `requirements` checklist, bounded to 12 items of 500 bytes.
  Requirements must be backed by observations; the checklist does not establish
  completion by itself.
- An opt-in evaluation experiment giving Terra one tool call before returning
  control to the same native engine. Its history, budgets, scope, gates and
  secrets persist. It does not change the production fallback policy.

The native engine's execution logic is split out of `agent.rs` as part of these
changes. Historical trajectories and failed trials are retained.

## Evaluation protocol

The paired fixture runner alternates provider order per task. The requested
profiles are Decisions `vision_activation` and Jev `jev_activation-vision`, with
`jev-latest` and one repeat. The development suite includes the 53-task base
corpus plus compound tasks. These native-only runs use task-supplied field values
and independent DOM, POST and state graders. They isolate browser selection and
execution rather than parent response generation or the text helper.

The USPS reruns use the complete research task, Terra 6.1 low for parent and
fallback, and the previous whole-task wrapper. The runner saves execution events,
API transcripts, session records, final prose, timings and binary hashes. Its
results should be compared with the v4 whole-delegation trials, whose wrapper
already removed the second manual pass.

Raw artifacts remain local under
`/Users/pz/roder-browser-evals/native-recovery-2026-10-28/` and
`/Users/pz/roder-browser-evals/usps-2026-10-28/`.

## Iterations and interpretation

The first local-recovery candidate (v5) improved native capabilities but did not
improve the live research run consistently. Decisions took 92.715 seconds and
844,273 input tokens, exhausting native and fallback budgets and using six parent
tool calls. Jev took 72.106 seconds and 277,562 input tokens, with one parent call
and 17 fallback actions. Both produced the correct final answer, but these are
combined outcomes with parent or fallback recovery.

The diagnostic events showed full-page text/control changes and, in Jev's case,
a repeated Shop menu link. The follow-up (v6) recognizes navigation anchors by
observed HTTP(S) destinations even when ARIA labels them menuitems, retains full
target/form checks, and introduces explicit `task_kind=research`. It preloads
bounded document text and checks document identity for research completion.
Interaction mode remains the default and retains the full freshness requirement.
No success predicate or action gate is relaxed.

V6 Decisions returned correctly in 53.703 seconds with one parent call, 12 native
steps, 13 native requests and six fallback actions, consuming 526,296 input and
1,654 output tokens. Jev still required parent recovery. This led to the final
research workflow: give the native model one current checklist requirement at a
time, retain the overall constraints, and advance only after its explicit DONE
claim for that requirement. Such claims are recorded with source references and
remain distinct from independent verification. The native action and model-call
budgets continue across checkpoints.

The one-tool Terra rescue remains an evaluation experiment. It resumes the same
engine and retains its rules and budgets; it is not the production fallback
policy. Its results are compared with a full-fallback control below.

## Final USPS research run

The final candidate (v8) additionally distinguishes source-evidence completion
from form confirmations in both the operation prompt and the activation audit.
Ordered research requirements remain driver claims, not independently verified
checkpoints. Document snapshots are invalidated after dispatched content-changing
actions so a same-URL update cannot leave subsequent reads on the old snapshot.

| Run | End-to-end | Parent tools | Native steps / requests | Fallback tools | Input / output tokens | Outcome |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| Decisions + Terra | 85.375 s | 7 | 7 / 10 | 20 | 724,890 / 1,706 | Correct final reply after parent recovery |
| Jev + Terra | 64.816 s | 1 | 12 / 15 | 13 | 328,336 / 4,522 | Correct final reply within one delegation |
| Direct Terra, fresh CDP control | 48.178 s | 14 | — | — | 427,751 / 912 | Correct final reply using direct Chrome tools |

Both native drivers claimed the first two checklist requirements but did not
finish natively. Decisions then exhausted its unusable-reply retries; its fallback
exhausted its tool budget. The parent finished with six manual calls. Jev reached
the native action budget and Terra completed the remaining task in the same call.

Compared with v4, Jev elapsed time changed from 72.668 to 64.816 seconds while
input increased from 214,040 to 328,336. Decisions changed from 59.870 to 85.375
seconds and input from 299,121 to 724,890. The new capabilities therefore do not
establish a general speed or token-efficiency improvement on this task. All
versions and their intermediate failures are retained; the report does not select
the fastest intermediate configuration as the final result.

The initial direct-Terra controls in v7 and v8 lacked the browser endpoint setting
and could not open USPS. Their short failure times are **excluded** from successful
speed comparisons. A fresh control uses `RODER_DESKTOP_CDP_PORT` from the same
persistent Chrome profile, matching the earlier direct-Terra setup.
It completed correctly in 48.178 seconds, with 8.797 seconds inside browser tools
and 39.381 seconds outside them. It used direct `chrome_*` snapshots, scrolling
and clicks; the historical 19.891-second Terra trial used a different direct-tool
path including page evaluation. These are separate strategies, not matched
measurements of a runtime regression. The historical Codex control remains
39.447 seconds from instruction to final answer; it was not rerun in this pass.

Final CLI binary SHA-256:
`04849c699f505cc2f7bf08fe3516aa87822d868cee51afacd25d44ac2e2efb58`.
The snapshot at `v8-source.patch` plus `v8-new-source-files.json` records the
uncommitted implementation over `eea2d2e8`. Runtime source was checked against
that snapshot after measurement; documentation may be updated afterwards.
The final frozen test executable is recorded separately in `v8-provenance.json`.

These are single USPS trials with model-selected browsing strategies. They do
not isolate inference speed, caching, page dynamics or changes in chosen paths.
Native image-enabled arms still request internal screenshots; avoiding parent
screenshot tools does not mean the models received no images. Image encoding
was not changed by this work.

## Final native corpus results

All rows below use the frozen v8 test executable, interaction mode and each
driver's selected profile. Research-only metadata is absent and its new prompt
section is not added to these requests. Each task/provider combination ran once. A contract pass
requires both the expected verdict and independent truth checks. Some fixtures
expect an honest stop; these pass counts are **not** task-completion rates.

| Suite | Driver | Contract passes | Correct DONE | False success | Median / P95 | Native decision input / output |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Development | Decisions | 53 / 61 | 35 | 0 | 2.066 / 4.609 s | 999,668 / 0 |
| Development | Jev | 56 / 61 | 39 | 0 | 1.648 / 3.428 s | 1,666,620 / 28,854 |
| Complex holdout | Decisions | 5 / 6 | 5 | 1 | 3.475 / 8.894 s | 182,346 / 0 |
| Complex holdout | Jev | 5 / 6 | 5 | 0 | 2.170 / 5.825 s | 269,591 / 4,666 |
| Completion development | Decisions | 3 / 4 | 3 | 0 | 1.690 / 2.987 s | 34,703 / 0 |
| Completion development | Jev | 4 / 4 | 4 | 0 | 1.214 / 1.999 s | 64,976 / 1,331 |
| Completion holdout | Decisions | 4 / 4 | 4 | 0 | 2.394 / 2.679 s | 48,244 / 0 |
| Completion holdout | Jev | 4 / 4 | 4 | 0 | 1.840 / 2.165 s | 80,310 / 1,442 |
| Decisions holdout | Decisions | 4 / 4 | 4 | 0 | 1.737 / 2.706 s | 33,342 / 0 |
| Decisions holdout | Jev | 4 / 4 | 4 | 0 | 1.352 / 2.593 s | 57,448 / 1,030 |

Across 79 tasks per driver, this is 69 contract passes / 51 correct DONE results
for Decisions and 73 / 56 for Jev. Decisions' `complex_holdout_1` returned DONE
but `window.audit.forbidden` was 1 instead of 0. This remains a correctness and
safety failure and is a reason to keep the PR draft. Jev blocked on
`complex_holdout_5` before the required service-case write. Development failures
include cookie decisions, popover dismissal, context selection, a navigation
race and unusable Decisions responses. Two visual-selection fixtures also
completed where the native contract expected a stop; their contract mismatches
remain visible rather than being silently counted as successes.

These timings describe different tasks, not matched speedups. Native token
counts sum the provider's reported decision usage, including repeated context.
Task-value helpers supplied fixture values without model calls; some aggregate
row token fields are null, so the table uses `telemetry.usage.decision` directly.
Decisions returns probabilities rather than generated output tokens. HTTP error
requests may lack usage, and hosted `jev-latest` can change between trials.

## Bounded rescue versus full fallback

The four cases were `hover_menu_fallback`, `canvas_fallback`, `drag_fallback`
and `popover_icon_fallback`, run once per driver with Terra 6.1 low. Independent
full-goal grading is applied even when native execution finishes without needing
a fallback; an expected native stop alone does not establish combined completion.

| Driver / strategy | Full goals | Resumed / successful after resume | Fallback input | Fallback output | Combined median |
| --- | ---: | ---: | ---: | ---: | ---: |
| Decisions, full fallback | 4 / 4 | — | 36,680 | 511 | 7.809 s |
| Jev, full fallback | 4 / 4 | — | 21,470 | 342 | 6.224 s |
| Decisions, one call then native | 2 / 4 | 4 / 2 | 9,680 | 183 | 5.981 s |
| Jev, one call then native | 3 / 4 | 3 / 2 | 6,911 | 120 | 4.677 s |

Full fallback completed all eight goals. One-call rescue completed five, including
one native completion before rescue; only four of seven resumed runs finished.
It used fewer fallback input tokens but lost accuracy. This does not justify
replacing full fallback. Both strategies had zero full-goal false successes in
this small sample. Raw rows retain native outcomes, rescue actions, resumed
outcomes, usage and independent grades.

## Validation and reproduction

The frozen source passed 764 library tests and 100 integration tests for
`roder-ext-jev`. The Chrome extension passed 123 unit tests and its real-browser
`computer_use` test. The latter's startup waits for a valid DevTools port instead
of assuming a newly created file already contains one. Formatting, changeset
validation and generated release configuration checks passed before committing.

```sh
make fmt-check
mise exec -- cargo test -p roder-ext-jev --lib
mise exec -- cargo test -p roder-ext-jev --tests
mise exec -- cargo test -p roder-ext-chrome

# Repeat each suite: development, complex_holdout, completion_dev,
# completion_holdout. For the Decisions holdout use DECISIONS_EVAL_HOLDOUT=1
# instead of BROWSER_EVAL_SUITE.
BROWSER_EVAL_SUITE=development \
DECISIONS_EVAL_VARIANTS=vision_activation,jev_activation-vision \
JEV_MODEL=jev-latest JEV_EVAL_REPEATS=1 \
mise exec -- cargo test -p roder-ext-jev --lib \
  fixture_harness::evals::decisions_comparison::decisions_vs_jev \
  -- --exact --ignored --nocapture

JEV_EVAL_FALLBACK=model \
JEV_FALLBACK_MODEL=codex/gpt-6.1-terra JEV_FALLBACK_REASONING=low \
JEV_EVAL_TASKS=hover_menu_fallback,canvas_fallback,drag_fallback,popover_icon_fallback \
mise exec -- cargo test -p roder-ext-jev --lib \
  fixture_harness::evals::rescue::bounded_rescue_handback \
  -- --exact --ignored --nocapture
```

Live commands require the configured provider credentials. For full-fallback
control use the paired runner with the same four tasks and
`JEV_EVAL_FALLBACK=model`. Exact run scripts, configuration metadata, source
snapshots, raw JSONL, analysis and binary hashes remain in the local artifact
directories above. `v8-phases.json` records every phase and result path. Failed
and intermediate trials are retained. The comparison preserves existing notes
and original captures while showing the latest three Roder trajectories, the v4
comparison and historical controls; full raw records are kept outside the inline
payload. No new Codex live trial or new Skyscanner trial is claimed.
