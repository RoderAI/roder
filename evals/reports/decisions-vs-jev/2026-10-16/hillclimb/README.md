# Jev image decisions with semantic DOM evidence

The subsequent [clipped-field image-routing iteration](../target-images/README.md)
retains this campaign and evaluates a refined profile after rebasing onto master.

The final candidate passed **72/72** on the 24-task
confirmation set, compared with **65/72** for the PR's
Baseline Jev image prompt and **67/72** for Visual Decisions.
This changes the browser evidence and typed questions sent to hosted Jev; no
model training was performed. Both Jev variants use `jev-preview`; actual model
IDs are retained in [models.json](models.json).

## Final frozen comparison

| Variant | 24-task confirmation | 8 compound dev tasks | 6 compound held-out tasks | 4 search regression tasks |
|---|---:|---:|---:|---:|
| Visual Decisions | 67/72 | 14/16 | 18/18 | 12/12 |
| PR baseline Jev + screenshots | 65/72 | 16/16 | 12/18 | 9/12 |
| DOM-grounded Jev + screenshots | 72/72 | 16/16 | 15/18 | 12/12 |

Every cell includes all attempts. A pass requires both the expected terminal
status and independent final-page/side-effect checks. Missing-value and
confirmation-gate tasks must stop as specified. These totals do not mean every
task should execute to DONE. The original four-task holdout's published failure
was already known and is a regression set here; the six compound held-out tasks
were untouched and not used for tuning in this campaign.

## Confirmation latency, tokens and false success

| Confirmation variant | Median wall ms | Median wire ms | Browser decisions | Input tokens | False success |
|---|---:|---:|---:|---:|---:|
| Visual Decisions | 2,441.5 | 344.5 | 260 | 554,704 | 4 |
| PR baseline Jev + screenshots | 1,613.5 | 114 | 229 | 1,025,822 | 3 |
| DOM-grounded Jev + screenshots | 1,584.0 | 129 | 213 | 1,292,745 | 0 |

The candidate used **1.26×** the Jev baseline's
input tokens across the confirmation attempts. This includes changes in number
of decisions; it is not a fixed per-call multiplier or a dollar-cost comparison.
The machine is shared, wall time includes browser work, and wire time includes
network latency. Provider tokenizers differ. Paired comparisons are within a
suite, not across separate runs.

- `jev_dom-vision@jev-preview vs jev-vision@jev-preview`: 65 both passed; 7 subject-only passes; 0 control-only passes; median paired wall delta 29 ms on both-passed attempts.
- `jev_dom-vision@jev-preview vs vision`: 67 both passed; 5 subject-only passes; 0 control-only passes; median paired wall delta -627 ms on both-passed attempts.
- `jev-vision@jev-preview vs vision`: 63 both passed; 2 subject-only passes; 4 control-only passes; median paired wall delta -615 ms on both-passed attempts.

There were 0 recorded transport errors across the four final suites.
The complete summaries include image-asset counts, unknown usage, returned model
IDs, terminal-confidence distributions and all per-task results. Confidence
thresholds were not changed to produce these gains.

## Residual failures

- confirmation, Visual Decisions: `twin_by_context` 1/3, `icon_by_picture` 2/3, `enter_to_search` 1/3.
- confirmation, PR baseline Jev + screenshots: `date_field` 2/3, `icon_by_picture` 0/3, `enter_to_search` 0/3.
- complex-dev, Visual Decisions: `complex_allocation` 0/2.
- complex-holdout, PR baseline Jev + screenshots: `complex_holdout_5` 0/3, `complex_holdout_6` 0/3.
- complex-holdout, DOM-grounded Jev + screenshots: `complex_holdout_5` 0/3.
- search-regression, PR baseline Jev + screenshots: `holdout_search` 0/3.

The service-case trace (`complex_holdout_5`, first repetition) shows the candidate
correctly filling the issue title, priority and assignment, then repeatedly
choosing TYPE_TEXT for the already correct title instead of Continue. It stopped
blocked with zero committed writes. The relevant controls and current values were
present; no service error or missing fixture fill value explains that failure.
The frozen prompt was not changed after seeing it.

All variants use fixture-supplied fill values and disable generative text completion
and fallback. This measures decision-backend behavior, rather than the full
production browser-agent cascade.

No failed attempt was removed or replaced. Consult each retained wire trace for
its exact state, offered targets, question meanings, probabilities and actions.

## What changed

