# Native visual Decisions: semantic DOM and completion audits

This campaign ports the [Jev DOM and activation work](../activation-audit/README.md)
to OpenAI's native Decisions API. The existing visual profile remains the default.
The two new profiles are opt-in until their measured tradeoffs justify a change.

The native DOM profile passed **157/166 (94.6%)**, versus **152/166 (91.6%)** for the original visual profile. False successes fell **7 → 1**. The combined audited profile passed **150/166 (90.4%)**, with two false successes; this run does not support enabling it broadly. Both new profiles remain opt-in.

| Profile | Passed | False successes | Component requests | Input tokens | Median task wall time |
|---|---:|---:|---:|---:|---:|
| Original visual Decisions | 152/166 | 7 | 612 | 1,188,988 | 2,319.0 ms |
| DOM Decisions | 157/166 | 1 | 591 | 2,065,983 | 2,246.5 ms |
| Audited Decisions | 150/166 | 2 | 696 | 2,201,423 | 2,386.5 ms |
| Audited Jev 1.14 reference | 165/166 | 0 | 701 | 3,819,896 | 1,463.0 ms |

DOM used 3.4% fewer component requests but **73.8% more input tokens** than the original. Among 147 jointly passing attempts, its median paired wall-time increase was 58 ms; its all-attempt median was 72.5 ms lower, partly reflecting avoided failure loops. There were ten DOM-only passes and five original-only passes. These measurements do not establish production latency or calibrated reliability.

The gains include duplicate cart additions, iframe form submission, login and one-time-code handling. Both new native profiles passed all six login/code attempts versus two of six for the original. DOM still had eight BLOCKED stops whose outcome probes otherwise matched, on deletion receipts and cookie dismissal, and one premature completion after allocating only one of two requested items. See `totals.json` for every failure and `paired.json` for both directions of each comparison.

All four profiles passed both 12-attempt completion suites and the 18-attempt compound regression suite. These checks include already-completed and explicitly forbidden-submission contracts; they do not prove general completion detection.

The audited native profile made **91 audits and proposed no corrective clicks**. Its 594 browser decision callbacks expanded into 696 component requests: 91 audits plus 11 additional image-target batches. Relative to DOM, requests rose 17.8% and input tokens 6.6%; the median paired wall-time increase on jointly passing attempts was 142 ms across 149 pairs. The comparison combines audits, image routing and run variation; it does not isolate a causal explanation for every differing failure.

The Jev reference made 111 audits, with three proposed Go corrections on `escape_popup`; each corresponding browser attempt passed. Its only failure was a BLOCKED stop on the generic login receipt. Native Decisions and Jev therefore have different remaining failure modes. No recorded transport errors occurred, and every row had reported input usage. Two original-native rows ended in an error after repeated selected-target refusals; those failures remain included. Every recorded component request reconciles with `model_calls` and aggregate usage call counts.

To try the measured native DOM profile with configured credentials and Chrome:

```sh
JEV_DECISION_PROVIDER=openai JEV_DECISIONS_PROFILE=dom roder
```

## What changed

`JEV_DECISIONS_PROFILE=dom` supplies the bounded semantic DOM, form ownership,
offered operations and complete non-secret field values through native Choice
questions. Secret values remain withheld; observed `filled` state replaces empty
redaction placeholders in both shared state and target descriptions. This prevents
one observed populated secret from simultaneously being described as empty.
Presence alone does not establish that a secret is valid. BLOCKED's criterion also
requires an unmet requirement, so a successful page without controls is not
inherently blocked.

