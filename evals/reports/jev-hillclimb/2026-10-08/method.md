# Jev hill-climbing protocol

The 24-task October 8 corpus is development data. Changes are selected on that
corpus and eight newly authored development tasks. Screening runs use one attempt
per task/profile; fresh confirmation uses three repetitions. Report these phases
separately, retain every failure, and never pool screening into confirmation.

A fixture subagent authored and keylessly validated eight development tasks and
six sealed holdout tasks. The parent optimizing prompts did not inspect holdout
contents or model outcomes before freezing the candidate. The holdout was expanded
from four shorter tasks to include two 11-action workflows before any model run.
The original four task objects were preserved. The development set includes a
12-action and a 10-action workflow. Script length describes the fixture contract,
not a mandated model action count.

Every new fixture checks exact final state, visible completion evidence, mutation
counts and forbidden side effects. Scripted contract tests also corrupt grader
inputs; separate real-browser negative traces execute wrong-row then correct-row
actions, duplicate writes, and partial completion. Those must fail the graders.
Keyless success establishes fixture validity, not model performance.

Expanded holdout freeze SHA-256 values:

- `tests/fixtures/evals/complex-browser-holdout.json`: `d70bc7a1a3acfc96636a54a5314ab87ed29cee170979b11e24be40bf328fd4ab`
- `tests/fixtures/pages/complex-browser-holdout.html`: `4b0d87d801eb3ca9cef54d73011f39c45d2b0fc0560fbbf4fe6309f484672cdf`
- `tests/fixtures/pages/complex-casework.html`: `67db070dc0df8b867ca5a3b581baa6499ab2f64bcaf01ade4229ecc48b87c2e7`
- `tests/fixtures/pages/complex-publication.html`: `b6a2347378a5fdd731e52e77eb478bd71db77d2ea625a4f97bda4adf1c1bc128`

All paths above are relative to `crates/roder-ext-jev`. Neither the fixture agent
nor the parent used model results to repair held-out fixtures. The parent reran
keyless contracts after adding form metadata to verify the observation change.

Each model attempt uses a fresh fixture site/page, identical goals, outcome
graders and fixture-supplied values, and a 60-second timeout. No generative text
helper or fallback model may rescue failures. Provider order rotates by task and
repetition; attempts run sequentially. This task ran no parallel Chrome/build jobs during live
measurement; unrelated system load was not isolated. Wall time includes browser work on a shared development machine and
is not isolated API latency.

The baseline explicitly selects the unchanged Jev request profile. New form and
disabled-control metadata is ignored by that profile and by the Decisions
adapter. The observation work is shared browser overhead. Jev consumes text only;
the visual Decisions reference also receives viewport images. This compares the
integrations, not identical model modalities. Report input tokens, not inferred
dollar costs across providers.

Jev wire requests/responses are now retained alongside Decisions wire data.
Authentication headers are never recorded. Screenshots in raw traces come only
from throwaway local fixtures. Joint-action experiments validate the actual joint
answer, then project it deterministically for the browser contract; their original
probabilities are in wire traces and projected one-hot values must not be used as
calibration evidence.

The first three screens isolate literal wording, action history, operation/target
alignment and form ownership. A fourth compares the strongest candidates and the
visual Decisions reference on the expanded development set. Any option-order
stress probe is reported separately, not silently substituted for normal attempts.

Limitations: task templates and repetitions are correlated; this is not a large
independent website sample. One legacy task says "Edit Grace Hopper's contact"
but its grader requires only opening the edit view, so its completion interpretation
is ambiguous. We preserve that task and its grades rather than changing it to help
a candidate. New tasks use explicit final-state requirements.

The frozen candidate is `jev_workflow`: original conditional operation/target
questions plus concise contextual history, native form ownership, disabled
controls, and staged-workflow guidance. It does not add a second model call,
completion predicate, confidence threshold, fallback, or image input. The exact
production-source hashes are recorded in `freeze.json`. Subsequent edits add
regression tests and documentation, not new model guidance.

The fixture author's independent review identified two coverage limits. The
allocation grader checks the final equipment set and mutation counters but permits
a direct replacement shortcut; its ten actions describe the scripted recovery
route, not an enforced minimum. Retained traces show which path the model took.
The short async transitions validate DOM updates, not sustained waiting. These
limits were recorded before reading holdout model outcomes; graders were not
changed after candidate freeze.

Wire call counts count calls to the transport wrapper; HTTP retries within the
poster are not separate records. Recorded response usage is provider-reported,
and no price or calibrated workflow-success probability is inferred from it.

Release review follow-up: the measured freeze is preserved at commit
`944537477d339d33f9575cabb858a07711a7a1f9`. Subsequent release hardening preserves
short-secret screenshot suppression across calls, tolerates optional capture
errors, scrubs secret echoes from new metadata, and fixes an experimental Grounded
guard and harness diagnostics. These paths have separate regression tests; the
886-attempt study was not rerun or relabeled as a measurement of those fixes.
