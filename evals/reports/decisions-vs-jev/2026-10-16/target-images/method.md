# Target-scoped image experiment

This is a second inference-integration iteration after rebasing PR #157 onto
`origin/master` at `b9e5b08a`. No Jev weights were trained or fine-tuned.

## Diagnosis and development probes

The preceding frozen comparison exposed a service-case failure. That task is
now development evidence, not a new holdout. Its current title, priority and
assignment were correct, yet Jev kept replacing the title instead of progressing.
The screenshot showed the title input horizontally scrolled to its end, hiding
the beginning. The DOM and target criteria contained its complete value.

We replayed exact recorded requests before changing execution behavior:

- `replay-probe.jsonl.gz`: 40 calls, five states × two repetitions × four
  variants: unchanged, explicit operation criteria, concise operation question,
  and both changes. None selected CLICK on the completed intake step.
- `replay-narrow.jsonl.gz`: 30 calls, five states × three repetitions × two
  variants: a field-needing-text Choice with NONE and a complete-action Choice.
  Both remained confused by the completed title when image evidence was shared.
- `replay-clipping.jsonl.gz`: 24 calls, two states × three repetitions × four
  variants: unchanged, no image, a clipping note, and that note plus explicit
  operation criteria. Only removing the image consistently selected CLICK on the
  completed intake step (3/3); each image-bearing variant selected TYPE_TEXT (3/3).
- `replay-scoped.jsonl.gz`: 15 calls, five states × three repetitions, moving the
  same image and rectangles into conditional target instructions. The operation
  matched the observed next requirement on all 15 states. Click target selection
  was still uncertain on the completed intake step. This did not prove successful
  execution; it justified the following browser screen.
- `service-screen.jsonl.gz`: two complete browser attempts, one each of the old
  DOM shared-image profile and the new target-image profile. The old profile
  reproduced the blocked outcome; the new profile passed the independent final
  state and single-write audit. This screen is not pooled into confirmation.

These 109 replays are billed model judgments over recorded states, not independent
browser tasks. The raw requests and responses, including rejected development
ideas, are retained. `replay.py` can submit those exact requests again. The model
alias is mutable; consult each returned model ID. Replays use one HTTP attempt
per call; browser runs retain the normal production retry policy.

## Interrupted confirmation and routing refinement

The first broad run was stopped after 42 complete attempts to address review
findings in DOM redaction and observations. Every completed row is retained in
`interrupted-confirmation.jsonl.gz`, with `interrupted-freeze.json` recording its
source and binary. These are not pooled into the final comparison. The shared
DOM control passed 14/14, Visual Decisions 13/14, and pure target-only images
13/14. The latter successfully submitted the date form but then reported BLOCKED
on its confirmation page. Removing image evidence from every operation therefore
introduced a completion regression, even though it fixed the service-case step.

The interim refined policy scoped images only when an offered editable field currently
contains a nonempty value. This was intended to cover cases where screenshot
clipping contradicts complete field contents; the next run exposed its overreach. All other pages retain shared
visual evidence, including confirmation pages. No fixture ID or expected outcome
is used in this routing rule. `hardened-service-screen.jsonl.gz` and
`hardened-date-screen.jsonl.gz` retain three attempts per provider per task: the
refined candidate passed 3/3 on each, the shared DOM control 0/3 on service and
3/3 on date, and Visual Decisions 3/3 on both.

The refined source also fixes JSON-escaped secret echoes, retains short labels
whole until scrubbing, preserves styled toggle state and controls nested in headings,
restores the existing viewport-scoped freshness behavior, and tests real generated
irreversible-action questions and hostile DOM marker text. The same hardened DOM
extractor is used for both final Jev profiles.

## Clipping-based refinement

The populated-field policy was also stopped, after 101 complete confirmation
attempts, when it produced false success on Enter-to-search. All 101 wire rows
and all 101 printed outcomes are retained in `populated-fields-interrupted`;
`populated-fields-freeze.json` identifies that source. This partial grid is not
pooled into any final score. It showed that merely having a value is too broad a
reason to remove planning images.