Roder already exposed flattened page text and action candidates with values,
context and form owners. Three inspected Codex browser chats additionally
showed a semantic tree with landmarks, headings, nesting and control states.
The Jev path now provides a bounded composed-DOM approximation in its observation
and final `jev_browse` tool text, and the experimental prompt consumes it.
It includes open shadow roots and same-origin frames, omits hidden/clipped text,
keeps selected dropdown state, and excludes secret field values. Typed secrets
are scrubbed. Node references map to offered candidates; descriptions of disabled
controls do not make them executable.

The prompt bundle also puts actual available targets and form ownership inside
each operation's criteria, and distinguishes prepared input from submitted
outcomes and pending updates. Conditional target questions remain batched, and
irreversible-action Nouls, candidate IDs and execution/freshness guards remain
in code. A separate image-path repair scales candidate rectangles into resized
screenshot pixels. These fixtures use unchanged 1120×780 screenshots; the
coordinate fix is covered by deterministic tests.

This is a bundled improvement. The final comparison does not isolate the DOM
tree's contribution from question wording or available-target criteria, and it
does not compare the production Workflow prompt on the entire corpus. Workflow
with images was screened on the six known problem tasks. The default remains
Workflow; the measured candidate is opt-in:

```sh
JEV_PROMPT_PROFILE=dom JEV_IMAGES=1 JEV_MODEL=jev-preview roder
```

See [DOM behavior and tool-output example](../../../../../docs/jev-semantic-dom.md)
and the [protocol](method.md) for exact scope, source guidance and Codex chat IDs.

## Development attempts, all retained

| Screening candidate | Passed | Main observation |
|---|---:|---|
| Baseline + images | 4/6 | Missed deletion completion and search submission |
| Workflow + images | 4/6 | Fixed deletion, regressed the date task, missed search |
| Criteria + images | 4/6 | Same total as Workflow |
| First DOM prompt + images | 5/6 | Search remained unresolved |
| First DOM prompt, text only | 6/6 | Tiny ablation; not a full-corpus result |
| Structured DONE/WAIT + DOM + images | 10/12 | Completion criteria alone did not fix search |
| Available targets + DOM + images | 6/6 | Search succeeded; advanced to repeated comparison |

A preliminary comparison before final DOM edge-case hardening passed 72/72 for
the candidate, 66/72 for Baseline Jev images, and 69/72 for Visual Decisions.
It is retained as `development-confirmation`, separately from the final pinned
build. It was not substituted for the final result.

Across screening, preliminary comparison and final tests, **618 paid
browser attempts** are retained in this directory. Screenshots in these traces
are throwaway local fixtures. API keys and authorization headers are not recorded.
Small local fixtures and repetitions are not independent real-world samples;
these results do not establish arbitrary-site reliability or justify changing
the default provider.

## Evidence and reproduction

- [Freeze manifest](freeze.json), [earlier freeze](development-freeze.json).
- [Confirmation data](confirmation.jsonl.gz), [summary](confirmation-summary.json), [paired](confirmation-paired.json).
- [Compound dev data](complex-dev.jsonl.gz), [summary](complex-dev-summary.json), [paired](complex-dev-paired.json).
- [Compound holdout data](complex-holdout.jsonl.gz), [summary](complex-holdout-summary.json), [paired](complex-holdout-paired.json).
- [Search regression data](search-regression.jsonl.gz), [summary](search-regression-summary.json), [paired](search-regression-paired.json).
- [Preliminary data](development-confirmation.jsonl.gz), [summary](development-confirmation-summary.json).
- Screening data: [profiles](screen-profiles.jsonl.gz), [DOM v2](screen-dom-v2.jsonl.gz), [DOM v3](screen-dom-v3.jsonl.gz), [DOM v4](screen-dom-v4.jsonl.gz).
- [Live reproduction](reproduce.sh); [regenerate all final summaries and paired artifacts](summarize.py).

The summarizer rejects missing or duplicate attempts, unexpected task counts,
repetition grids or provider sets. The campaign checked frozen source/fixture hashes
after every suite. API model aliases can move; compare returned model IDs.

Validation on pinned Rust 1.95: **714 library tests passed, 13 ignored**;
integration and doc tests passed. Final focused DOM, model-pin and scaled-rectangle
tests passed, as did clippy with warnings denied, `make fmt-check`, the changeset
gate and release-config validation. An initial Rust 1.99 parallel run hit a timing
assertion in `mid_step_navigation`; the isolated rerun and full four-thread pinned
run passed. No fixture or assertion was weakened.