`JEV_DECISIONS_PROFILE=activation` adds the same clipped-field policy and
completion audit evaluated for Jev. The [Decisions API](https://developers.openai.com/api/reference/resources/decisions/methods/create)
puts images in shared input. On a page with an observed populated clipped field,
operation and safety questions therefore run in a text batch while conditional
target questions run in a separate image batch. These independent requests share
the same observation. Singleton targets resolve locally; no empty image batch
is sent. Other pages keep the shared viewport image.

When the original operation is DONE and a valid conditional click target exists,
a later text-only request asks three native predicates: required, performed,
ready. Their probabilities compose as `max(0, required + ready - performed - 1)`.
Support above 0.5 recovers the original offered click, with the original target
Choice distribution and existing action gate. This is a conservative rule on
estimated probabilities, not calibrated correctness, raw Choice confidence or a
universal completion test. An inferred DONE from the existing bounded loop is
outside this audit. Missing prerequisites can produce an unusable answer instead
of completion.

This follows the [Decisions guide](https://developers.openai.com/api/docs/guides/decisions):
batch independent questions, separate dependent requests, and consume predicate
probability separately from Choice confidence. Split batches and audits reserve
the remaining request budget before transmission. Usage includes all components;
a failed batch cannot hide the other batch's usage or dispatch a partial action.
Text-only mode retains the selected profile. Jev's runtime settings remain
independent. See [configuration and embedding API](../../../../../docs/openai-decisions-browser.md).

`dom-example.json` is an exact DOM/form excerpt from the frozen native contact
form request. `image-scope-example.json` identifies a text/image batch pair with
identical observed state apart from image rectangles. `extract_examples.py`
regenerates both from retained wire data without API calls.

## Frozen method

`freeze.json` identifies source commit `50fdd040910dcfe466b31b9a96947436717643cc`,
377 source/configuration/fixture hashes, the isolated Rust 1.95.0 library test
executable and protocol digest. The source and binary were frozen before this
campaign's paid calls. `protocol.json` declares every task ID, repetition and
provider: seven suites, 166 attempts per provider, 664 total on 58 distinct tasks.

All tasks were used in earlier campaigns. Nine tasks received native-profile
browser calls during development here; the other 49 received no browser calls
with these new native profiles before the freeze. These are known regression
and transfer fixtures, not a newly unseen corpus. The old `holdout` names are
fixture identifiers and do not establish fresh holdout status in this campaign.
The completion fixtures include exact mutation counts, duplicates, already
completed goals and explicitly forbidden submission. Scripted negative tests
check actual resulting mutations, rather than passing on any unrelated failure.

Variants run the same browser harness, oracle checks, fixture value resolver,
action candidates, gates and confidence thresholds. Expected results and oracle
probes are outside model requests. Generated fill text and fallback are disabled.
Providers rotate within task/repetition groups. All scheduled attempts and raw
errors are retained; there is no selection of successful reruns. OpenAI requests
use `gpt-6-luna`; Jev is pinned to `jev-1.14.0`. Response IDs are retained, but
these API model IDs do not provide immutable provider weight snapshots.

Task wall times include local browser activity on a shared machine. Reported
paired latency uses only attempts both variants pass, and reports that count.
Input tokens are provider-reported; tokenizers differ. Request totals count wire
requests. Browser decision callbacks are recorded separately because one callback
can contain multiple requests. These are not dollar costs or isolated API timings.

The runner writes raw data under this report's `live-source` directory and archives
each complete suite immediately. Retained `.jsonl.gz` files contain request and
response bodies, screenshots of throwaway local fixtures, usage and browser
outcomes. Authorization headers and API credentials are not recorded. Compressed
stdout logs are byte-preserved. `artifacts.json` covers retained evidence and
analysis helpers; the executable is identified by digest, not committed.

## Development evidence and its limits

The two development pilots scheduled 54 and 12 browser attempts. Their stdout logs
are retained as `pilot-v1.log.gz` and `pilot-v2.log.gz`. Their complete raw wire
files disappeared from the scratch cache before archiving; `development-gaps.json`
records both missing sources. They are not reconstructed or counted in the frozen
campaign. Pilot outcome logs support observed outcomes, not a complete request
or token audit.

`probe-secret.jsonl.gz` retains six paid requests on three recorded populated
secret states. Baseline wording selected TYPE_TEXT in all three; presence-aware
evidence selected CLICK in all three. These repeated known states are diagnostic,
not independent full-workflow validation. Unit tests also cover an empty secret.

`probe-blocked.jsonl.gz` retains 39 paid requests: three wording variants on 13
recorded states. Baseline had one missed DONE, exclusive wording zero, and
receipt wording one. None selected DONE in the states labelled not complete.
Those labels classify DONE versus non-DONE, not correctness of the next action.
No genuinely blocked state was included, so this does not establish blocked-case
calibration. Both probe sets retain every request and response separately from
the frozen browser results.

After this frozen campaign completed, a small integration follow-up exposed the
same reservation boundary to custom decision clients, retained known call counts
when `usage.calls` is omitted, and strengthened audit request-count contracts.
It does not retune the candidate prompt or fixtures. [Review notes](review-notes.md)
explain that separate change and the native image contract. The source freeze
continues to identify the actual measured code.

## Reproduction and validation

From this directory, analyze retained evidence without API calls:

```sh
python3 check_helpers.py
python3 check_previous_helpers.py
python3 -O verify_source.py
python3 -O summarize.py
python3 -O extract_examples.py
python3 -O inspect.py confirmation.jsonl.gz complex-dev.jsonl.gz compound-regression.jsonl.gz search-regression.jsonl.gz transfer.jsonl.gz completion-regression.jsonl.gz completion-transfer.jsonl.gz
```

The source verifier checks Git objects at the frozen commit, independent of later
report edits. Summaries reject any missing, duplicate or substituted task ID,
provider or repetition. Inspection checks image placement, component call counts
and every completion audit. A proposed correction is not proof of execution;
consult the browser action trace and independent oracle.

For a paid rerun, provide Chrome, configured OpenAI/TypeSafe credentials and an
isolated copy of the test executable:

```sh
JEV_EVAL_BINARY=/absolute/path/to/copied-test-binary python3 reproduce.py --output /absolute/path/to/new-report-output
```

The runner requires the recorded executable and protocol digests and refuses
existing outputs. An independently rebuilt executable with a different digest
requires its own verified source/binary freeze; do not label it as this run.
It sets `RUST_MIN_STACK=16777216` for the standalone Rust executable and checks
current source bytes before paid calls. Keep the original build checkout at the
frozen source: the executable loads fixture pages through its compiled manifest
path. These checks do not make an existing executable portable to another host.

The frozen-source package suite passed: 742 library tests, 88 integration tests
and one doc test. After the integration follow-up, the complete suite passed
again with 742 library tests, 90 integration tests and one doc test. Fourteen paid/live library tests are ignored by the ordinary suite.
New contracts cover native DOM/form evidence, scoped batch routing, native
predicate mapping, secret presence, usage aggregation, missing/refused answers,
request budgets, and applying the irreversible-action gate after an audit
correction. Package Clippy (`--all-targets --no-deps -D warnings`), `make fmt`,
`make fmt-check` and the changeset gate passed. Compressed validation logs retain
the successful run. The public enum and builder have a minor package changeset.