The final candidate routes by actual observed horizontal/vertical scrolling or
content overflow of an offered nonempty editable field. Chrome supplies the
scroll/client/content dimensions; the rule does not use the task ID, expected
answer, goal words, or historical benchmark results. A Chrome regression test
contrasts a populated field whose full contents fit with a horizontally scrolled
field. All other pages retain shared image evidence. Oversized DOM strings are
omitted in full instead of leaking secret prefixes or terminating the traversal;
following controls remain represented. A separate Chrome test covers long text
and an oversized role before a usable button.

Before the final freeze, `clipped-service-screen.jsonl.gz` retained six attempts
(candidate 3/3, shared DOM 0/3) and `clipped-six-screen.jsonl.gz` retained 36 attempts
(candidate 18/18, shared DOM 18/18). The six tasks cover dropdown selection, styled
checkbox state, native dates, icon appearance, unlabelled fields and Enter search.
`development.json` records source commits, policy, hashes and every development
browser attempt. Across the three prototypes, 205 development/interrupted browser
attempts and 109 recorded-state model calls are retained separately from the final
comparison. No completed paid attempt was discarded.

## Frozen candidate and controls

`dom_targets` uses the identical DOM questions and candidates as `dom`. It changes
image placement on pages with actually clipped populated editable fields: screenshots and candidate
rectangles appear in conditional target instructions, rather than shared state.
Operation selection and irreversible-action Nouls then receive DOM evidence
without images. Other pages retain the shared image. No extra decision
round trip, threshold change, forced next action or fixture-specific runtime rule
was introduced. The default production prompt remains Workflow, with images off by default.

The final candidate was frozen in `freeze.json` before its broad model evaluation.
Both final Jev variants pin `jev-1.14.0`; earlier development used the
`jev-preview` alias and retains each returned actual model ID.
The manifest records source and fixture hashes, the isolated Rust 1.95 test
binary hash, source commit, upstream base, suite definitions and provider order.
TypeSafe documents that versioned IDs are accepted and aliases may move
([models](https://docs.typesafe.ai/models)). Each browser attempt uses a fresh page;
provider order rotates within task and repetition. Every attempt is retained.
The paired controls are Visual Decisions and the preceding DOM shared-image
candidate, not an older run of either provider. Fill values come from fixtures;
generative value completion and fallback are disabled. Status and independent
page/side-effect graders must both pass. Stop/confirmation tasks are graded against
their required stops rather than counting every stop as failure.

The 24-task confirmation and eight compound development tasks are reused
regression/confirmation evidence. The six former compound holdouts and four
published search holdouts are now explicitly regression sets. The additional
transfer set was declared before model evaluation and contains eight existing
fixture tasks absent from all retained Decisions/Jev paid datasets in this repo:
covered_target, icon_by_picture_twin, focus_handoff, escape_popup, off_origin,
cookie_banner_refused, login_form and one_time_code_resolved. These are existing
fixtures with known keyless contracts, not secret benchmark tasks. Do not tune on
transfer results and then present them as held out.

## Source guidance and limits

TypeSafe's [image placement documentation](https://docs.typesafe.ai/images)
explains that shared-state images reach every question, while instruction images
are specific to their question. It also explains that repeated instruction image
references increase expanded input size and token usage. This experiment applies
that documented scope directly; repeated target references are a measured cost,
not a claim of free visual evidence.

The [Choice guide](https://docs.typesafe.ai/primitives/choice),
[building guide](https://docs.typesafe.ai/concepts/how-to-build-with-system-one)
and [speculative fan-out pattern](https://docs.typesafe.ai/patterns/fan-out)
guide narrow judgments and explicitly conditional target selection. Full DOM
values can be better evidence for input contents than a clipped picture, while
images can remain useful for visually identifying the right target. This causal
hypothesis is supported by a small recorded-state ablation; browser outcomes
measure the complete profile, not a general theorem about model vision.

Machine load is shared; these are paired local browser measurements, not isolated
production latency or dollar-cost measurements. Raw provider input-token counts
include different tokenizers. Image references can multiply expanded request size;
normal provider rejection handling remains in place. No model-service errors or
failed attempts may be removed from reported totals.
