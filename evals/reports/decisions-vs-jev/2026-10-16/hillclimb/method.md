# Jev image hill climb: protocol

Continue PR #157's October 16 screenshot evaluation. This campaign optimizes browser evidence and question construction; it does
not train the hosted model.

## Development and freeze

Screen Baseline, Workflow and Criteria with images on the six known problem tasks
(`delayed_spa`, `date_field`, `icon_by_picture`, `scroll_region`, `enter_to_search`,
`frame_form`). Then test successive DOM candidates. Retain all screening attempts
and wire requests, including candidates that regressed. The original four-task
holdout's published search failure is development knowledge for this campaign;
its rerun is a regression set, not independent evidence of generalization.

The final candidate keeps Workflow's form ownership, disabled controls and workflow
evidence. It adds a bounded semantic DOM tree and node-to-candidate mapping,
conditional available targets inside each operation's criteria, and structured
DONE/WAIT definitions that distinguish prepared fields, submissions and outcomes.
Independent conditional target choices and irreversible-action Nouls remain in one
request. No threshold is selected using the test sets. No task IDs, grader probes,
expected answers, fixture scripts or hidden DOM variables enter model state.

Before transfer testing, freeze the model questions and the fixture files in
`freeze.json`. A preliminary 24-task comparison was run while deterministic DOM
edge-case tests were being completed; retain it as `development-confirmation`.
The final confirmation and transfer runs use the corrected, pinned-Rust build.
No prompts are tuned after examining transfer results.

## Final paired runs

- Confirmation: the original 24 development tasks, three repetitions, 216 attempts.
- Compound development transfer: 8 tasks, two repetitions, 48 attempts.
- Compound held-out transfer: 6 existing frozen tasks, three repetitions, 54 attempts.
- Search regression: the original 4 held-out tasks, three repetitions, 36 attempts.

Every suite compares `vision` (Visual Decisions), `jev-vision@jev-preview`
(Baseline Jev with images), and `jev_dom-vision@jev-preview` (candidate with images).
Record actual answering model IDs, not merely mutable aliases. These are the
same models, screenshot dimensions and capture policy per variant as PR #157. Never remove or replace
an attempt after inspecting its result. Record failures and transport errors.
Each attempt has a fresh local site and page; fixtures and graders are unchanged.
Grade both terminal status and independent final-page/side-effect checks.
All variants use the same fixture-supplied fill values; generative text completion
and fallback are disabled. These measure the decision backend, rather than the
full production browser-agent cascade.

Run one suite at a time, with sequential attempts and rotating provider order
within task and repetition. Use one isolated copy of the test executable so a
shared Cargo target cannot silently substitute another worktree's binary.
Capture and fitting failures retain the established text-only behavior and remain
visible through the number of image assets in the wire evidence.

All paid attempts run on a shared development machine, sometimes alongside local
compilation or deterministic tests. End-to-end times include browser overhead and
network latency; no dedicated-host or isolated model-latency claim is made. Compare
paired attempts within each suite; don't interpret separate screening/final runs
as a controlled latency comparison. Input tokens are usage, not dollar costs.

The six compound held-out fixtures were not edited or used to choose a candidate
in this campaign. They are pre-existing repository tests, not newly sampled real
websites. Repeated attempts on a small set of tasks are not independent examples;
perfect fixture performance would not establish arbitrary-site reliability.

## Source guidance and browser-tool comparison

Read TypeSafe's live [images](https://docs.typesafe.ai/images),
[Choice](https://docs.typesafe.ai/primitives/choice), and
[building guide](https://docs.typesafe.ai/concepts/how-to-build-with-system-one).
Use structured criteria for confused neighboring choices, include actual relevant
state, and batch independent conditional questions. Attach each screenshot once
in state. Keep actions, freshness and authorization in code.

Inspect `tab.playwright.domSnapshot()` tool outputs from three local Codex chats:
`01a13ccf-0385-7173-bd98-c685ea70244f`,
`01a13c9d-641c-70f3-8ea7-db141d03f7bf`, and
`01a13deb-66b3-7872-9d1f-b77b92deae64` (October 14).
They expose indented semantic roles and names, heading levels, control states,
landmarks, dialogs, lists and text. No private page content from those chats is
copied into this report or the evals.

Roder's Jev path already exposed flat action candidates, field state, form owners,
context and page text; its tool digest grouped options but did not preserve that
semantic hierarchy. Add the bounded tree to observations, the experimental model
state, and the final `jev_browse` text inside the existing untrusted-content markers.
Keep Browser Use's existing indexed state/action observations and Chrome's ref-based
snapshot APIs intact; this implementation targets the Jev path being evaluated.
This is a composed-DOM approximation, not a complete browser accessibility tree.

## Reproduction

Run `reproduce.sh` with an isolated compiled test executable and configured API
credentials. Gzip each printed attempt path under its suite name. Run
`python3 .../hillclimb/summarize.py` to validate all expected task/repetition/provider
grids and regenerate every summary and paired comparison. Raw wire evidence
includes synthetic fixture screenshots; credentials and authorization headers are
never recorded.
