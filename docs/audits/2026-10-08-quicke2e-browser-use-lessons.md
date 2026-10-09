# What Roder's browser-use features can learn from QuickE2E

Status: read-only study, 2026-10-08. Nothing was built or run. Compared QuickE2E at `98f74b6` (`/Users/pz/w/quicke2e`) with Roder at `5e727cee` (branch `pz/browser-use-agent-improvements-779594`).

How to read it: ranked items #1-#18 each name the files to change, the test that would prove it, and a confidence. Numbers labelled "author simulation" or "recorded in a comment" are not repo measurements. The code claims behind #1 (result-size cap, `browser_use_agent` getting a follow-up `get_state`), #2 (native report passing with wrong submits), #3 (unmerged extension branch, unrouted Desktop `chrome_select`), #4, #5 and #7 were re-checked by hand against the repos. The claim that the follow-up `get_state` after `browser_use_agent` shows a different browser comes from reading the pinned upstream package and was not re-verified.

## TL;DR

Do these first, in this order. Items 1 and 2 can run in parallel.

1. **Make `browser_use` results report-first and one line per element, and make dead clicks fail loudly.** Payoff: a 30-element page (208 lines in an unreproduced local check by the author) no longer spills past core's 200-line cap, the action report is never cut, and stale-index and select clicks become errors that name a working route.
2. **Build the minimal measurement slice.** That is a verdict/truth split on Jev eval rows, n>=3 with a saved baseline, and a strict native-computer grader. Payoff: every later gate becomes checkable, and the saved native run that "passed" with 3 wrong submits stops passing.
3. **Close the paired-extension and Desktop gaps.** Ship the unmerged `pz/computer-use-primitives` branch, render compact outcome-first results, and route `chrome_select` on Desktop. Payoff: silent first-match clicks and "not connected" selects go away.
4. **Retry an unusable Jev decision reply (cap 2), then hand off with a named cause.** Payoff: transient unusable-reply endings stop discarding whole runs.
5. **Add the twin-row repeat guard in Jev.** Payoff: stops the recorded wrong-row delete in `icon_by_picture`, which failed in every recorded live run.
6. **Add Jev run-wide loop and stale-reread caps, all routed to the frontier fallback.** Payoff: A-B-A-B and ticker-page loops end in about 6 actions with a named cause, not at the 60-action or 120 s limits (estimate, fixture-gated).
7. **Stop telling non-Responses providers a screenshot is "attached", and give the native computer path text facts.** Payoff: removes two sources of confidence in evidence the model never saw.

Everything model-facing in Jev (what the hosted chooser sees) is gated on an A/B. Several items are measure-first (see the probe list in #19).

Citation prefixes: `Q:` = `/Users/pz/w/quicke2e`, `R:` = the Roder worktree, `E:` = `/Users/pz/w/roder-web-extention` (the paired extension; branch `pz/computer-use-primitives`, tip `40ffd05`).

## What QuickE2E is, and the central lesson

QuickE2E is a small deterministic exploratory tester. Each step, code turns the page into a short menu of legal moves such as `CLICK 7 Pay [button]` (`Q:README.md:156`). A small decision model, either hosted Jev or a local logit reader, returns one offered key plus a confidence. It never writes text: typed values come from the spec's `inputs` (`Q:src/loop.mjs:1-5`). Code, not the model, decides success by checking assertions on every snapshot (`Q:src/loop.mjs:954-959`, `Q:src/loop.mjs:990`; the step-0 snapshot of an ordinary flow is skipped). Many bad options are removed from the menu rather than discouraged in a prompt. `Q:src/loop.mjs` is about 1,400 lines of field-hardened comments, so most of its numbers are code comments, not data files.

**The central lesson is that failures must be visible and code-owned, not inferred by a model.** Its anti-stuck machinery is deterministic: a run-wide (page, action) pair counter (`Q:src/loop.mjs:1105-1119`), a named reason when a click is covered (`Q:src/loop.mjs:687-704`), a named stop on a 5xx page (`Q:src/loop.mjs:733-747`), and history shown only while it agrees with the page (`Q:src/loop.mjs:836-842`).

**What carries over to Roder:**
- Naming causes in results, and visible omission counts instead of silent drops.
- Counting repeats in code.
- Letting code judge success when the caller states an end state.
- Fixture pages that fail before a change and pass after, graded by an app-side truth log.

**What does not carry over:**
- Menu shaping (heats, role ranking, hold-back, spec-key binding, the local engine, speculation). Roder's frontier surfaces have no menu, and Jev already has its own equivalents.
- QuickE2E's own reverts argue against copying it. Link promotion fixed starvation but took create-campaign from 5/5 to 0/5 (`Q:src/loop.mjs:85-89`). Masking typed values as "(filled)" took book-with-code from 5/5 to 0/5 (`Q:src/loop.mjs:277-278`).
- Its DONE-removal bakeoff (tree "no DONE option, gated BLOCKED") was neutral: 120/120 on both trees, identical steps, T1/T2 cost down 3-6% per cell (T3 unchanged, total about -2.2%). No run chose DONE or BLOCKED on either tree, so the matrix cannot show a benefit (`Q:docs/bakeoff-2026-09-24.md:506-512`). The "27/30 false DONEs" figure exists only as a comment (`Q:src/loop.mjs:861`).

**The weakest Roder links are places where the main frontier agent is told something untrue or told nothing.** Examples:
- A click on a select reports "Clicked element N" while changing nothing (inferred from upstream source; no recorded Roder run failed on a select).
- A screenshot is called "attached" on providers that never receive it.
- An extension click returns only `{ref}`.
- `browser_use_agent` is followed by an observation of a different, usually blank browser (read from the pinned server, see #1).

## What Roder already does (credit, don't rebuild)

| Capability | Where | Surfaces covered |
|---|---|---|
| Live-node freshness guard; stale target raises `StaleObservation` and the loop re-observes and re-decides | `R:crates/roder-ext-jev/src/agent.rs:137`, `R:crates/roder-ext-jev/src/assets/snapshot.js:159` | Jev only |
| 5-point hit-test, uncover ladder (Escape, close button, outside press) in the same step, 3-covered stall, DONE-after-covered downgrade | `R:crates/roder-ext-jev/src/assets/act.js:85-102`, `R:crates/roder-ext-jev/src/page/uncover.rs:35-66`, `R:crates/roder-ext-jev/src/page/act.rs:189-197`, `R:crates/roder-ext-jev/src/agent.rs:323-332`, `R:crates/roder-ext-jev/src/agent.rs:464-481` | Jev; chrome direct refuses a covered ref and names it (`R:crates/roder-ext-chrome/src/direct/target.rs:71-81`) |
| Three-in-a-row no-op stall rule | `R:crates/roder-ext-jev/src/agent.rs:464-481` | Jev only (shallow grep of chrome and browser-use found no analogue) |
| Per-step effect text ("nothing visible changed", "opened a new tab") | `R:crates/roder-ext-jev/src/effects.rs` | Shown in the Jev tool result, the digest and the fallback prompt (`R:crates/roder-ext-jev/src/fallback/prompt.rs:86`); not sent to the chooser. An `effect` eval variant (`Variant::Effect`, `evals/effect_variant.rs`) injects it into `recent_actions`, but its A/B never ran (HTTP 402, `R:docs/jev-browser.md:616-623`) |
| Post-DONE `success_condition` check with fallback on `outcome_mismatch` | `R:crates/roder-ext-jev/src/session/completion.rs:35-37`, `R:crates/roder-ext-jev/src/session/call.rs:163-167`, `R:crates/roder-ext-jev/src/fallback/trigger.rs:26-27` | Jev; opt-in, no corpus task sets one |
| Twin disambiguation by card/row/column context | `R:crates/roder-ext-jev/src/assets/context.js:68-150` | Jev; absent in chrome look |
| Native select flattened to one action per option, regrouped under one element; current option deliberately offered | `R:crates/roder-ext-jev/src/assets/snapshot.js:292-298`, `R:crates/roder-ext-jev/src/space.rs:135-147`, `R:crates/roder-ext-jev/src/space.rs:171-173` | Jev; chrome has `jev_tab_select` but `chrome_select` is unrouted on Desktop |
| Label proxy for styled checkbox/radio, generic clickables, open shadow roots and same-origin frames | `R:crates/roder-ext-jev/src/assets/snapshot.js:117-133`, `R:crates/roder-ext-jev/src/assets/snapshot.js:202-224` | Jev only |
| DOM-quiet settle; chrome direct has a similar clock | `R:crates/roder-ext-jev/src/assets/settle.js:15-72`, `R:crates/roder-ext-chrome/src/direct/session.rs:246-261` | Jev, chrome direct; the extension master build has none |
| Decision HTTP retry classes; one run deadline across phases, cut to the host's remaining time | `R:crates/roder-ext-jev/src/http/mod.rs:184-227`, `R:crates/roder-ext-jev/src/runner/drive.rs:5-8`, `R:crates/roder-ext-jev/src/runner/request.rs:167-175` | Jev only |
| Typed values come from a separate text helper; secrets scrubbed; `needs_input` names the field | `R:crates/roder-ext-jev/src/agent/typing.rs:14-77`, `R:crates/roder-ext-jev/src/secret.rs` | Jev only |
| Fallback loop already elides old page reads (keeps newest 2 reads, 1 image) | `R:crates/roder-ext-jev/src/fallback/run.rs:37`, `R:crates/roder-ext-jev/src/fallback/run.rs:448-481` | Jev fallback only; the main transcript has no equivalent (absence claim) |
| Honest stop vocabulary and escalation (`JevStatus`, `JevStopCause`, fallback trigger, `access_denied` before any decision) | `R:crates/roder-ext-jev/src/fallback/trigger.rs:57-85`, `R:crates/roder-ext-jev/src/block.rs:63-119` (classifier), `R:crates/roder-ext-jev/src/agent.rs:98-103` (wired before the first decision) | Jev |
| Rich Jev eval tiers: 52-task keyless corpus graded by POST log and DOM probes, live tier, `JEV_EVAL_VARIANTS` A/B slot | `R:crates/roder-ext-jev/src/fixture_harness/evals/grade.rs`, `R:crates/roder-ext-jev/tests/fixtures/evals/tasks.json` | Jev only |
| Webwright disk workspace and no-model rerun | `R:crates/roder-ext-webwright/src/workspace.rs:17-22`, `R:crates/roder-app-server/src/webwright.rs:281-342` | Webwright |
| Core artifact spill and generic stub compaction | `R:crates/roder-core/src/tool_output.rs:1-30`, `R:crates/roder-core/src/compaction.rs:173-205` | All, browser-blind |

Pattern: the Jev crate has most of QuickE2E's ideas. `browser_use_*`, extension `chrome_*`, native computer and webwright have almost none.

## Ranked recommendations

Ranking weights how often the failure occurs, how severe it is, and how strong the evidence is. Boring, deterministic, in-repo fixes rank above clever ones.

| # | Frequency | Severity | Evidence |
|---|---|---|---|
| 1 | every `browser_use` step | high | read in code and upstream source |
| 2 | enabler | enabler | saved native report, Jev rows |
| 3 | every extension click | high | code read; branch read, not run |
| 4 | 3 recorded runs plus one booking run | medium | recorded live runs |
| 5 | one task class | high (wrong delete) | recorded trace, failed in every live run |
| 6 | unknown | high when it hits | fixtures only |
| 7 | every non-Responses browser step | medium | code read |
| 8 | native computer only (opt-in, OpenAI) | high there | one saved run |

**Judgement calls on rank.**
- Measurement (#2) could go first or much later. I run it in parallel with #1, because #1 needs only a unit test.
- The Jev items #4-#6 sit above #7 and #8 because they are small, in-repo and (for #4, #5) backed by recorded failures; #6 is fixture-only. The weakest axis is still the frontier surfaces, which is why #1 and #3 lead. If capacity is short, swap #6 and #7.
- #4 fixes only one hosted-model failure, so it sits below the `browser_use` work.
- The advisory repeat line on frontier surfaces is #13, because Roder has no loop trace on those surfaces.

**Evidence caution.** Most QuickE2E "measured" figures are code comments with no run files. Where I quote them, they are labelled as recorded in a comment or doc. The rankings lean on Roder's own recorded failures and on reading the code.

### 1. `browser_use`: report-first, compact, truthful results

**What to change** (all in our Rust wrapper):
- Put the action report first, labelled as a claim.
- Parse `get_state` JSON into one line per element with a visible omitted count and a wrapper-side `offset` argument. Fall back to raw text on any parse miss.
- Drop `Agent` from the set of tools that trigger a post-action `get_state`.
- Classify exact pinned strings as `is_error`: `Element with index N not found` and `Error:` prefixes.
- Refuse `browser_click`/`browser_type` on an index whose tag is `select`, with a message that names a working route.
- Name browser loss in errors: when a transport error or timeout makes the wrapper shut down the client and drop the owned profile (`R:crates/roder-ext-browser-use/src/server.rs:170-196`), say the browser and its logins are gone and the next call starts fresh. Never restart silently.

**Why (Roder gap, read from code).**
- Upstream state is `json.dumps(indent=2)`, roughly 5-7 lines per element. A 30-element page measured 208 lines in a local check by the author (unreproduced), which exceeds core's 200-line cap (`R:crates/roder-core/src/tool_output.rs:1-4`). Core then swaps in a 6,000-char head-and-tail excerpt. In an author simulation (unreproduced), about 38 of 100 indices stayed visible.
- The wrapper's head-only 24,000-byte cut (`R:crates/roder-ext-browser-use/src/tools.rs:26`, `R:crates/roder-ext-browser-use/src/tools.rs:222-235`) is applied to the joined text (`R:crates/roder-ext-browser-use/src/tools.rs:186-199`), and the report is appended last (`R:crates/roder-ext-browser-use/src/server.rs:113-133`), so a cut drops the trailing Action report. Core spills the already-cut `result.text` (`R:crates/roder-core/src/tool_execution.rs:697-712`).
- Upstream returns "not found" and select clicks as non-errors, per upstream source I read. The wrapper passes `isError` through (`R:crates/roder-ext-browser-use/src/tools.rs:148-151`), so core's failure counters never see them. This premise rests on upstream behaviour only: the wrapper's own fixture at `tools.rs:342` uses `isError: true` for "Element 9 not found".
- For `browser_use_agent`, the wrapper appends a `get_state` of what I read as a different, usually blank browser, labelled "Observed page after the action" (`R:crates/roder-ext-browser-use/src/server.rs:126`) with "verify against the observation above" (`R:crates/roder-ext-browser-use/src/server.rs:131`). The agent path builds its own session and closes it (read in pinned 0.13.10 in the uv cache; not re-read for this edit). `observe` includes Agent at `R:crates/roder-ext-browser-use/src/tools.rs:114-119`; it is also set for Navigate and Act, which are legitimate.

**QuickE2E basis.** Visible drops (`Q:src/loop.mjs:260`, `Q:src/loop.mjs:313`) and cause-naming failures (`Q:src/loop.mjs:1256-1260`). The compaction principle echoes the Jev digest, which budgets 8,000 chars and 120 lines and says what it cut (`R:docs/jev-browser.md:1488-1550`).

**Where.** `R:crates/roder-ext-browser-use/src/tools.rs` (`render_result`, `truncate`), `src/server.rs` (`call_observed`), `src/catalog.rs` (notes, schema for `offset`; `offset` does not exist today), new `src/state_view.rs`, tests in `tests/fake_server.rs`.

**Interface.**
- `browser_use_get_state` gains optional `offset` (default 0), not forwarded upstream.
- Result text: line 1 an outcome summary, then the untrusted note, then the action report, then page header, then `[idx] tag "text" ph=... -> href` lines.
- Over budget (about 18,000 chars and 150 lines, under core's caps) it ends with `… N more interactive elements not listed. Call browser_use_get_state with offset B.`
- Select refusal text names `browser_use_agent` (if keyed), `jev_browse` and `chrome_select`, and says the last two drive a different browser.

**Test/eval.** Extend the fake server so `browser_get_state` emits an upstream-shaped pretty-printed state of N elements. Assertions:
- A 100-element state renders in at most 150 lines and 18,000 chars with every index exactly once.
- A 400-element state still shows the report and an exact omitted count.
- Stale index and select click return `is_error` and the server sees 0 clicks.
- The agent result has no foreign observation.
- Non-JSON state passes through unchanged.
- Secrets are redacted before cutting.
- A killed client yields an error that says the browser was reset.

**Effort/risk.** M (S for report-first, drop Agent from `observe`, exact-string `is_error`, and the browser-loss message).
- The parser is pinned to 0.13.10 while the local checkout I read is 0.12.5, so fail open.
- A hard element cap is safe only with the `offset` argument.
- `is_error` feeds core's 5-consecutive-failure stop (`R:crates/roder-core/src/reliability.rs:121-127`), so match only exact strings and assert them in the pinned-tools test.
- An epoch guard for parallel index calls must not refuse legitimate parallel typing; key it on element-map change.
- Do not reorder elements by role (QuickE2E reverted a role re-rank).

**Axes.** Not confused (high), not stuck (high), longer (medium), faster (medium; one fewer `read_artifact` round trip, about half the chars per state in the author simulation). Own decisions (low).

**Confidence.** High on the gaps (read in code, and in upstream source for the stale-index and select behaviour). Inferred: the simulated sizes, that parallel calls can hit a renumbered index, and that a select click is silent on the pinned version.

### 2. Measurement bed, minimal slice

**What to change.**
- **(a) Native grader.** Require exactly one trusted correct submit. Extract `grade_events()` from `Fixture::grade` at `R:crates/roder-ext-chrome/examples/native_computer/support.rs:171`, which uses `any()` over submit events.
- **(b) Jev rows.** Split `Row.pass` into `verdict_ok`, `truth_ok` and `false_green` (`R:crates/roder-ext-jev/src/fixture_harness/evals/rows.rs:15`).
- **(c) Repeats and baseline.** Add `JEV_EVAL_N` and a saved per-task baseline recording commit, worktree hash, corpus hash and model. Read the pin at run start and end.
- **(d) Telemetry.** Persist `omitted_actions`, the settle `{reason, waited_ms}`, and per-phase laps.
- **(e) One decoy page.** Copy the shape of the recorded `enter_to_search` false DONE: status `done`, 0 posts.

**Why.** The live tier runs each task once (`R:crates/roder-ext-jev/src/fixture_harness/evals/live.rs:244-253`). Its only gate, `JEV_EVAL_STRICT`, is all-or-nothing (`R:crates/roder-ext-jev/src/fixture_harness/evals/live.rs:275-277`). The audit's run was 49/52 (figure taken from the audit, not re-derived). The saved native report (`evals/reports/native-computer/2026-09-30/live-openai/report.json`) shows 4 submits, 3 wrong, 5 calls, 74.8 s, and still passed. By hand count of the 2026-09-29 `live-jev.jsonl`, 1 of 32 done verdicts is false (`enter_to_search`). Decision latency is 34% of summed wall (median 191.5 ms over 202 decisions), so about 66% of time is unattributed. The hosted service has returned HTTP 402 on earlier A/B attempts, so keyless proof matters.

**QuickE2E basis.** Verdict vs right-element separation (`Q:fixtures/README.md:3-44`), baseline compare that fails on regression (`Q:fixtures/run.mjs:132-150`), and a baseline once invalidated by a mid-run edit (`Q:fixtures/run.mjs:17-19`).

**Where.** `R:crates/roder-ext-jev/src/fixture_harness/evals/{rows.rs,grade.rs,live.rs}`, a new `tally.rs`, `R:crates/roder-ext-chrome/examples/native_computer/support.rs`, and a new `tests/native_grader.rs`.

**Test.** A no-Chrome test regrades the saved native report and expects `passed=false`. A Jev unit test builds a `Done` outcome with a failing DOM probe and expects `verdict_ok` true, `truth_ok` false, `false_green` true.

**Effort/risk.** S for (a), (b) and (e); M for (c) and (d). Measurement only, so the gain depends on acting on the numbers. Live latency is noisy, so gate on pass counts and offered-set facts, not wall time. Offered labels are untrusted page text: keep them host-only and scrubbed. Do **not** build a four-surface frontier-model bed (about 144 frontier runs) until a recorded `chrome_*` or `browser_use_*` wrong-target failure exists to seed it.

**Axes.** Indirect on all five. It makes false-green countable and attributes the unattributed time.

**Confidence.** High that the grader and rows are as described. The 1-of-32 count is my hand count of the saved file.

### 3. Chrome: paired extension and Desktop stop being blind

**What to change.**
- Merge and rebuild the extension branch `pz/computer-use-primitives` in `roder-web-extention` (`E:`, tip `40ffd05`), and add an `action-observation` capability to the hello message.
- Add a Rust-side compact result renderer: outcome line first ("No visible change", "URL a->b"), then controls before text.
- If an old build returns an action without `observation`, send one `page/snapshot` after 150 ms.
- Route `page/select` in `desktop_cdp.rs` to the existing direct select. That is more than routing: the direct select takes `{ref, option}` (`R:crates/roder-ext-chrome/src/direct/tools.rs:168`) while `chrome_select` is `{selector, value}` with both required (`R:crates/roder-ext-chrome/src/tools.rs:150-159`). On Desktop, resolve `selector` to a `ref` through the direct resolver, map `value` to an option by value then label, and list the options on a miss.
- Give `getText`/`highlight` an explicit "unsupported on Desktop" error.
- Treat the two backends as one contract: the same tool names currently return different schemas, observations and errors on the extension and on Desktop direct. The renderer and the parity test below are that contract.

**Why.**
- On extension master, click returns only `{ref}` and fires `element.click()` in a 150 ms timer. It takes the first match and has no dialog handling (`E:src/content/actions.ts:29-34`, `E:src/content/actions.ts:55-62`).
- Results are pretty JSON cut at 24,000 chars (`R:crates/roder-ext-chrome/src/session.rs:37`, `R:crates/roder-ext-chrome/src/session.rs:61-71`). Page text comes before controls, inferred from the fixture key order (`evals/fixtures/chrome/snapshot.json`), because the real extension serializer is not in this repo. By my arithmetic, roughly 20 of 160 controls survive. This is not measured.
- On Desktop, `chrome_select` answers that no extension or Desktop browser is connected (`R:crates/roder-ext-chrome/src/desktop_cdp.rs:34-38`, `R:crates/roder-ext-chrome/src/tools.rs:365-373`) although a working direct select exists (`R:crates/roder-ext-chrome/src/direct/act.rs:313-345`).
- The fix for the extension already exists on the branch. Rust cannot detect a stale build because it stores only hello capabilities (`R:crates/roder-api/src/chrome.rs:142`, `R:crates/roder-api/src/chrome.rs:329`).
- Parallel tool calls default on (`R:crates/roder-core/src/runtime.rs:5752`) and extension refs are shared across calls and forgeable, so the ref-forgery fixture below also covers parallel use.

**QuickE2E basis.** Unique-or-fail target resolution (`Q:src/loop.mjs:611-631`; its security note records that `.first()` on "Delete" after a re-mount deleted Row A when Row B was chosen) and DOM-quiet settle (`Q:src/loop.mjs:461-477`).

**Interface.** `chrome_select {tabId?, ref?, selector?, value}`, required `[value]`, with errors listing options on a miss. Compact action results as above.

**Test.** Extend `tests/fixtures/primitives.html` and `tests/computer_use.rs` with a dead button, a covered button, an SPA update and a `data-roder-ref` forgery. Add a six-case `desktop_select` test (Growth chosen by ref and by selector, miss lists options, non-select refused, no target errors, restoring page reported). Parity test: the same fixture through Desktop and extension yields the same outcome sentence.

**Effort/risk.** M, spanning two repos that must ship together; an extension reinstall is needed. The change signature can read "unchanged" on canvas or animation-only pages. Dropping bounding boxes forces ref-based action. **Do not** copy QuickE2E's per-click settle with an unfiltered `getAnimations` wait (up to 400 ms per click in `Q:src/loop.mjs:462-474`).

**Axes.** Not stuck (high), not confused (high), faster (medium), longer (medium).

**Confidence.** High on the code facts. The branch behaviour is read, not run.

### 4. Jev: retry an unusable decision reply, then hand off

**What to change.** Tag `validate_choice`/`read_answer` failures with an `UnusableAnswer` marker inside the existing `JevBilled` wrapper. In `Agent::run`, beside the `StaleObservation` arm, keep billed usage and re-ask up to 2 times. On exhaustion, set a new `JevStopCause::DecisionUnusable` and map `Error` plus that cause to the fallback.

**Why.**
- Any non-stale error currently ends the run: `Err(error) => return Some(self.fail(&error))` (`R:crates/roder-ext-jev/src/agent.rs:148`), via `R:crates/roder-ext-jev/src/decide.rs:284-324`, `R:crates/roder-ext-jev/src/decide.rs:394-418` and `R:crates/roder-ext-jev/src/decide.rs:350-351`. `status_of` maps it to `Error` (`R:crates/roder-ext-jev/src/engine.rs:350-357`) and `trigger()` returns `None` for it (`R:crates/roder-ext-jev/src/fallback/trigger.rs:61-84`).
- Recorded live runs: `date_field` failed validation with "Invalid TypeSafe response" and then passed three reruns (`R:docs/jev-browser.md:1006-1008`); `delayed_spa` and `below_the_fold` ended on "an invalid decision reply" (`R:docs/jev-browser.md:1051-1052`); one booking run ended on "the decision service returned an unusable answer" (`R:docs/jev-browser.md:1657`).

**QuickE2E basis.** Partial and different in kind. An unoffered key is coerced to BLOCKED (`Q:src/loop.mjs:1067`), and BLOCKED waits up to 3 s for a DOM mutation and counts strikes to 3 (`Q:src/loop.mjs:1133-1147`). That is a blocked-discipline counter, not a malformed-reply retry. The retry rationale here is Roder's own, from its recorded runs.

**Test.** In `tests/agent_limits.rs`, update `billed_calls_with_unusable_answers_count_in_the_usage`. Invalid-once then valid must end Done with `model_calls` 2. Always-invalid must end after 3 calls with the new cause and `trigger()` returning `Some`.

**Effort/risk.** S. A deterministic hosted model may return the same bad reply, wasting up to 2 billed calls, so keep the cap at 2. Routing an exhausted `Error` to the fallback reverses the documented "never on an error" rule in `R:crates/roder-ext-jev/src/fallback/trigger.rs:7-12`; that is a product decision, and the only one in #4-#6. The stop reason should state the reply count and first validation reason. **Update 2026-10-09: the owner decided that an exhausted decision error does fall back to the frontier model; it shipped (see the status section), so this is no longer an open product decision.**

**Axes.** Longer (high), not stuck (medium), own decisions (medium). Slightly slower on bad-reply runs.

**Confidence.** High. The failures are recorded, and the retry path is in code.

### 5. Jev: twin-row repeat guard

**What to change.** In `repeats_a_finished_click` (`R:crates/roder-ext-jev/src/agent.rs:290-305`), add an arm that fires when all of these hold:
- The label equals the last page-changing click's label.
- The choice id differs.
- The label passes `names_commitment` (`R:crates/roder-ext-jev/src/irreversible.rs:74`).
- Call confidence is below the existing `REPEAT_CONFIDENCE` of 0.7.

The run then ends Done without clicking. Surface the suppressed click's context to the caller.

**Why.** The only recorded Jev failure with a fully understood mechanism. In `icon_by_picture` (`evals/reports/browser-computer-use/2026-09-29/live-jev.jsonl`, line 18; the fallback-focused live run shows the same pattern), Jev clicked e6 (delete Bob Budget) at 0.99, then e9 (delete Bob Lunch) at 0.60, then BLOCKED. The guard compares choice ids, so a different twin id slips past. The task failed in every recorded live run; the keyless run passed because it has no live model. The 0.7 threshold was set on same-target repeats (`R:crates/roder-ext-jev/README.md:565-570`), not twin rows.

**QuickE2E basis.** History that agrees with the page and rows keyed by context (`Q:src/loop.mjs:836-842`, `Q:src/snapshot.js:399-433`).

**Test.** In `tests/agent_loop.rs` with twin "delete" ids e6/e9: confidences `[1.0, 0.6]` end Done with 1 act and 2 model calls; `[1.0, 0.8]` clicks again; a non-commit label at 0.6 is unchanged. Add a scripted `icon_by_picture` variant asserting mail3 stays.

**Effort/risk.** S. A goal like "remove all Bob mails" could end Done early below 0.7, so re-measure the threshold and limit to commit-named labels. Feeding `effect`/`context` into `recent_actions` (`R:crates/roder-ext-jev/src/decide.rs:253-262`) is a separate follow-up behind `JEV_EVAL_VARIANTS` (see #17).

**Axes.** Not confused (high, one task class), not stuck (medium), own decisions (medium).

**Confidence.** High on mechanism and trace. The fix has not been run.

### 6. Jev: the run cannot silently burn its budget

**What to change.**
- **(a) Run-wide pair counter.** A map keyed (observation fingerprint, chosen id), limit 4, so the three-in-a-row rule still fires first. A fourth identical pair ends Blocked with new `JevStopCause::Looped`.
- **(b) Stale-reread cap.** Count consecutive `StaleObservation`, reset on any recorded step. At 3, stop Blocked with a new `Unsettled` cause naming the last stale message. Count only stales whose re-observation shows no progress.
- **(c) Wait cap.** Treat unchanged waits as transparent in `stalled()` and stop after 6 consecutive unchanged waits.
- **(d) Cross-call line.** If three consecutive non-done calls end on the same page fingerprint, add one digest line to the outer agent.

`Looped` and `Unsettled` are Blocked-status causes, mapped to the fallback in `trigger.rs` the same way the existing Stalled/Covered causes are. That follows the current policy and needs no product decision.

**Why.**
- The stall rule needs three consecutive chosen non-wait no-ops (`R:crates/roder-ext-jev/src/agent.rs:464-481`). It misses A-B-A-B, open/close, and wait-interleaved no-ops. The docs concede the move-history case (`R:docs/jev-browser.md:1845-1848`). The step fingerprint leaves out `context` and offscreen actions so a below-the-fold countdown cannot defeat the stall rule (`R:docs/jev-browser.md:820-822`); that an on-screen ticker would count as progress is my inference.
- Stale re-decides are never recorded, so the next decision repeats. With the 120-decision and 60-action budgets (`R:crates/roder-ext-jev/src/agent.rs:229-230`), the 120 s default timeout likely fires first. `timed_out` never falls back (`R:crates/roder-ext-jev/src/fallback/trigger.rs:79-83`), so the handoff is forfeited. That ordering is inferred.

**QuickE2E basis.** Pair limit 4 (`Q:src/loop.mjs:31`) and named stop (`Q:src/loop.mjs:1105-1118`). Evidence is field counts in a comment (8x same field, 11x wrong combobox, 24x autocomplete, `Q:src/loop.mjs:785`). Treat it as cheap insurance, not a measured win.

**Test.** Add `toggle-menu.html` and `an_open_close_cycle_ends_looped` in `tests/agent_loop.rs` (ScriptedBrowser pages A,B,A,B): expect Blocked, cause Looped, 6 acts, 7 model calls. Add an on-screen-ticker form fixture ending within 3 stale decisions. Keep `stall_detection` and `step_budget` as no-regression guards. Gate on zero new Looped/Unsettled stops across the 52 keyless tasks.

**Effort/risk.** M. Evidence is anecdotal and there is no live A-B-A-B trace yet, so keep it fixture-gated. A false trip costs one fallback (median 15-16 s per the state digest), not a failed task. Digit-masking hides real numeric progress in this rule only. A cap of 3 could cut a slowly hydrating SPA. Leave `page_changed` and the upstream-parity fingerprint untouched. **Update 2026-10-09: the owner decided that nothing is masked; the stale cap shipped comparing pages exactly, so a digits-only ticker is left to the budgets (see the status section).**

**Axes.** Not stuck (high), longer (high), faster (medium; about 6 actions instead of up to 60 on an oscillating fixture, estimated), own decisions (medium).

**Confidence.** Medium. Gaps are read in code. The benefit rests on fixtures, not a real trace.

### 7. Stop advertising screenshots providers never receive

**What to change.** Add a default-false `InferenceEngine::tool_result_image_input()` (new symbol), true for Responses. In `runtime.rs`, build the request copy without `__view_image` for engines returning false and append a one-line notice. Never rewrite the persisted transcript. Extend `native_tool_supported` to hide screenshot tools for those engines. As a follow-up, skip the per-action screenshot request in `browser_use` on those engines (inferred saving).

**Why.** Only the Responses replay forwards `__view_image` (`R:crates/roder-ext-openai-responses/src/response_replay.rs:110-118`; grep finds the helper in no other provider crate). Anthropic, Gemini, Vertex and chat-completions send tool results as text. Tool text still says "attached" (`R:crates/roder-ext-browser-use/src/tools.rs:175`, `R:crates/roder-ext-chrome/src/session.rs:57`, `R:crates/roder-ext-chrome/src/direct/capture.rs:62`). Prompt accounting charges 1,600 tokens per result with no provider check (`R:crates/roder-core/src/prompt_accounting.rs:16-23`), and compaction uses it. `browser_use` requests a screenshot on every action (`R:crates/roder-ext-browser-use/src/server.rs:117`).

**QuickE2E basis.** None. #7 rests on Roder code facts alone. QuickE2E never compared text with screenshots; its 35/120 to 120/120 bakeoff result is a better text snapshot (shadow-root/iframe piercing plus a goal-anchored keep), not text versus images.

**Test.** A mock engine with image input false over a 30-step fake `browser_use` run: no `data:` URL or `__view_image` in the request, no screenshot tools advertised, notice appears once. A Responses regression test that `input_image` still emits.

**Effort/risk.** S. Hiding screenshot tools removes visual verification on non-Responses providers until forwarding exists (a canvas page loses its only evidence). Mid-thread model switches leave history calling hidden tools. Responses assumes images for every profile except Xai, including OpenRouter and Fireworks regardless of model (`R:crates/roder-ext-openai-responses/src/provider.rs:221-227`). Open question: if other providers should get images, forwarding may beat hiding.

**Axes.** Not confused (medium to high), longer (medium; no phantom accounting), faster (small).

**Confidence.** High on the code facts.

### 8. Native computer: never lose the screenshot, return the facts

**What to change.**
- Collect per-step hit label, dialogs, new-tab and HTTP facts in `run_computer`, and replay them as a capped text sidecar on success too.
- Substitute a static "screenshot unavailable" image instead of bailing when no image exists.
- Remap Control chords to Meta for editable focus on a Mac page, and say so.
- Stop a batch after a navigation or new tab, naming the unrun actions.

**Why.** `run_computer` keeps only a completed count, failure text and `typed_secret` (`R:crates/roder-ext-chrome/src/direct/computer.rs:24-45`) and returns a screenshot whose text is just the viewport size (`R:crates/roder-ext-chrome/src/direct/capture.rs:61-63`). Replay adds text only when `is_error` (`R:crates/roder-ext-openai-responses/src/response_replay.rs:87-95`). The facts are already computed in `after()` (`R:crates/roder-ext-chrome/src/direct/session.rs:190-237`) and dropped. Cmd chords map to editing commands on Mac (`R:crates/roder-ext-chrome/src/direct/keys.rs:206-219`); that Ctrl chords do not is my inference from the saved run's `penguinorcaA`-style submit values. The saved live run: 4 submits (3 wrong), 5 calls, 74.8 s; the visible run: 1 submit, 3 calls, 28.8 s (uncontrolled comparison). Scope: native computer only, OpenAI, opt-in, so reach is narrow.

**QuickE2E basis.** Name the cover in history (`Q:src/loop.mjs:1257-1260`), and counting repeats (`Q:src/loop.mjs:1105-1117`).

**Test.** Extend the scripted ACP fixture in `acp_native_computer.rs` with a Mac-reporting page and the failing batch (Ctrl+A, type orca, Shift+a, Enter): wrong submits 3 to 0, calls at most 3. Add a four-trap fixture (500 page, login wall, `target=_blank`, confirm) asserting the result text names each cause.

**Effort/risk.** M (grader fix is S, see #2). Page strings (title, dialog text, labels) enter a user-role message: keep the untrusted header, cap, quote and scrub (dialog text is not scrubbed today, `R:crates/roder-ext-chrome/src/direct/client.rs:279-285`). Extra text shifts the cached prefix, so emit notes only for notable events. I did not verify that the API accepts text inside `computer_call_output`; copy the existing error-path sidecar. Control-to-Meta breaks Emacs-style Ctrl+A pages.

**Axes.** Not stuck (high on native runs), not confused (high), faster (medium).

**Confidence.** High on the code. The two reports are uncontrolled single runs.

### 9. Handoffs are outcomes, not tool failures; unattended approvals deny

**What to change.** In `R:crates/roder-ext-jev/src/tools.rs:144` set `is_error=false` with `data.outcome_class` for `needs_input`, `needs_confirmation` and `access_denied`. Flip back to true on an identical repeat on the same URL. In `tool_approvals.rs`, deny immediately under non-interactive runtime profiles with actionable text.

**Why.** Every non-done Jev status is `is_error`, so legitimate handoffs count toward the 5-consecutive-failure stop (`R:crates/roder-core/src/reliability.rs:34`, `R:crates/roder-core/src/reliability.rs:121-127`). The approval wait is a bare `rx.await` (`R:crates/roder-core/src/runtime/tool_approvals.rs:142`), while `request_user_input` already short-circuits (`R:crates/roder-core/src/tool_execution.rs:1254-1266`). The Eval profile continues past the failure limit, so the effect is for non-eval non-interactive turns (inferred).

**QuickE2E basis.** A fault-class taxonomy where exit codes say nothing about the app (`Q:AGENTS.md:143-157`).

**Test.** Five scripted handoff results do not trip `ConsecutiveToolFailures`; an identical repeat does. A Default-mode approval under non-interactive resolves denied in under 1 s.

**Effort/risk.** S to M. Changing `is_error` alters how providers and UIs render these results. Gate immediate denial on `RuntimeProfile` only, never on a missing client, or app-server approval breaks. A predicate-wait tool is a larger follow-up that holds a tool slot.

**Axes.** Own decisions (high), longer (medium to high), not stuck (medium).

**Confidence.** Medium to high. Read in code, not exercised.

### 10. Webwright: make a failed run explain itself

**What to change.**
- Return an outcome class, elapsed time, a redacted stderr tail (about 4 KB) and the first error line in the `run_script` text.
- Add `kill_on_drop(true)` to the `Command` chain at `R:crates/roder-ext-webwright/src/tools/support.rs:94-97`.
- Make a nonzero exit fail verification.
- Preserve the manifest on `create()`.
- Add a per-run policy contributor.

**Why.** `run_script` returns `exited with {:?}` (`R:crates/roder-ext-webwright/src/tools.rs:320-330`); stdout, stderr and `timed_out` sit in `data`, which the display whitelist drops (`R:crates/roder-api/src/transcript.rs:88-114`). A timeout drops the future with no kill, so the child can survive (`R:crates/roder-ext-webwright/src/tools/support.rs:88-125`; inferred from tokio semantics, no `kill_on_drop` exists anywhere in the crate). `verify.rs` checks plan boxes, a PNG and a "final datum" line, never the exit code (`R:crates/roder-ext-webwright/src/verify.rs:70-159`). No `PolicyContributor` is registered (`R:crates/roder-ext-webwright/src/extension.rs:38-42`); browser-use and Jev register one, and the chrome crate has none either.

**QuickE2E basis.** Failure-evidence fields on every result record (`Q:src/loop.mjs:1347-1366`) and the engine-error versus app-failure split (`Q:AGENTS.md:143-157`, `Q:src/loop.mjs:1291-1297`).

**Test.** Offline fixtures: a script that exits 1 but prints `final datum:` and writes a PNG must now fail verification. A pid-file script asserts the process is dead after a 1 s timeout. Per-class stderr fixtures.

**Effort/risk.** S for text, exit code and `kill_on_drop`; M for the approval policy (per-run approval adds prompts in Default mode; show the script hash and let AcceptAll allow). Substring classification of Playwright messages is brittle; treat the class as a hint and always include the raw tail. Defer verdict vocabulary and script diffs until a content assertion exists.

**Axes.** Not stuck (high for webwright; 2+ calls per failure cycle down to 1), not confused (high; false-green gone).

**Confidence.** High on the code.

### 11. Tell the main agent which browser surface to use

**What to change.** A roughly 120-word tool-name-conditional block in core, modelled on `apply_parallel_web_tools` (`R:crates/roder-core/src/instructions.rs:254`, called at `R:crates/roder-core/src/runtime.rs:3884`). Emit it only when two or more browser families are advertised, one line per advertised family. Name alternatives in Jev's blocked hint ("use another browser tool", `R:crates/roder-ext-jev/src/report.rs:69`) and in the missing-key error (`R:crates/roder-ext-jev/src/runner/mod.rs:92`).

**Why.** About 38 browser schemas ship before Jev's tools (16 `browser_use_*`, 22 `chrome_*` by my count); only `browser_use_navigate` carries routing text (`R:crates/roder-ext-browser-use/src/catalog.rs:26-30`). `jev_browse` is always registered and fails at call time on a missing key.

**Test.** Unit tests: block absent with zero or one family, lists only advertised families, under 150 words. Behavioural: 12 prompts crossed with tool sets, graded like `wrong_tool_family` in `roder-evals`, with a baseline measured first.

**Effort/risk.** S. Over-steering toward cheap `jev_browse`; core cannot see key or pairing state, so a listed surface can still fail. Changing the tool set mid-thread changes the cached prefix. No QuickE2E measurement backs a specific rule; the rules are inferred from Roder's docs.

**Axes.** Own decisions (high), not stuck (medium).

**Confidence.** Medium. The gap is verified. The payoff is unmeasured.

### 12. Jev `success_condition`: normalise first, early-end second

**What to change.**
- **Step 1 (S).** Fold whitespace, NBSP, zero-width characters and case on both sides in the match. Add `text_absent`. Exclude lines that only echo a typed field value.
- **Step 2 (M, opt-in, A/B-gated).** Pass the condition into the Agent. End Done on the first fresh snapshot after at least one non-covered, non-wait action that shows `text_contains`. Require arming (a snapshot seen not holding), report `satisfied_at_start`, and word the result "condition met", never "verified".

**Why.** Matching is raw case-sensitive `contains` (`R:crates/roder-ext-jev/src/session/completion.rs:36`) on text cut to 6,000 chars (`R:crates/roder-ext-jev/src/runner/look.rs:65`). `text.js` joins nodes with newlines, so "Count: 1" never matches "Count:\n1", and typed values count as page text (`R:crates/roder-ext-jev/src/assets/text.js:74`). The check runs only after DONE (`R:crates/roder-ext-jev/src/session/call.rs:163-167`), and no corpus task sets a condition.

**QuickE2E basis.** Check before deciding, not after acting (`Q:src/loop.mjs:954-990`), and weak-assertion guard (`Q:src/loop.mjs:1299-1305`). Evidence is anecdotal plus a neutral DONE-removal bakeoff (`Q:docs/bakeoff-2026-09-24.md:506-512`, which cannot show a benefit because no run chose DONE). Do not remove DONE and do not re-ask after a rejected DONE.

**Test.** `counter.html` with plan [click, click] and `text_contains 'count: 1'` passes today only by luck (2 actions, blocked `outcome_mismatch`); target is Done after 1 action. Add NBSP/newline/case variants and a true-at-load case. Live A/B on `icon_by_picture`, `contact_form`, `enter_to_search` and a new overshoot task, n>=3 per arm. If any annotated task passes without the early end and fails with it, demote it to a flag.

**Effort/risk.** S for step 1, M for step 2. The predicate is the outer model's guess: too loose (true at load) or never matching (a vanished toast, text past the 6,000-char cut). The live A/B may be blocked by HTTP 402. Extending a code-owned end-state check to `chrome_*`/`browser_use_*` (the direct analogue of QuickE2E's central idea) is not ranked: no recorded failure seeds it (see Open questions).

**Axes.** Not confused (medium-low; stops overshoot, inferred n=1), faster (small; one decision per annotated run), own decisions (medium).

**Confidence.** Medium-low. Impact is narrow and caller-dependent.

### 13. Advisory repeat line on frontier-driven surfaces (baseline-gated)

**What to change.** In `DirectSession::run` (reaches Desktop `chrome_*`, `jev_tab_*` and the Jev fallback), hash (tool, args) plus a page signature (URL, scroll, first 50 elements' role/label/value/state, first 1,200 text chars, digits collapsed). From the third identical repeat on an unchanged page, append a "Repeat check" line naming causes to check. Keep the log in a per-target static map, because each Desktop call builds a fresh `DirectSession`.

**Why.** Nothing on these surfaces says a repeated action did nothing. Core counts only `is_error` results (`R:crates/roder-core/src/reliability.rs:121-127`), and `browser_use` fetches state after each action but never compares it (`R:crates/roder-ext-browser-use/src/server.rs:113-137`).

**QuickE2E basis.** Field repeat counts of 8x, 11x and 24x (same field, wrong combobox, autocomplete; comment at `Q:src/loop.mjs:785`). Many of its LOOPs were environmental (late hydration, cookie banner, covering label), so the line must name causes, not blame the model.

**Gate.** Run a frontier fallback model n>=5 on a dead "Load more" fixture first (a Roder-designed probe). If the median is 2 or fewer repeats of the dead click, drop this item.

**Effort/risk.** M. Advisory only, never `is_error` and no refusal, because core stops non-interactive turns after 5 consecutive errors. Skip `browser_use`: its state has no input values and would false-trip. False positives on pagination and counters mean text stays in the hash.

**Axes.** Not stuck (medium), longer (medium).

**Confidence.** Low until the baseline exists.

### 14. Superseding stale page observations on the main transcript (measure-first, default off)

**What to change.**
- Slice 0: a keyless 40-step transcript-growth fixture for `browser_use` and chrome, plus a `JEV_FALLBACK_KEPT_READS` A/B knob.
- Slice 2, only if Slice 0 or a live run shows decay: lift the Jev fallback's `compact` (`R:crates/roder-ext-jev/src/fallback/run.rs:448-481`) into a shared chunked pass keyed on a reserved display key. Keep the newest 2 reads and 1 image. Stubs must record the action and say the page state was "then, not now".

**Why.** Every `browser_use`/chrome result with a state and an image stays in the transcript. Generic stubbing is token-distance based, runs only near the compaction threshold, and is skipped for native-compaction providers (`R:crates/roder-core/src/compaction.rs:173-205`, `R:crates/roder-core/src/transcript_compaction.rs:49-58`). Slice 1 of #1 shrinks per-step size first.

**QuickE2E basis.** Each decision is a fresh bounded request. `Q:bench/results/launch-task.json` records `tokens` 7,124 over 7 hosted steps (identical in all five hosted runs, which is suspicious) against 122k-161k over 15-16 steps for Claude Code with Playwright MCP. The two arms count tokens differently (input only versus input plus cache plus output), so this is direction, not proof. Stale history also made the model answer BLOCKED (`Q:src/loop.mjs:836-838`).

**Effort/risk.** M, depends on #1 and #2. Editing mid-transcript items breaks the Responses WebSocket exact-prefix delta and provider caches, and cache loss cannot be measured today (no cached-token field was found in `roder-api/src`). Over-eliding removes evidence. Revisit-an-old-page tasks may regress. Gate: last-call input tokens at most 0.5x baseline and pass rate not below baseline at n=3, including a revisit task.

**Axes.** Longer (high if the gate passes; about 53k down to about 5k tokens over 40 steps is an author estimate, unreproduced), faster (medium). Not confused is mixed.

**Confidence.** Low until growth is measured.

### 15. Jev: make omissions visible and fix two small bugs

**What to change.**
- Read `omitted_actions` (`R:crates/roder-ext-jev/src/assets/snapshot.js:354`) into `JevRunResult` and the digest header. Today only a test reads it.
- Count select options skipped past 255 targets (`R:crates/roder-ext-jev/src/space.rs:101-122`) so a 300-option list or a second long select is no longer silently unoffered.
- Fix `controls()` so native checkbox/radio report `checked`/`unchecked`, not the HTML value `on` (`R:crates/roder-ext-jev/src/agent/result.rs:133-142`), and render `[unchecked]` in `R:crates/roder-ext-jev/src/report/digest/options.rs:97-98`.

**Why.** Cheap hygiene. A goal-anchored keep is telemetry-first: Jev's caps are 100/250, four to eight times looser than QuickE2E's 30/52, and no recorded run shows a goal target cut. Do not credit QuickE2E's T3 0/40 to 40/40 (`Q:docs/bakeoff-2026-09-24.md:94`, `Q:docs/bakeoff-2026-09-24.md:164`) as Roder's expected gain.

**Effort/risk.** S. Do not tell the hosted chooser about omissions without an A/B. Any later goal-anchored keep needs a bound near 20, exclusion of generic and destructive twin labels, and the viewport-distance order left intact.

**Axes.** Not confused (low to medium), not stuck (low), faster (low).

**Confidence.** High on the gaps. The benefit is unproven.

### 16. Jev: name dead ends and refuse a DONE contradicted by a failed POST

**What to change.** Phase 1 (S): extend start-page handling to a persistent main-frame 5xx after one 1 s reload, with a new `server_error` status and zero decisions. Rename a blocked or `needs_input` run that ended on a 5xx page. Phase 2 (M, only if a live optimistic-cart fixture shows the false DONE): track same-origin mutating requests (CDP `Network`) and downgrade a DONE contradicted by an uncleared 5xx. Sign-in redirect becomes a named hint on runs that already failed, not a hard stop.

**Why.** Jev handles 401/403/429 and challenge markers at start (`R:crates/roder-ext-jev/src/block.rs:63-119`), not 5xx. A dead 500 page costs a decision, a BLOCKED and a frontier fallback (median 15-16 s). The optimistic "Added to cart" with a failing POST shape comes from QuickE2E's wave-2 test (`Q:src/loop.mjs:735-736`); I found no Roder run with it. The nearest recorded Roder false done is `enter_to_search` (`R:docs/browser-computer-use-audit.md:135`), which has no POST at all.

**QuickE2E basis.** SERVER_ERROR and sign-in stops (`Q:src/loop.mjs:733-747`, `Q:src/loop.mjs:817-826`). QuickE2E gates sign-in on a declared storageState (`Q:src/loop.mjs:822`) and its regex also matches `/author` and `/oauth`; Roder has no such gate. The 3-BLOCKED rule is at `Q:README.md:599`.

**Effort/risk.** M overall; Phase 1 is S. False stops on sites that serve working pages with a 5xx status. `Network.enable` adds event load, so measure per-step latency (budget +20 ms). Jev reads events only inside calls, so a late response fails open. Chrome direct gets text only, never a stop. Real-world frequency of 5xx/sign-in dead ends in Roder runs is unknown.

**Axes.** Not confused (medium for the false-DONE guard), not stuck (medium), own decisions (medium).

**Confidence.** Medium-low. Evidence is QuickE2E fixtures, not Roder runs.

### 17. Jev: feed failure reasons to the chooser (variant-gated)

**What to change.** Safe now: put the cover's name in the digest and fallback prompt (carry it from `R:crates/roder-ext-jev/src/assets/act.js:102` through `Covered::new` at `R:crates/roder-ext-jev/src/page/act.rs:308`, whose message today is fixed text with no name), and fix chrome's covered-click advice, which says "press at x/y" although `chrome_click` has no coordinates (schema is selector/text/ref, `R:crates/roder-ext-chrome/src/tools.rs:179-188`; the message is shared with direct tools that do take x/y, `R:crates/roder-ext-chrome/src/direct/target.rs:71-79`, `R:crates/roder-ext-chrome/src/direct/tools.rs:74-88`). Model-facing: extend the existing `Variant::Effect` (`evals/variants.rs`, `evals/effect_variant.rs`) rather than adding a new variant. Add a failure-only, same-page-only filter (refused/covered/no-visible-change, capped at 100 chars, scrubbed) and the cover-name field to what it injects into `recent_actions`.

**Why.** `recent_actions` carries only action/kind/text/page_changed (`R:crates/roder-ext-jev/src/decide.rs:253-262`). Two of four live Resy runs stalled on blind retries (`R:docs/jev-browser.md:1654`, `R:docs/jev-browser.md:1656`). The `effect` variant exists but its A/B never ran (HTTP 402; `R:docs/jev-browser.md:616-623`).

**Effort/risk.** S to M. The hosted model may misread free text (QuickE2E measured BLOCKED at 0.37 when state was shown as a non-choice, `Q:README.md:720`). Cover text is page-sourced and a possible injection vector. `choose_request.json` must stay byte-identical for clean histories.

**Axes.** Not stuck (medium; target decisions per refused/covered target 3 to at most 2, estimated), not confused (medium).

**Confidence.** Low until the A/B runs.

### 18. Jev: optional `inputs` map in front of the text helper

**What to change.** Promote the eval-only `TaskValues` resolver (`R:crates/roder-ext-jev/src/fixture_harness/evals/scripted.rs:287`) and its `Supervised` wrapper (`R:crates/roder-ext-jev/src/fixture_harness/evals/text_sources.rs:22`) into a production `InputsResolver` (new name) behind an optional `jev_browse` `inputs` object. Match on folded label tokens in code first and fall through to the text helper on any miss. Add a pin test that typed values stay visible in `elements[].value` and `recent_actions[-1].text`.

**Why.** Each text-helper fill is a call with median latency 0.72-2.1 s (p95 4.2-17.3 s for the slower model; `R:docs/jev-browser.md:1174-1175`), and with no text model every fill ends `needs_input`. The pin test guards against anyone adding "(filled)" masking, which cost QuickE2E 5/5 to 0/5 (`Q:src/loop.mjs:277-278`).

**Effort/risk.** S to M. Values still pass through frontier tool arguments, so do not market it as a secrets path. A miss must fall through. Same-label fields get the same value. Caller-only text might regress copy-paste tasks where only one text-helper model succeeds (`R:docs/jev-browser.md:1177-1181` shows per-model wins; the extension to caller-supplied text is my inference).

**Axes.** Faster (medium on N-field forms, inferred N times median latency), own decisions (medium).

**Confidence.** Medium. Gain depends on label matches.

### 19. Probe first: build nothing until a fixture reproduces the failure

Write the cheap probe, run it once on current code, and build only on a fail. Each item's only evidence is a single comment or a synthetic fixture.

| Probe | Question |
|---|---|
| `late-enable.html` | Does live Jev pick WAIT or BLOCKED on a late-enabled submit? QuickE2E waits up to 3 s for a DOM mutation before counting a BLOCKED strike (`Q:src/loop.mjs:1133-1147`); Jev's BLOCKED ends the run. (`blocked-discipline`) |
| Slow link `?delay=1500` and `3000` | Does Jev's current settle already catch a slow link? (`link-click-url-wait`) |
| `settle-hydrate.html` | Do SSR buttons with late handlers get a dead click? (`settle-dom-quiet`) |
| 300 ms toast | Is a short-lived toast missed in more than 10% of runs? (`transient-text-recorder`) |
| `required-then-optional.html` | Does the blocked digest name empty required controls? (`hold-back-submit` diagnostic only) |
| Typing-ignoring calendar input; slider; todo-enter page | Are Jev's offered options enough? (`text-field-menu-rules`, `affordance-synthesis`) |
| MUI/antd opacity-0 checkbox in chrome look | Does `direct.js` list it? (`click-targets-aria-hides`, `inpage-dom-snapshot`) |

Building before probing recreates QuickE2E's reverted experiments. Only remove options, never re-rank.

## By goal: longer / faster / not stuck / not confused / own decisions

**Longer. Weakest axis on the frontier surfaces.**
- Moves it: #1 (flat per-step size, and a browser-loss message instead of silent state loss), #4 (a long run no longer dies on one bad reply), #6 (budget not burned on oscillation), #14 (supersession, if measured).
- Today `browser_use` and chrome results grow with every step, and nothing supersedes old observations in the main transcript. Only Jev's fallback does.
- The numbers I can quote are author simulations, unreproduced: about 38 of 100 indices visible today, and 15,462 to 8,255 chars at 100 elements.

**Faster. Real savings are small and mostly come from skipping work.**
- Moves it: #1 (no `read_artifact` round trip), #12 step 2 (one decision per annotated run, 144-421 ms each in recorded rows), #18 (median 0.7-2.1 s per matched field), #6 (about 5 s instead of about 50 s on an oscillating fixture; inferred).
- First attribute the 66% of unattributed wall time (#2). Do not build speculation or a local chooser: QuickE2E counts discarded speculative calls in cost (`Q:README.md:164-170`) but records no hit-rate or break-even figure, and Roder has no hit-rate data either.

**Not stuck.**
- Jev: #4, #5, #6.
- Frontier surfaces: #1 and #3 turn silent dead clicks into errors with a named route. #13 adds an advisory line only if a baseline shows more than 2 repeats of a dead click.
- Webwright: #10.

**Not confused.**
- Jev: #5 (wrong destructive twin), #12 (overshoot), #17.
- Frontier surfaces: #1 (report never cut, no blank-browser observation), #3 (first-match clicks), #7 (phantom screenshots), #8 (native facts).
- Eval: #2 (false greens countable).

**Own decisions.**
- Moves it: #11 (routing block), #9 (handoff versus fault), #16 (the cause reaches the caller), #12 (the caller's end state ends the call).
- Default surface by task kind:
  - `jev_browse`: one bounded goal on a public or fresh-session site; always pass `success_condition`.
  - `chrome_*`: anything needing the user's signed-in Chrome, debugging, or after Jev stopped on a wall. Ship the extension branch first.
  - `browser_use_*`: isolated step-by-step exploration, but only after #1.
  - `webwright`: repeatable or evidence-producing flows, once verify checks exit codes.

**Weakest axis today:** "longer" on `browser_use_*` and extension `chrome_*`, followed by "not stuck" on every surface except Jev.

## Roder-native pain points QuickE2E does not solve

These come from Roder's own architecture. QuickE2E has no analogue, so the fixes are Roder-native.

- **Frontier tool-schema honesty (#7).** Screenshots promised to providers that cannot receive them.
- **Two-browser confusion in `browser_use_agent`** (w15). The agent runs in its own session and closes it; the wrapper then observes another browser (`R:crates/roder-ext-browser-use/src/server.rs:126`, `R:crates/roder-ext-browser-use/src/server.rs:131`). Fix: drop the observation and say so (in #1). Also rewrite the `model` description to OpenAI ids only, because the server builds only `ChatOpenAI`, and consider `temperature` 0.2 and a shorter 10-minute timeout.
- **`browser_use` lifecycle** (w25). A transport error or timeout shuts down the client and drops the owned profile (`R:crates/roder-ext-browser-use/src/server.rs:170-196`), so the browser and its logins vanish mid-task. Fix: say so in the error text (in #1). Whether to preserve the profile across a restart is a separate product call, not recommended without a recorded loss.
- **Stale refs under parallel tool calls.** Parallel tool calls default on (`R:crates/roder-core/src/runtime.rs:5752`), and the wrapper only serialises, so a queued second index click can hit a renumbered page. Fix with an element-map epoch guard in #1. Extension `chrome_*` refs are shared across calls and forgeable; #3's forgery fixture covers them.
- **Two `chrome_*` backends under one set of names** (w28). Extension and Desktop direct differ in schema (`selector/value` versus `ref/option`), observations and errors. #3 fixes select and adds a parity test; a full schema unification is not proposed.
- **Native computer batches** that discard what they computed and bail on any no-image result (#8).
- **No recovery vocabulary in Jev** (w13). Jev's operations are element controls plus DONE/BLOCKED, with no back, reload, goto or tab close, and native computer has no recovery from a wrong turn. Not recommended yet: adding operations widens the hosted chooser's menu (A/B-gated) and touches the irreversibility gate, and BLOCKED already hands off to a frontier fallback that can navigate (inferred). Revisit if a recorded run ends Blocked after a wrong navigation.
- **Popups and tabs** (w23, w22), all medium and unaddressed:
  - Desktop `chrome_*` attaches as `DirectTab::Page`, so it never follows an opened tab (`R:crates/roder-ext-chrome/src/desktop_cdp.rs:160`, `R:crates/roder-ext-chrome/src/direct/client.rs:148`).
  - Native computer drops the "opened a new tab" text.
  - Jev auto-declines confirm/prompt dialogs with no accept path, and an off-origin landing ends the run Blocked with no auto-back.
  - Jev's own popup follower (`R:crates/roder-ext-jev/src/page/tabs.rs:95`) is the model for a fix. Whether Roder Desktop (probably Electron) exposes a browser websocket for `Target.attachToTarget` is unverified.
- **Page-unresponsive recovery.** A frozen renderer shows only as a generic 30 s CDP timeout (`R:crates/roder-ext-jev/src/cdp.rs:30`, `R:crates/roder-ext-chrome/src/direct/client.rs:28`), and `timed_out` advises a larger `timeout_seconds` (`R:crates/roder-ext-jev/src/report.rs:75-78`). That Chrome tools ignore the host deadline is unchecked. Only the webwright `kill_on_drop` part is verified missing.
- **Page text and omission on Desktop** (w10). Look text is cut at 3,000 chars with no marker (`R:crates/roder-ext-chrome/src/direct/direct.js:186-187`), while elements get an `omitted` count. Fix: marker plus a paged text reader (#3, #15).
- **Fixed per-step latency floors** (w26): settle waits, a screenshot request on every `browser_use` action and an extra `get_state` round trip. Only attribution (#2) and the follow-up in #7 touch this; no direct fix is proposed.
- **Eval beds for `chrome_*` and `browser_use_*`** do not exist (w17): only scripted tests. Build one only from a recorded failure.

## What we rejected and why

**QuickE2E ideas that do not transfer:**

- **Remove DONE from Jev's menu.** QuickE2E's own bakeoff was neutral and could not show a benefit (`Q:docs/bakeoff-2026-09-24.md:506-512`). In Roder it is untested and could burn the 60-action budget. Keep DONE and the post-DONE check.
- **Re-ask after a rejected DONE.** Roder falls back to a frontier run instead. Indirect support: jev-1.13.0 never chose PRESS_ENTER or PRESS_ESCAPE in 3,225 MiniWoB episodes (`R:docs/jev-browser.md:504-509`; episodes, not times offered), so I would not expect a re-ask to find a new move.
- **Tournament heats.** They fix a 52-letter local-engine ceiling (`Q:src/loop.mjs:124-131`) that Roder does not have (Jev head accepts 255, snapshot offers at most 250, `R:crates/roder-ext-jev/src/space.rs:101-103`). They changed nothing on T3 (40/40 before and after) and cost about 2.07x per overflow page (`Q:docs/bakeoff-2026-09-24.md:416-447`).
- **Role re-ranking or link promotion** (`Q:src/loop.mjs:85-89`).
- **"(filled)" masking, and widening secret detection with QuickE2E's name regex.** The regex needed "name on card" patches; Jev already shows typed values and scrubs real secrets.
- **Hold-back-submit as specified.** Roder has no input spec to define "pending". QuickE2E's own defect (counting remaining options) hid Create forever; it regressed vanilla T2 and iframe T2 to 0/5 for that cause, and vue-ep T2 for a different one (`Q:docs/bakeoff-2026-09-24.md:286-301`). It sat in the "discover" candidate pin and was fixed in re-run 2 (`Q:docs/bakeoff-2026-09-24.md:320`). Keep only a diagnostic naming empty required fields.
- **Prune BLOCKED/WAIT/DONE wholesale while a list is open.** BLOCKED is Jev's escalation path to the fallback (`R:docs/jev-browser.md:1906`). QuickE2E keeps WAIT in an open dialog because removing it lost a server error (create-duplicate 0/3, `Q:src/loop.mjs:848-850`), and a permanent `role=menuitem` sidebar once stripped WAIT/DONE/BLOCKED on every page (`Q:fixtures/pages/menu-sidebar.html:2-4`; since fixed by requiring a popup, `Q:src/loop.mjs:852`).
- **A control-count readiness predicate.** QuickE2E itself measured it insufficient and replaced it with DOM-quiet (`Q:src/loop.mjs:441-444`). Jev uses `readyState` (`R:crates/roder-ext-jev/src/assets/ready.js`) plus a quiet clock (`R:crates/roder-ext-jev/src/assets/settle.js`) and re-reads a near-empty first look at 0.3, 0.8 and 1.5 s (`R:docs/jev-browser.md:1692-1696`).
- **Per-click load-event gate, or an unfiltered `getAnimations` wait.** QuickE2E's settle races animations against 400 ms (`Q:src/loop.mjs:462-474`). Jev chose `interactive` over `complete` when opening a tab (`R:crates/roder-ext-jev/README.md:375`); that does not cover per-click gating.
- **Per-option select rows, skip-current-option, 8-option cap in Jev.** Jev already flattens selects (cap is 255 targets, no 8-option cap found) and measured the opposite: offering the current option took choose-list from 7 to 10 of 10 (`R:docs/jev-browser.md:762-765`).
- **A flat 0.3 confidence floor.** On recorded Roder data it would have blocked 11 of 202 decisions, 8 of them in passing tasks; typing at 0.26-0.28 is correct on long forms. At most an advisory "least certain step" line: it would flag 16 of 52 tasks, 3 of the 3 failures and 13 of 49 passes (precision 3/16, n=1). Confidence signals are logged but little used (w29); #5 is the one place they are put to work.
- **Speculative decisions, local logit chooser, or any billed discard path.** The local reader scored 71/74 against hosted 74/74, was trained to QuickE2E's prompt, and is L effort for about 100 ms per decision on `jev_browse` only. The 263 vs 161 ms first-decision gap is unattributed. No hit-rate data exists.
- **Replay emitter, site-map crawler, route walker, safe-crawl hygiene.** They serve repeated tasks, not stuck or long novel runs. QuickE2E's own replay needed verified locators (3/7 and 3/9 replayed before; 9/9 after). Its map teleport cost 18/33 to 12/33. Roder has no persistent per-site store.
- **Semantic re-resolve of stale `browser_use` indices.** It couples the wrapper to pinned 0.13.10 JSON, risks approval mismatch, and "unique now" is not the element chosen. Error on a stale index instead.
- **Hard LOOP refusal or `is_error` on repeat notices for frontier surfaces.** Core stops non-interactive turns after 5 consecutive errors, and a frontier model can re-plan. Advisory only.
- **Accept confirm/prompt dialogs by default.** It reverses the deliberate "Jev never accepts" rule and is consent from untrusted page text. Name dialogs in results only. An authorized accept needs owner sign-off and a per-call scope.
- **`neverclick` lists, covered-click force rung, opaque value handles.** No logged Roder incident for any of them; each widens or tightens autonomy with real downside.
- **Free-wait step refunds.** The only evidence is one 25-vs-14-step comment.

**QuickE2E's own reverted experiments to keep in mind** (all from code comments or its bakeoff doc):
- Link promotion (create-campaign 5/5 to 0/5).
- Masking typed values (book-with-code 5/5 to 0/5).
- Counting remaining options for hold-back (three cells regressed; two share that cause).
- A permanent-sidebar open-list rule that removed WAIT/DONE/BLOCKED everywhere.
- Heal-once that emitted a byte-identical failing spec.

**Do not change the hosted Jev request** (`recent_actions`, inert marking, twin context) without an A/B variant. `choose_request.json` is a recorded contract, and hosted confidences shift between versions.

## How to prove it: an eval plan

**Method.** QuickE2E's lesson is "fails before, passes after" fixtures graded by app-side truth, not by the agent's final message (`Q:fixtures/README.md:3-44`). Roder's Jev tier already does this for POSTs and DOM probes. The work is to extend it, not to start over.

**Stage 0: instrument (keyless, no hosted service).**
- Strict native grader regrades the saved 2026-09-30 report to failed.
- Jev rows carry `verdict_ok`, `truth_ok`, `false_green`. The keyless corpus reports `false_green=0`, and a decoy page reproduces the false-done shape.
- Live tier at `JEV_EVAL_N=3` writes a pinned baseline with commit, worktree hash, corpus hash and model, read at start and end.
- At least 95% of keyless step wall time is attributed to named laps.
- `omitted_actions` and the offered set are persisted for a 300-control page.
- The 40-step growth fixture prints tokens at step 8 and step 40 for `browser_use` and chrome.
- Every #19 probe has a recorded pass or fail.

**Fixtures to add (each fails on today's code).**
- 100- and 400-element upstream-shaped `get_state`, and a killed-client case (#1).
- `toggle-menu.html`, an on-screen ticker form, and a wait-interleaved no-op page (#6).
- Twin "delete" rows with scripted confidences (#5).
- A Mac-reporting native page with the failing Ctrl+A chord batch, and a four-trap page (500, login wall, `target=_blank`, confirm) (#8).
- `primitives.html` extended with a dead, covered and forged-ref case, and a select page (#3).
- An exit-1-but-prints-`final datum:` webwright script (#10).
- `counter.html` with `text_contains` and NBSP/newline/case variants (#12).
- An optimistic-cart page whose POST returns 500 (#16).

**Metrics.**
- Per task, n>=3: `truth_ok`, `verdict_ok`, false-green rate, median steps, median input tokens. Gate on pass counts and offered-set facts, never wall time (QuickE2E measured the same task at 20.89 s and 25.29 s, on its Claude Code plus Playwright MCP arm, `Q:README.md:326-328`; Jev latency is also noisy).
- Per surface: tokens per observed step at step 1, 8 and 40; lines per element; report-visible rate; results over 200 lines per 40-step run.
- Per fix: wasted calls after the first no-op; wrong submits (native: 3 to 0); invalid-reply endings (target 0 across `delayed_spa`, `below_the_fold` and 10 booking runs); `icon_by_picture` 5/5 with mail3 kept against a baseline of 0 in every recorded live run.
- No-regression: all 52 keyless tasks unchanged and no new Looped/Unsettled stops; pass count at or above the audit's 49/52 live; MiniWoB seeds 0-4 reward not below baseline. Per-step latency within 20 ms of the same run without added instrumentation.

**The long-horizon eval Roder lacks.** There is no browser run of 30+ steps with a revisit-an-old-page task. Build this:
1. A 30-60 step ledger page (60 rows, an "Approve N" button each, a counter) served by the existing fixture site. Run it through the Jev fallback arm (`JEV_EVAL_FALLBACK=model`, `JEV_FALLBACK_MAX_STEPS=60`) with `JEV_FALLBACK_KEPT_READS` 2 against 1000.
2. A "revisit" pair where page B needs a code shown on page A after a 5-click detour, which punishes over-eliding.
3. Report pass rate, last-call input tokens, call count and wall time, n=3. Per-call prompt tokens at step 40 should be at most about 1.3-2x step 8 if pruning works.
4. For the main transcript, the keyless growth test (fake `browser_use` server plus chrome ledger page) decides whether supersession is built at all.

**Do not** build the four-surface model-in-the-loop bed (about 144 frontier runs) until a recorded `chrome_*` or `browser_use_*` wrong-target failure exists to seed its pages. QuickE2E's own warning: pages designed from our own fix list flatter us.

## Open questions and limits of this study

**Limits.** This was a read-only study. I read code and docs; I ran nothing, and no test, build or live run was executed. Numbers I give fall into three groups:
- Numbers recorded in Roder docs or saved reports (4 submits and 3 wrong, 191.5 ms median decision, 34% of wall) are quoted as found. The 49/52 live pass count is taken from the audit and was not re-derived. The 1-of-32 false done is my hand count of the saved file.
- Numbers from QuickE2E code comments are labelled as such and are anecdotal.
- Author simulations, unreproduced, no generator saved (38 of 100 indices, 15,462 to 8,255 chars, 208 lines for 30 elements, about 53k to about 5k tokens over 40 steps), are not repo measurements.

**Open questions.**
1. Is the hosted Jev service reachable for live A/B runs? It returned HTTP 402 before. #12 step 2, #17 and #18 gate on it; otherwise they stay on keyless fixtures only.
2. Should an exhausted `Error` from the Jev chooser route to the frontier fallback? That reverses the documented "never on an error" policy in `trigger.rs` and needs a product decision (#4). The new Blocked causes in #6 do not. **Decided 2026-10-09: yes; it falls back (shipped).**
3. Is the pinned browser-use 0.13.10 `get_state` schema stable, and is it comparable to the 0.12.5 checkout I read? The compactor must fail open.
4. Who owns the `roder-web-extention` merge of `pz/computer-use-primitives`, and can the extension and Rust crate release together (#3)?
5. How often do real `chrome_*` and `browser_use` traces show repeated no-op actions or wrong-twin clicks? Roder has no trace for this, and #13 and any chrome twin annotation depend on the answer.
6. Is it intended that only the Responses path delivers tool-result images? If other providers should, forwarding might beat hiding (#7).
7. Will the outer model write `success_condition` once the tool text asks for it? No corpus task sets one today. Should a code-owned end-state check (a `browser_check`-style assert) exist for `chrome_*` and `browser_use_*`? It is the direct analogue of QuickE2E's central idea, but no recorded failure seeds it.
8. Is Roder Desktop's browser Electron, and does it expose a browser websocket for target attach (popup following)?
9. What is the real user distribution by provider and by browser surface? It would reorder #1, #3, #7 and #8.
10. Owner decision on authorized dialog acceptance: does `authorize_irreversible=true` justify accepting a confirm, given the standing "Jev never accepts" rule?
11. Prompt-cache and WebSocket-prefix loss cannot be measured today. Is adding a cached-token field a prerequisite for the supersession decision (#14)?
12. Does a Jev BLOCKED benefit from QuickE2E's 3 s DOM-mutation wait before ending the run? The `late-enable.html` probe in #19 decides.
13. Delegation of a browsing sub-task (w30) and drift between Roder docs and recorded evidence were not examined beyond the weakness list; no recommendation is made.

## Appendix: technique ledger

Status is Roder's coverage of the idea. Verdict is what to do. Impact and effort are as sized in this report.

| id | title | status | verdict | impact | effort |
|---|---|---|---|---|---|
| criteria-budget-visible | Budget in readable options; make drops visible | partial | adopt narrowed (#1, #15) | medium | M |
| minimal-stateless-payload | Fresh bounded request per decision | partial (Jev done) | slice 1 now (#1); supersession measure-first (#14) | medium | M |
| native-select-per-option | One action per select option | implemented in Jev | route Desktop `chrome_select`, guard `browser_use` (#1, #3) | medium | S |
| eval-fixtures-truth | Defect-seeded fixtures, truth log | partial | narrow subset first (#2) | medium | S-M |
| eval-observability-regression | Record laps, offered set, pin baseline | partial | Jev first (#2) | medium | M |
| loop-detect | Unchanged-page signature plus run-wide pairs | partial | Jev counter (#6); frontier advisory gated (#13) | medium | M |
| early-dead-end-stops | Stop on deterministic dead ends | partial | Phase 1 only (#16) | medium | M |
| code-judges-goal | Code checks goal every snapshot | partial | normalise, then opt-in early end (#12) | medium-low | S-M |
| plan-once-roles | Plan once, pick per step, judge in code | partial | inputs and judge parts only (#12, #18) | medium | M |
| identical-control-context | Disambiguate twins by card line | Jev done; chrome absent | gate on a real chrome failure; Jev tie-break optional | low-medium | M |
| history-agrees-page | History is a view over page state | partial | re-key repeat guard (#5); annotate behind variant | low | S |
| name-the-cover | Name the cover on a covered click | partial | text-only legs now (#17) | low | S |
| text-field-menu-rules | Bind values strictly, drop dead moves | partial | refused-fill reason only, later | low | S |
| caller-text-key-choice | Key choice, text from caller inputs | partial | optional `inputs` (#18) | low | S |
| values-map-state | Form state in a separate map | partial | digest checked/unchecked fix only (#15) | low | S |
| echo-spec-values | Show typed text only when task value | Jev implemented | pin test only (#18) | low | S |
| goal-anchored-keep | Never truncate a goal-named candidate | partial | telemetry first (#15) | low | S |
| open-list-prunes-escape-hatches | Offer only options while a list is open | partial | chrome top-layer flag optional; no wholesale pruning | low | S |
| blocked-discipline | Give-up believed only on a stable page | partial | probe first (#19) | low | S |
| link-click-url-wait | Wait for URL change after link click | partial | probe first (#19) | low | S |
| settle-dom-quiet | Snapshot only after DOM quiet | partial | ship extension branch (#3); load gate probe | low | S |
| hold-back-submit | Hide submit while required field unset | absent | reject; diagnostic only (#19) | low | M |
| covered-click-ladder | Click through by-design covers | partial | reject until reproduced | low | S |
| escape-retry-recovery | Escape retry, dialogs | partial | modal guard and authorized accept need owner call | low | M |
| confidence-gate | Gate low-confidence picks by side effect | partial | advisory line; validate existing gate live | low | S |
| neverclick | Forbidden actions removed from menu | partial | reject now | low | M |
| unique-match-reresolve | Re-resolve stale targets on unique match | partial | extension unique-or-fail (#3); no `browser_use` re-resolve | low | M |
| free-waits | WAIT first-class, first 10 free | partial | reject refund; unproven | low | S |
| deadlines | Every blocking wait has a deadline | partial | webwright `kill_on_drop` (#10); rest hygiene | low | S |
| speculative-latency-hiding | Speculative decision, warm connection | partial | shadow measure and pre-connect only | low | M |
| perceived-text-oracle | Judge on perceived text and state | partial | typed-value exclusion (#12) | low | S-M |
| transient-text-recorder | Record text visible at any moment | absent | probe first (#19) | low | S |
| start-state-guards | Prove goal false at start | partial | informational flag (#12) | low | S |
| failure-evidence-taxonomy | Page-sourced causes, fault class | partial | handoff vs fault (#9); small page-error line later | low | S |
| name-and-mark-choices | Name controls from evidence, mark state | partial | narrow icon-only naming later | low | S |
| inpage-dom-snapshot | One in-page DOM walk | Jev implemented | chrome look regression test (#19) | low | M |
| click-targets-aria-hides | Offer what a user would click | partial | probe first (#19) | low | S |
| logit-chooser-small-model | Choose by option-label logits | partial | reject | low | S/L |
| affordance-synthesis | Synthesize missing affordances | partial | slider only if probe fails | low | M |
| map-persist-route | Crawl once into a site map | absent | reject | low | S |
| map-safe-crawl-hygiene | Explore safely, never logout | partial | reject; validate irreversible gate live first | low | S |
| replay-distill | Distill a pass into a replay | partial | webwright hygiene only (#10) | low | S |
| replay-verify-locators | Verify each locator live | partial | drop | low | S |
| replay-or-heal | Replay first, heal on drift | partial | cheap webwright legs (#10) | low | S |
| opaque-value-handles | Opaque value handles for frontier tools | partial | defer; needs provisioning and per-origin binding | low | M |
| role-ranking-reverted | Static role ranking | partial | not transferable | low | n/a |
| ready-predicate | Control-count readiness predicate | n/a | refuted by QuickE2E's own measurement; Jev has readyState plus quiet clock | n/a | n/a |
| tournament-heats | Split oversized menu into heats | n/a | refuted | n/a | n/a |

Pain-point ids map to the ranked items as follows:

| Pain-point id | Topic | Ranked item(s) |
|---|---|---|
| w01 | repeated no-op actions on frontier surfaces | #13 |
| w02 | Jev loops and stale rereads | #6 |
| w03 | failure reasons not reaching the chooser | #17 |
| w04 | success condition matching | #12 |
| w05 | wrong destructive twin | #5 |
| w06 | `browser_use` result size and order | #1 |
| w07 | native computer facts dropped | #8 |
| w08 | transcript growth | #14 |
| w09 | extension click blindness | #3 |
| w10 | Desktop text omission | #3, #15 |
| w11 | `browser_use` silent select and stale-index clicks | #1 |
| w12 | unusable decision replies | #4 |
| w13 | no recovery vocabulary in Jev | pain points section (deferred, with reason) |
| w14 | native computer screenshot loss | #8 |
| w15 | `browser_use_agent` two-browser observation | #1 |
| w16 | webwright failure reporting | #10 |
| w17 | eval beds | #2 |
| w18 | Jev budget burn | #6 |
| w19 | Jev stale rereads | #6 |
| w20 | surface routing | #11 |
| w21 | handoffs counted as failures | #9 |
| w22 | dialogs | pain points section; owner decision on dialog accept |
| w23 | popups and tabs | pain points section |
| w24 | phantom screenshots | #7 |
| w25 | `browser_use` lifecycle and profile loss | #1, pain points section |
| w26 | fixed per-step latency floors | #2, #7, pain points section |
| w28 | two `chrome_*` backends | #3, pain points section |
| w29 | confidence signals logged but unused | #5; rejected (confidence floor) |
| w30 | delegation of browsing sub-tasks, docs drift | Open question 13 (not examined) |

## Implementation status (2026-10-09)

This part records the first implementation round against the recommendations above. The second round and a final fix pass over it are in "Implementation status, part 2" below, which also holds the verification table and the "Deliberately not done" and "Needs attention" lists for both rounds. The work sits in the worktree on branch `pz/browser-use-agent-improvements-779594`, which was at master `ef5a4b44` when it started. The first round covered twelve work items (the table below). A report item that has no row in either part is not covered here, and no status is claimed for it. Where a row says an item is partial, the parts not done are listed under "Deliberately not done" in part 2.

Each item was built test-first: a test or fixture that failed on the old code, then the change, then the crate's whole test set and `fmt`. In the Files column, entries are written `crate: path`, with paths relative to `crates/<crate>/`; paths that start with `docs/` are relative to the repository root. "New" marks files created by this work.

### First round (twelve items)

| report item | what shipped | files | tests | status |
|---|---|---|---|---|
| #1, part a (`bu-1a`) | `browser_use`: the action report now comes first and is labelled as browser-use's own claim, and the observed state follows it. `browser_use_agent` no longer gets an observation of a browser it does not drive. Dead clicks and types set `is_error` and still carry a fresh observation: this matches the eight fixed error strings of the pinned server, plus the `Agent task failed: ` prefix for the agent tool only. A killed or timed-out browser says it is gone, and the next call starts fresh with a one-shot "ran in a fresh browser" notice. Secrets are redacted before the cut, and the truncation marker gives dropped and total bytes. | `roder-ext-browser-use`: `src/{lib,server,tools,catalog}.rs`; new `src/observation.rs`, `src/failed_action.rs`, `src/browser_loss.rs`; `tests/fake_server.rs`, `tests/support/live_fixture.rs`. `docs/roder-browser-use-provider.md` | `fake_server`: 5 new (`the_action_report_comes_first_and_survives_a_long_state`, `the_agent_tool_returns_its_own_report_without_a_foreign_observation`, `dead_clicks_and_types_are_errors_that_still_carry_a_fresh_observation`, `a_killed_client_says_the_browser_is_gone_and_the_next_call_starts_fresh`, `a_timed_out_call_says_the_browser_is_gone`) and the cancellation test extended. Unit tests in `observation`, `failed_action`, `browser_loss`, `tools` and `catalog`. The ignored live test `live_profiles_observations_and_operator_domain_scope` was extended for a stale index against the real pinned server; it was not run. | shipped |
| #1, part b (`bu-1b`) | `browser_use_get_state` is a compact view: a header (page, viewport, scroll, tabs capped at 8), then one line per element, with the omitted count shown and an exact continuation line. `offset` is a wrapper-side argument that is checked and removed before the call reaches the server. Budgets are 18,000 chars, 150 lines and 20,000 bytes, so the legacy 24,000-byte cut is never reached. Secrets are redacted before any field is shortened. Page text cannot add lines or break out of its quotes. A state that is not the pinned shape passes through unchanged. | `roder-ext-browser-use`: `src/{lib,server,tools,catalog}.rs`; new `src/state_view.rs`, `src/state_view_tests.rs`, `tests/support/compact_state.rs`; `tests/fake_server.rs`, `tests/support/live_fixture.rs`. `docs/roder-browser-use-provider.md` | `fake_server`: `a_100_element_state_is_one_line_per_element_and_every_index_appears_once`, `a_400_element_state_keeps_the_report_and_counts_what_it_leaves_out`, `offset_is_handled_by_the_wrapper_and_checked`, `a_state_that_is_not_the_pinned_shape_passes_through_unchanged`, `secrets_are_redacted_before_the_view_is_cut`. `catalog::get_state_takes_an_offset_that_only_roder_reads`. `state_view_tests`: header and line format, quoting, shortening, paging, omission line, offset, redaction, and `the_real_pinned_servers_state_is_compacted`. | shipped |
| #1, part c (`bu-1c`) | A click or type on a native `<select>` that the thread was last shown is refused before the server sees it. The refusal names routes that work: `browser_use_agent` as a whole-task route in its own temporary browser (offered only when a key is configured), and `jev_browse` and `chrome_select` as available if enabled but driving a different browser. Typing is refused as well, because typing into a select reports success either way and the state cannot confirm it. The per-thread record is the set of select indexes from the last state shown, and it is dropped with a lost browser. | `roder-ext-browser-use`: `src/{lib,server,tools,catalog,state_view}.rs`; new `src/select_guard.rs`; `tests/fake_server.rs`, `tests/support/{compact_state,live_fixture}.rs`. `docs/roder-browser-use-provider.md` | `fake_server`: 8 tests, among them `a_click_on_a_native_select_is_refused_and_the_server_never_sees_it`, `typing_into_a_native_select_is_refused_and_the_server_never_sees_it`, `the_refusal_offers_the_agent_only_when_a_key_is_configured`, `a_select_is_only_known_in_the_thread_that_was_shown_it`, and two regression guards that passed before. `select_guard` unit tests (7). `catalog::click_and_type_say_a_native_select_is_not_theirs_to_operate`. The ignored live test was extended; not run. | shipped |
| #2, part a (`ch-2a`) | The native-computer example grader (`submitted_value` check) now requires exactly one trusted correct submit. A correct submit after a wrong one, before a wrong one, or repeated all fail. Untrusted submit events are ignored. The grade JSON gains a `trusted_submits` count. The saved live OpenAI report with three wrong submits now fails; the saved protocol and visible-Chrome reports still pass. `docs/native-computer-use.md` and `docs/browser-computer-use-audit.md` said the live run "passed the independent submission grader", which held only under the lenient rule; both are corrected. | `roder-ext-chrome`: `examples/native_computer/support.rs`; new `tests/native_grader.rs`. `docs/native-computer-use.md`, `docs/browser-computer-use-audit.md` | `tests/native_grader.rs` (12), including `saved_live_report_with_three_wrong_submits_fails_the_strict_grader`, `saved_runs_with_one_correct_submit_still_pass_the_strict_grader`, `correct_submit_after_a_wrong_one_fails` and `repeating_the_correct_submit_fails`. | shipped |
| #8 (`ch-8`) | The native `computer` tool returns the facts and never loses the screenshot. A capped Notes block names a new tab, a dialog, a sign-in wall, an HTTP error status, and a click that hit nothing pressable and changed nothing; page words in it are labelled untrusted. When a screenshot cannot be taken or the browser cannot be reached, a small valid placeholder PNG replaces it, and `ComputerTool::execute` no longer returns an executor error. Ctrl+A/C/V/X/Z, and also Ctrl+Y and Ctrl+Shift+Z, are sent as Command on Mac pages. A batch stops after a navigation and names the actions it did not run. The failure line counts the failing action 1-based ("action K of N failed"). Dialog text is scrubbed before the 500-char cut. | `roder-ext-chrome`: `src/computer.rs`, `src/direct/{act,capture,client,computer,keys,mod,session}.rs`, `examples/native_computer/support.rs`; new `src/direct/{computer_notes,computer_notes_tests,mac,unavailable}.rs`, `tests/native_notes.rs`. `docs/native-computer-use.md` | `tests/native_notes.rs` (7): Ctrl+A on Mac and non-Mac pages, each trap named, batch stop after a navigation, placeholder screenshot, unreachable browser, uneventful batch. Unit tests in `direct::keys`, `direct::computer_notes`, `direct::unavailable` and `direct::client`. | shipped |
| #3, desktop select (`ch-3a`) | Desktop `chrome_select` is routed to the shared direct select and takes exactly one of `ref` or `selector`, plus `value`. A miss lists the options with values, capped at 20 with "and N more". A disabled option and ambiguous text are named and not chosen, and a non-select is refused. A page that restores the old value is reported with `is_error` and `data.restored`. Matching order is exact value, then visible text ignoring case and spacing, then value ignoring case, then the one option whose text contains it. The extension still receives only selector and value, and a `ref` is refused before it reaches the extension. Desktop `chrome_page_text` and `chrome_highlight` now say they are not supported on the Roder Desktop browser. The `direct.js` helper version went from 2 to 3. | `roder-ext-chrome`: `src/{desktop_cdp,tools,lib}.rs`, `src/direct/{act,mod,session,tools}.rs`, `src/direct/direct.js`, `tests/computer_use.rs`; new `src/chrome_select.rs`, `src/direct/select.rs`, `tests/fixtures/select.html`, `tests/support/select.rs`. `docs/roder-chrome-browser-extension.md` | A ten-case `desktop_select` run, called from the existing headless-Chrome test `evaluates_real_input_observations_and_hidpi_coordinates`. `chrome_select::tests` (5, including that nothing is queued for the extension on a `ref`). `direct::select::tests` (3). | shipped |
| #3, compact results (`ch-3b`) | Extension and Desktop action results are compact and outcome-first. Line 1 is the untrusted label and line 2 is the Outcome: "No visible change.", a new address as old to new, or changed and new controls counted and marked `[new]`. Snapshots list controls first, then page text, and the omitted count is shown. For old extension builds, one fallback snapshot is taken 150 ms after click, type, keypress, scroll and select; such a build is detected by result shape because the extension repo is read-only. `boxes` is dropped from the model-facing snapshot schema. Navigate and snapshot render compactly with no Outcome line. Results without a page stay labelled JSON. | `roder-ext-chrome`: `src/{lib,tools,session,desktop_cdp}.rs`, `src/direct/{direct.js,look.rs}`, `tests/computer_use.rs`; new `src/{observed,observed_tests,observed_render,observed_read,extension_result,desktop_outcome}.rs`, `tests/extension_observation.rs`, `tests/support/{scripted,outcome}.rs`, `tests/fixtures/outcome.html`. `docs/roder-chrome-browser-extension.md` | `tests/extension_observation.rs` (21): unchanged page, new address, counted controls, old-build fallback, partial snapshots, tab naming. `observed_tests` unit tests. `direct::look` text-cut marker. Desktop versus scripted-extension parity in `desktop_and_extension_say_the_same`. | shipped |
| #17, chrome half (`ch-17`) | The covered-click advice now fits the tool that gets it. `chrome_click`, `chrome_type`, `chrome_scroll` and direct `type` have no x/y, so they are told what they can do instead. Direct `click` keeps "press at x/y". The cover name is scrubbed, kept to one line, unquoted and capped. Tool schemas were already correct and are unchanged. | `roder-ext-chrome`: `src/direct/{target,act,session}.rs`, `src/desktop_cdp.rs`, `tests/computer_use.rs`; new `tests/fixtures/covered.html`, `tests/support/covered.rs` | `direct::target::tests` (5). Step `covered_targets_get_advice_their_tool_can_follow` in `tests/computer_use.rs` against headless Chrome: nothing is pressed or typed, and the schemas have no x/y. | partial: the Jev cover-name half shipped in the second round (`jev-17`); the chooser half is not done |
| #10 (`ww-10`) | A Webwright script run explains its outcome. The class is one of ok, nonzero_exit, signaled, timeout or launch_failed, followed by a hint label, the first error line, and a capped stderr tail (stdout tail when stderr is empty). A clean run is one line. A launch failure is a classified result rather than a tool error. The child is killed on drop and on timeout, and a timeout keeps the partial output. The exit status is stored in a new `run_exit.json`, written as "running" first. Verification fails on a nonzero exit, a timeout, or a run that never finished. Preparing a workspace again keeps the existing manifest. | `roder-ext-webwright`: `src/{lib,tools,verify,workspace,tools_tests,workspace_tests}.rs`, `src/tools/support.rs`; new `src/run_outcome.rs`, `src/run_outcome_tests.rs`, `src/run_script_tests.rs`, `src/tools/exec.rs`. `roder-skills`: `builtin/webwright/SKILL.md`. `docs/roder-webwright-browser-agent.md` | `run_script_tests` (failed-run text, one-line success, tail caps, launch failure, timeout keeps partial output, `dropping_the_run_kills_the_child`, background children holding the pipes, verification gates). `run_outcome_tests` (class from exit status, hints, first-error rules, tails, exit-record round trip, verification). `workspace_tests::create_keeps_an_existing_manifest`. | shipped |
| #7 (`core-7`) | Providers that never receive tool-result images are no longer shown screenshots. The new default method `InferenceEngine::tool_result_image_input(&self, model: &str)` returns false. For such an engine, core strips the image key from tool results and adds one notice line per result, and it hides the image-only tools `chrome_screenshot`, `browser_use_screenshot`, `jev_tab_screenshot` and `view_image`. Prompt accounting charges image tokens only for engines that receive them. The persisted transcript keeps the image and never the notice, so switching engines mid-thread replays the same history. The Responses engine reports true exactly when its replay emits `input_image`. Anthropic, Gemini, SuperGrok (per model, from the catalog) and the Codex OAuth wrapper each got a one-method override. | `roder-api`: `src/inference.rs`. `roder-core`: `src/{lib,prompt_accounting,compaction,compaction_runtime,transcript,transcript_compaction,provider_compaction,runtime,tool_advertisement}.rs`; new `src/tool_result_images.rs`, `tests/tool_result_images.rs`. `roder-ext-openai-responses`: `src/provider.rs`, `src/image_replay_tests.rs`; new `src/image_support.rs`. Overrides in `roder-ext-anthropic/src/provider.rs`, `roder-ext-gemini/src/provider.rs`, `roder-ext-xai/src/provider.rs`, `roder-extension-host/src/codex_oauth.rs`. `docs/native-computer-use.md`, `docs/roder-browser-use-provider.md` | `roder-api`: `tool_result_images_are_off_unless_the_engine_opts_in_per_model`. `roder-core` `tool_result_images` unit tests, plus `tests/tool_result_images.rs` (6, including a 30-step fake browser run and `switching_engines_mid_thread_replays_the_same_history`, which checks the accounting delta). One test each in openai-responses, anthropic, gemini, xai and extension-host. | shipped (see Needs attention in part 2); the Jev fallback's own screenshot tool followed this gate in part 2 (`jev-img`) |
| #8, replay (`core-8`) | The Responses replay passes native-computer notes through on success too. They go out as a separate labelled user message after the screenshot, the same shape as the existing error sidecar. They are capped in count, length and total, cleaned, never sent twice with the failure sidecar, and also sent on a websocket continuation. | `roder-ext-openai-responses`: `src/{computer,response_replay,computer_tests}.rs`. `docs/native-computer-use.md` | `computer_tests`: `native_success_notes_replay_as_a_labelled_user_message_after_the_screenshot`, `native_success_without_notes_adds_nothing_and_keeps_the_prefix`, `native_notes_are_capped_in_count_length_and_total_and_cleaned`, `native_notes_reach_the_replay_through_the_real_result_conversion`, `native_websocket_continuation_sends_the_notes_beside_the_new_screenshot_only`, and two more. | shipped; reworked in part 2 (`cl-notes`): the notes are read from structured data in `roder-api` and not parsed out of the result text, and these notes tests moved to `computer_notes_tests.rs` |
| #11 (`core-11`) | A routing block of 108 words is appended to the developer text once. It appears only when more than one of the four browser tool families is advertised, lists only the advertised families, and is a pure function of the advertised tool names. It never promises that a surface works, and it tells the model not to use another family to get around a site's refusal of automated access. Jev's `blocked` and `access_denied` hints, and both Jev missing-key errors, name the other families without promising them, through one shared `OTHER_BROWSERS` constant. | `roder-core`: `src/{lib,runtime}.rs`; new `src/browser_routing.rs`, `tests/browser_routing.rs`. `roder-ext-jev`: `src/report.rs`, `src/runner/decision_backend.rs`; new `src/report/hint_tests.rs`. `docs/roder-browser-use-provider.md` | `browser_routing::tests` (7, including all 16 subsets of the four families and a word count between 90 and 149). `tests/browser_routing.rs` (3): two families get the block, one family gets none, and the block is identical on every turn. `report::hint_tests` (2). `decision_backend::tests::missing_key_errors_name_other_browser_families_without_promising_them`. | shipped |

## Implementation status, part 2 (2026-10-09)

This part records the second round and a final fix pass over it: the Jev items, the structured form of the native-computer notes, and the review follow-ups for `browser_use`, Chrome and Webwright. The "Verification by crate", "Deliberately not done" and "Needs attention" sections below are cumulative: they cover the first round as well. Jev work in this round was kept off the hosted service: no `JEV_API_KEY` or other key was used, and the tests that need one are the ignored ones.

### Second round items

Same method as the first round, and the same Files convention: `crate: path`, paths relative to `crates/<crate>/`, "New" for files created by this work. Every new Jev path below is under `roder-ext-jev`.

| report item | what shipped | files | tests | status |
|---|---|---|---|---|
| #4 (`jev-4`) | Jev asks a decision again, up to twice, when the service answered and was billed but the reply cannot be used: an action the page never offered, probabilities that do not add up, a missing answer or a refusal. The third unusable reply in a row ends the run `error` with the new `JevStopCause::DecisionUnusable`, and `stopped_because` gives the reply count and the first reason on one line. The owner decided on 2026-10-09 that this error falls back to the frontier model, with the new trigger `decision_unusable`. A usable reply starts the count again. Every unusable reply counts in `usage`, in `model_calls` and against the 120-call budget. A call that failed instead of answering (a refused key, an unreachable provider) is not asked again. A body that cannot be decoded is: after the HTTP layer's one resend it counts as an unusable reply, with unknown usage. The OpenAI Decisions client counts an unreadable reply the same way. `JevBilled::unusable` lets a decision client mark a reply. `trigger()` maps an `error` with `decision_unusable` to the trigger `decision_unusable`; every other `error` still has no trigger. | `roder-ext-jev`: `src/{agent,decide,decisions,engine,usage,lib}.rs`, `src/engine/records.rs`, `src/fallback/trigger.rs`, `src/decisions/tests.rs`, `tests/agent_limits.rs`; new `src/agent/unusable.rs`, `src/decide/unusable_tests.rs`. `README.md`, `docs/jev-browser.md` | `tests/agent_limits.rs`: `an_unusable_reply_is_asked_again`, `a_usable_reply_starts_the_unusable_count_again`, `the_stop_names_the_first_reason_on_one_line`, `a_decision_that_fails_otherwise_is_not_asked_again`, and `billed_calls_with_unusable_answers_count_in_the_usage` updated. `decide::unusable_tests` (4). The `fallback::trigger` tests assert that an `error` with `decision_unusable` maps to the trigger and that every other `error` has none. The third round added `decide::body_loop_tests` (3, the real hosted transport against a local mock server), `tests/agent_undecodable.rs` (2, the loop's accounting through the public API; they also pass on the old code) and `fixture_harness::decision_fallback_tests` (3), the only ones that run the fallback from an `error` base on a real page. Three existing tests were changed on purpose: `billed_calls_with_unusable_answers_count_in_the_usage` (an always-invalid reply now ends after 3 calls), `fallback::trigger::tests::statuses_that_never_fall_back` and `decisions::tests::malformed_responses_fail_closed_and_keep_billed_usage`. | shipped, including fallback routing (owner decision, 2026-10-09) |
| #5 (`jev-5`) | An unsure click on a twin control ends the run `done` without clicking. It fires when the label equals that of the page-changing click immediately before (a click on any other control in between turns the guard off), the control differs, the label passes `names_commitment` (delete, buy, send and the like), and the call confidence is below `REPEAT_CONFIDENCE` (0.7). A twin whose label commits nothing ("Open", "Next") is clicked as before. The result gains `suppressed_click` (`JevSuppressedClick`, `JevSuppressedKind`: label, row or section, the earlier click's row, confidence) and the result text a "Not clicked:" line, scrubbed, on one line and cut. The older same-control rule reports what it held back too. The keyless corpus gains `icon_by_picture_twin` (the recorded failure, with mail3 staying). | `roder-ext-jev`: `src/{agent,engine,lib,report}.rs`, `src/agent/{predict,result}.rs`, `src/engine/records.rs`, `src/report/digest/*`, `src/fixture_harness/evals/scripted.rs` (the keyless plan step gains an optional `confidence`), `tests/fixtures/evals/tasks.json`; new `src/agent/repeat.rs`, `src/report/not_clicked_tests.rs`, `tests/agent_repeat.rs`. `README.md`, `docs/jev-browser.md` | `tests/agent_repeat.rs` (9), among them `an_unsure_delete_on_a_twin_row_ends_done_without_clicking`, `a_confident_delete_on_a_twin_row_clicks_it`, `a_delete_on_a_twin_row_at_the_repeat_confidence_clicks_it`, `an_unsure_twin_whose_label_commits_nothing_clicks_as_before` and `what_is_reported_of_the_click_is_scrubbed_and_cut`. `report::not_clicked_tests` (4). Keyless corpus: 53 tasks. | shipped; 0.7 is the existing threshold and was not re-measured |
| #6, parts a to c (`jev-6`) | Three run-wide caps, each ending the run `blocked`, so the fallback takes over as it does after a stall. (a) The fourth time Jev would choose the same control on a page with the same fingerprint, the run ends `looped` before acting; the fingerprint is not masked, so a counter that rises with every click is progress. (b) Three stale decisions in a row, each followed by exactly the same view of the page, end the run `unsettled`; a recorded step or a different page starts the count again. The owner decided on 2026-10-09 that nothing is masked here: the compare covers url, scroll, text, the visible controls' id, kind, checked, selected, expanded, scrolled, label, value and current value, and frames, as the observation reports them, so a page whose only change is digits (a clock, a countdown) is progress and is left to the action and model-call budgets. (c) Six waits in a row that changed nothing end it `looped`, and an unchanged wait no longer hides a stall. New public `JevStopCause::Looped` and `JevStopCause::Unsettled`. The page's own fingerprint and `page_changed` are untouched. A toggle-menu run ends after 6 actions and 7 decisions instead of 60 actions. | `roder-ext-jev`: `src/{agent,engine}.rs`, `src/fallback/trigger.rs`, `tests/agent_loop.rs`; new `src/agent/loops.rs`, `src/fixture_harness/loop_tests.rs`, `tests/agent_loop_caps.rs`, `tests/fixtures/pages/{toggle-menu,ticker-form,clock-form}.html`. `README.md`, `docs/jev-browser.md` | `tests/agent_loop_caps.rs` (13): `an_open_close_cycle_ends_looped`, `the_stall_rule_still_fires_before_the_pair_counter`, `pages_that_only_change_in_their_numbers_are_progress_to_the_pair_counter`, `six_unchanged_waits_end_looped`, `a_page_whose_view_never_changes_ends_unsettled_within_three_decisions` (renamed from `a_page_that_never_holds_still_ends_unsettled_within_three_decisions`), `a_page_that_changes_only_in_its_digits_is_left_to_the_budgets`, `a_label_that_changes_only_in_its_digits_is_left_to_the_budgets`, `a_stale_decision_that_leaves_a_new_page_is_not_counted`. `loop_tests` (6) on real pages, including `a_clock_whose_digits_are_on_the_page_is_progress_to_the_stale_cap` and `a_menu_that_looped_falls_back_like_a_stall_does`. `agent::loops` unit tests (5). The keyless corpus still passes and now asserts that no task ends `looped` or `unsettled`. `tests/agent_loop.rs::an_always_missing_choice_ends_unsettled_after_three_tries` is the old `an_always_missing_choice_loops_until_the_model_call_budget` converted in place: it expected 120 calls and `BudgetExceeded`, which was the rough edge this item removes. | partial: (a) to (c) shipped, (d) not done; the limits 4, 3 and 6 are unmeasured |
| #2, Jev parts b to e (`jev-2`) | (b) `Row.pass` is split into `verdict_ok`, `truth_ok` and `false_green`, and the table tells a false green from a miss. (c) `JEV_EVAL_N` (default 1, at most 20) repeats each task, with a per-task tally and a saved baseline (`JEV_EVAL_SAVE_BASELINE`, `JEV_EVAL_BASELINE`). A pin records the commit, a hash of the sources and fixtures, a hash of the corpus, the model and the switches, and is read at the start and the end of a run, so a mid-run edit is flagged and no baseline is saved from it. A baseline is refused below three runs per task and for a narrowed corpus. `JEV_EVAL_STRICT` against a baseline fails on a regression or a new false green and not on known failures. (d) Rows carry `omitted_actions`, each settle's `{reason, waited_ms}` and per-phase laps. (e) `search-decoy.html` reproduces the recorded `enter_to_search` false DONE: status `done`, no posts. All of it is test-only code under `fixture_harness`; the one production edit is that `Page::settle` returns the page's settle reply, with no change in behaviour. | `roder-ext-jev`: `src/fixture_harness/{mod,evals/mod}.rs`, `src/fixture_harness/evals/{rows,grade,live,probe,keyless,scripted,decisions_comparison,decisions_live,complex_contracts,secret_tests}.rs`, `src/page.rs`; new `src/fixture_harness/evals/{tally,pin,watch}.rs` and their tests `tally_tests.rs`, `watch_tests.rs`, `split_tests.rs`, `tests/fixtures/pages/search-decoy.html`. `README.md`, `docs/jev-browser.md` | `split_tests` (9), among them `a_done_with_a_failing_dom_probe_is_a_false_green` and `the_decoy_page_gives_a_false_done_the_split_catches`. `tally_tests` (20), among them `a_mid_run_edit_is_flagged_and_no_baseline_is_saved_from_it` and `strict_against_a_baseline_fails_on_a_task_getting_worse_not_on_known_failures`. `evals::pin` unit tests (5). `watch_tests` (2), including the 300-control page. | partial: no baseline file has been produced and the live tier was never run (see below) |
| #12, step 1 (`jev-12`) | `jev_browse` `success_condition` gains `text_absent`. `url_contains`, `text_contains` and `text_absent` compare ignoring case, whitespace runs, no-break spaces and zero-width characters, so "Count: 1" matches "Count:\n1". A line that only echoes a typed field value no longer counts as page text, and the observation lists the field values its text holds. A failed check names the unmet predicates ("condition met / not met"), and a failed DONE's `stopped_because` ends with `not met: <names>`. A predicate with nothing visible in it (only whitespace or zero-width characters) is now refused as a parse error; the empty string still means skip. | `roder-ext-jev`: `src/session/completion.rs`, `src/assets/{text,snapshot}.js`, `src/runner/look.rs`, `src/tools.rs`, `src/secret.rs`, `src/fallback/{mod,tests}.rs`, `src/report/digest/header.rs`, `src/fixture_harness/mod.rs`; new `src/session/completion/text.rs`, `src/fixture_harness/completion_match_tests.rs`, `tests/fixtures/pages/completion-state.html`. `README.md`, `docs/jev-browser.md` | `session::completion::text` unit tests (8). `completion_match_tests` (7): `text_contains_matches_across_nodes_whatever_the_case`, `a_condition_true_at_load_reports_as_it_always_did`, `no_break_and_zero_width_characters_do_not_hide_a_match`, `text_absent_needs_the_text_gone`, `all_predicates_must_hold_and_the_failing_one_is_named`, `a_typed_value_is_not_page_text`. | partial: step 1 shipped, step 2 not done |
| #15 (`jev-15`) | A result says what the page held that Jev was not offered: a counts-only "Not offered to Jev" line in the digest header and an `omitted` field (`JevOmitted`: `controls` the snapshot left out, counted per control and not per action, and `options` past the 255 a choice takes, such as a 300-option select or a second long select). The count is the final page's. Native checkboxes, radios and switches report `checked` true or false (`JevControl.checked`) and read `[checked]` or `[unchecked]` in the options list, not the HTML value "on". The hosted chooser request is unchanged. | `roder-ext-jev`: `src/{space,page,engine}.rs`, `src/assets/snapshot.js`, `src/agent/result.rs`, `src/engine/records.rs`, `src/report/digest/{header,options,page}.rs`; new `src/report/{omission_tests,toggle_tests}.rs`, `src/fixture_harness/{omission_tests,toggle_tests}.rs`, `tests/agent_omitted.rs`, `tests/fixtures/pages/past-the-caps.html`. `README.md`, `docs/jev-browser.md` | `tests/agent_omitted.rs` (4), among them `a_300_option_select_reports_the_options_no_choice_could_take`. `report::omission_tests` (5), `report::toggle_tests` (3), `fixture_harness::omission_tests` (4), `fixture_harness::toggle_tests` (2). | shipped |
| #17, Jev cover-name half (`jev-17`) | A covered step names what covered it. `act.js` reports the element that took the click (its label, title or alt, a field's placeholder, its text, else its tag, never a field's value), `Covered::with_cover` and `Covered::cover` carry it, and the loop records it as `JevActionRecord.covered_by`. The name is page text: scrubbed of typed secrets, on one line, without control characters or double quotes, cut to 100 characters. The digest shows it among the page-supplied lines, and the fallback's opening message lists it in Jev's last steps, labelled untrusted. The hosted chooser request is unchanged. | `roder-ext-jev`: `src/assets/act.js`, `src/page/act.rs`, `src/engine/records.rs`, `src/report/digest/*`, `src/fallback/prompt.rs`, `tests/fixtures/pages/covered.html`; new `src/engine/covered.rs`, `src/agent/cover.rs`, `src/report/cover_tests.rs`, `src/fixture_harness/cover_tests.rs`, `tests/agent_cover.rs`. `README.md`, `docs/jev-browser.md` | `tests/agent_cover.rs` (4), `fixture_harness::cover_tests` (4), `report::cover_tests` (3), `agent::cover` (2). In `fallback::prompt`: `a_covered_step_names_its_cover_and_the_list_says_the_name_is_untrusted` and `a_cover_name_that_speaks_to_the_fallback_is_held_to_one_short_line`. `decide::tests::what_covered_a_step_is_not_in_the_chooser_request`. | partial: the chooser half is not done |
| #9, Jev half (`jev-9`) | A first `needs_input`, `needs_confirmation` or `access_denied` result from `jev_browse` is no longer a failed tool call: `is_error` is false and `data.outcome_class` is `handoff`, so legitimate handoffs no longer count toward core's five-in-a-row stop. The same handoff coming back unchanged (same status, same page, same reason, whitespace aside) is an error again, with `outcome_class` `repeated_handoff`. What a handoff said is remembered per session, so per thread. A handoff the fallback ends in is marked too. Every other status keeps its meaning and gets no class, and the text the caller reads is the same. | `roder-ext-jev`: `src/{tools,lib}.rs`, `src/session/{state,call}.rs`; new `src/handoff.rs`, `src/handoff/tests.rs`, `src/fixture_harness/outcome_class_tests.rs`. `README.md`, `docs/jev-browser.md` | `handoff::tests` (10), among them `five_different_handoffs_in_a_row_are_not_a_run_of_errors` and `an_identical_handoff_is_a_repeat_and_an_error_for_as_long_as_it_repeats`. `outcome_class_tests` (5) on real sessions, including `another_threads_handoff_is_not_a_repeat` and `blocked_and_the_rest_stay_failed_tool_calls`. | partial: the approval half is not done |
| #7, Jev follow-up (`jev-img`) | The Jev fallback no longer decides whether to offer its screenshot tool from `capabilities().image_input`, which is about images a user attaches. `FallbackModel::sees_tool_result_images` asks `InferenceEngine::tool_result_image_input(model)` for the fallback's model. The screenshot tool is offered (`fallback/offered.rs`) only when that is true, and a model without it is not told of one: neither its instructions nor its opening message mention a screenshot. | `roder-ext-jev`: `src/fallback/{model,offered,prompt,run,mod}.rs`, `src/fixture_harness/{fallback_tests,fallback_script}.rs`, `src/fixture_harness/evals/fallback_live.rs`. `docs/jev-browser.md` | `fallback::model` (4), among them `pictures_follow_what_the_engine_forwards_in_tool_results_not_what_it_takes_from_users`. `fallback::offered` (2). `fallback::prompt::a_model_without_the_screenshot_tool_is_not_told_of_one`. `fixture_harness::fallback_tests::the_screenshot_tool_is_offered_only_to_a_model_shown_tool_result_images`, on a real tab. | shipped |
| #8, structured notes (`cl-notes`) | The native-computer notes travel as data. `tool_display_payload` in `roder-api` keeps a bounded `computer_notes` array (8 notes of 160 characters), and `tool_result_computer_notes` reads it again with the same bounds, so a reader does not trust the writer's limits. The Responses replay sends those notes beside the screenshot and no longer parses the Notes block out of the result text; the text-parsing path is deleted. New public `COMPUTER_NOTES_DISPLAY_KEY`, `MAX_COMPUTER_NOTES`, `COMPUTER_NOTE_CHARS`. | `roder-api`: `src/transcript.rs`. `roder-ext-openai-responses`: `src/{computer,response_replay}.rs`, `src/computer_tests.rs`; new `src/computer_notes_tests.rs`. `docs/native-computer-use.md` | `roder-api` `transcript` tests (5), among them `display_payload_keeps_computer_notes_and_still_drops_other_keys`, `computer_notes_reader_bounds_a_payload_it_did_not_write` and `computer_notes_come_from_the_result_data_never_the_arguments`. `computer_notes_tests` (8), among them `native_notes_come_from_the_record_and_not_from_the_result_text` and `native_notes_are_the_only_record_field_sent_beside_the_screenshot`. The first-round notes tests were rewritten for structured data (same names), and `native_notes_ignore_bullets_that_are_not_in_the_notes_block` was removed with the parser. | shipped; the replay no longer adds a "N more notes were left out" line for notes past eight (see Needs attention) |
| `browser_use` review follow-ups (`cl-bu`) | A JSON-RPC error reply from a healthy browser-use server (for example an argument that fails the tool's schema) no longer shuts the server down and drops the owned browser, and its error no longer says the logins are gone. `roder-ext-mcp` exposes the reply as the typed `McpRpcError` (code and message) in the error chain, and `browser_loss.rs` tells it from a transport error, a timeout, a dead server, cancellation or a failed page read after an action, which still drop the browser and say so. The 1,293-line `tests/fake_server.rs` is split into modules (the root file is 40 lines). | `roder-ext-browser-use`: `src/{server,browser_loss,lib}.rs`, `tests/fake_server.rs`; new `tests/fake_server/{fake,support,lifecycle,ordering,loss,state_view,selects}.rs`. `roder-ext-mcp`: `src/{client,lib}.rs`. `docs/roder-browser-use-provider.md` | `fake_server::loss`: `a_json_rpc_error_from_a_healthy_server_keeps_the_browser_and_says_nothing_was_lost` and `an_error_reply_to_the_observation_after_an_action_still_takes_the_browser_down`. `browser_loss` unit tests (4). `roder-ext-mcp`: `rpc_error_is_typed_and_keeps_its_text_under_added_context` and `a_result_and_other_failures_are_not_rpc_errors`. | shipped |
| Chrome review follow-ups (`cl-chrome`) | The keyboard covered-focus refusal (`covered_focus`) now scrubs, tidies and caps the cover name as the click path does, and scrubs the focused control's label. Extension and Desktop page results are budgeted by lines (150) as well as characters (18,000), so core's cut at 200 lines and 20,000 characters is not reached and the outcome line and the omitted counts stay the last word. The native-computer batch header counts honestly: a failed action reads "action K of N failed" with K at most N, a batch where every action completed and only the cleanup failed says so, and a refused batch is not an action that failed (`direct/computer_header.rs`). A failed cleanup no longer replaces an action failure; `data.cleanup_error` is set whenever cleanup failed and `data.error` is unchanged for action failures. The direct-tool renderer `look::render` (Desktop and Jev) also cuts its text by lines. The oversized test files are split by behaviour. The built-in chrome skill no longer says `chrome_page_snapshot` returns element boxes. | `roder-ext-chrome`: `src/direct/{act,target,computer}.rs`, `src/{observed_render,session,extension_result}.rs`; new `src/direct/computer_header.rs`, `tests/extension_observation/{outcomes,old_build,budget,snapshots}.rs`, `tests/native_notes/{mac_chords,traps,lost_screenshot}.rs`. `roder-skills`: `builtin/chrome/SKILL.md`. `docs/roder-chrome-browser-extension.md` | `direct::computer_header` (6). `extension_observation::budget` (4), among them `a_page_of_many_short_controls_is_cut_by_lines_and_keeps_the_outcome_first`. `direct::target` unit tests `the_keyboard_refusal_names_a_hostile_cover_like_the_click_refusals_do` and `the_keyboard_refusal_scrubs_before_it_cuts_and_names_an_unnamed_cover`. Existing tests changed on purpose: `controls_come_before_the_text_and_all_of_them_survive_a_long_page` now uses 120 controls (a 160-control page can no longer list every control under the 150-line budget), `controls_are_cut_last_and_the_omitted_count_is_shown` asserts more than 100 listed, and `the_cap_holds_whatever_the_page_holds` became `the_budget_holds_whatever_the_page_holds`. The split test files are pulled in with `include!`, so test names are unchanged. | shipped |
| Webwright and docs follow-ups (`cl-ww`) | The "first error" line of a failed `webwright.run_script` is labelled as untrusted, redacted script output, as the stderr tail already was. `docs/browser-computer-use-audit.md` now says the action report comes first. `docs/jev-browser.md` says `jev_tab_select` follows the shared select matching and refusal rules. | `roder-ext-webwright`: `src/run_outcome.rs`, `src/run_outcome_tests.rs`, `src/run_script_tests.rs`. `roder-skills`: `builtin/webwright/SKILL.md`. `docs/roder-webwright-browser-agent.md`, `docs/browser-computer-use-audit.md`, `docs/jev-browser.md` | `run_outcome_tests::page_text_in_the_first_error_line_is_labelled_untrusted`, and `run_script_tests` asserts the "first error (untrusted script output, secrets redacted): …" wording on a real failed run. | shipped |
| Fallback reason label (final fix pass) | The fallback's opening message printed `Jev's reason` with no label. A `looped` stop quotes the page label of the control Jev kept choosing (up to 80 characters) and its row (up to 120), and a page that toggles a menu A, B, A, B can cause that stop on purpose, so page text reached a model with the full browser toolset outside the one list the prompt called untrusted. The line now reads "Jev's reason (quotes page labels; untrusted): …", is put on one line and cut to 300 characters, and the standing instructions name Jev's reason and last steps as untrusted beside the page content. | `roder-ext-jev`: `src/fallback/prompt.rs`. `README.md`, `docs/jev-browser.md` | `fallback::prompt`: `jevs_reason_can_quote_a_page_label_so_the_line_and_the_rules_call_it_untrusted` and `a_reason_with_line_breaks_stays_one_line_and_is_cut`, both failing before the change. | shipped |
| Dependents check (final pass) | `cargo check --tests` on the changed crates and every workspace crate that consumes the changed APIs, one crate at a time, with no compile errors. The `roder-api` changes are additive (the new default method `InferenceEngine::tool_result_image_input`, new consts and functions in `transcript.rs`, and an unchanged `tool_display_payload` signature), so compiling was judged enough. `docs/app-server/api.md` (the Cua desktop section) said model providers receive the screenshot as image content, which stopped being true when tool-result image forwarding became a per-engine opt-in; it now says engines that forward tool-result images get the image, any other engine gets the request without it plus a one-line notice, and the stored transcript and the ACP projection are unchanged. | `docs/app-server/api.md` | `roder-protocol` `--lib schema` (4 passed, including `checked_app_server_schema_file_matches_generator`), `roder-skills` (15 + 3 passed), and both `sdk/codegen/generate-{typescript,python}.mjs --check` (exit 0). `schemas/app-server/*.json` is unchanged and no generated SDK or schema file names a changed type. | shipped; the test suites of the large consumers were not run (see Needs attention) |

### Verification by crate

The project clippy command is the mise task `rust:clippy` in `.mise.toml`: `cargo clippy --workspace --all-targets -- -D warnings`. No workflow under `.github/workflows` runs clippy or fmt. The command fails on this tree because of lints in the dependency `roder-api`, so the crates below were also linted with `--no-deps`, or without `-D warnings`, to see their own findings. Baselines are the test counts recorded before the first round. The counts below are from the last full run of each crate after the second round. `roder-core`, `roder-ext-anthropic`, `roder-ext-gemini`, `roder-ext-xai` and `roder-extension-host` have no change since the first round, so their counts are the first round's. The second round also ran `cargo check --tests` on the changed crates and the crates that depend on them (`roder-app-server` and `roder` among them), with no errors; the check's own report states 45 crates but names 40, so no exact count is given here. `roder-ext-anthropic` printed 3 warnings in its test target (the `KeySourceGuard` and `clear_key_sources` `dead_code` pair listed below) and `roder-app-server` 5, in files this work did not touch.

| crate | tests | fmt | clippy, the crate's own findings |
|---|---|---|---|
| `roder-ext-browser-use` | 105 passed, 0 failed, 2 ignored (75 lib, 30 `fake_server`; baseline 34) | ok | clean |
| `roder-ext-chrome` | 128 passed, 0 failed (baseline 35) | ok | 3 warnings, all in code identical to HEAD |
| `roder-ext-webwright` | 68 passed, 0 failed, 1 ignored (baseline 35) | ok | clean |
| `roder-skills` | 18 passed | ok | clean |
| `roder-api` | 287 passed, 0 failed, 1 ignored | ok | 5 findings, all in files this work did not touch; `inference.rs` and `transcript.rs` are clean |
| `roder-core` | 411 passed, 0 failed, 1 ignored across 31 test binaries; lib 281 against a baseline of 267 | fails on 2 files this work did not touch | 14 locations, none in this work's code |
| `roder-ext-openai-responses` | 133 lib tests passed (baseline 124); 2 live tests ignored | ok | 1 pre-existing; changed code is clean |
| `roder-ext-anthropic` | 58 lib tests passed; 4 live tests ignored | ok | 2 pre-existing `dead_code` warnings in the test target |
| `roder-ext-gemini` | 20 lib tests passed; 1 live test ignored | ok | 2 pre-existing |
| `roder-ext-xai` | 7 passed; 1 live test ignored | ok | clean |
| `roder-extension-host` | 73 passed | ok | 1 pre-existing |
| `roder-ext-mcp` | 22 passed, 0 failed | ok | clean (3 findings in the `roder-api` dependency only) |
| `roder-ext-jev` | 715 passed, 0 failed, 12 ignored (635 lib, 80 in 11 integration binaries; 539 after the first round, and the baseline run was killed by the disk guard) | ok | clean |

`.changeset/` holds 28 files, one or more for each changed crate (12 after the first round), and the diff has no `Cargo.toml` or `Cargo.lock` change.

A full `roder-ext-jev` run during the final pass failed two Chrome-backed settle tests, `settle_tests::settle_waits_for_late_renders_and_is_bounded` (the test Chrome was killed with SIGKILL at start-up) and `settle_tests::a_page_that_never_goes_quiet_stops_at_the_cap` (a timing assertion saw `quiet` after 231 ms where it expects `cap`). Neither touches the code changed in that pass, both passed in the run before it and in the run after it, and the table gives the passing run.

### Deliberately not done

- **#13, #14 and #19.** These are measure-first items. #13 is baseline-gated, #14 is default-off until measured, and #19 builds nothing until a fixture reproduces the failure.
- **#16 and #18.** Low confidence in the report, and no work in either round.
- **#12 step 2.** The opt-in early end on `success_condition` (arming, `satisfied_at_start`) needs a live A/B and was not started. Step 1 shipped. One known limit of step 1: the page text carries no marker for which line came from a field, so the check removes one whole matching line per field value that `text.js` reports. Values cut off by the 6,000-character text limit are not reported, so on a page with that much text a partial echo can stay in the text.
- **The extension-repo merge in `roder-web-extention`.** That repo was read-only reference. For that reason `ch-3b` detects an old extension build by the shape of its result, not by a version flag.
- **#4, fallback routing of `DecisionUnusable`: no longer open.** The owner decided on 2026-10-09 that an exhausted decision error routes to the frontier fallback (fallback: yes), and a third-round change shipped it. An `error` with `stop_cause: decision_unusable` now falls back with the new trigger `decision_unusable`; every other `error` still has no trigger. The transport-level "Invalid TypeSafe response" (a body that cannot be decoded) is no longer left out: after the HTTP layer's one resend it counts as an unusable reply and is asked again up to twice, with unknown usage. Connection errors, timeouts and 401/402/403/429/5xx stay errors without a fallback. The report's retry cap of 2 is still unmeasured.
- **A Webwright per-run approval policy.** Product decision.
- **#9, approval auto-deny under non-interactive profiles.** The Jev half (handoff statuses as outcomes) shipped in the second round. Denying a Default-mode approval at once under a non-interactive `RuntimeProfile` (`tool_approvals.rs` in `roder-core`) was not done.
- **#1, the stale-index guard.** Skipped. In the pinned release indexes are backend node ids, so the renumbering premise does not hold; the evidence is in `docs/roder-browser-use-provider.md`. The epoch guard was also left out of `bu-1b`.
- **#2, the baseline and the live tier.** Parts a and b to e shipped. No `live-baseline.json` has been produced and the live tier (`JEV_EVAL_N`, the pin, the strict gate against a baseline) was never run, because the hosted service has answered HTTP 402 to earlier attempts and no key was used in either round. The tally, the pin and the save rules are covered by offline tests only. Still not built, as the report says: a four-surface frontier-model bed.
- **#3, other parts.** The merge of the extension branch is not done. `ch-3a` listed the dead-button and ref-forgery fixtures as not done, and no later item result names them. `ch-3b` added the 150 ms fallback snapshot and the parity test, and `ch-17` added a covered-button fixture.
- **#7, the browser crates' per-action screenshot request.** Unchanged, as instructed. The change only stops sending images to engines that drop them.
- **#8, the ACP fixture.** `roder-app-server/tests/acp_native_computer.rs` was not extended, and the `roder-app-server` tests were not run (the crate was only type-checked with `cargo check --tests`). The Mac-page and four-trap fixtures are in `roder-ext-chrome/tests/native_notes.rs` instead. Native computer also still does not stop on an access wall: there is no new `StopKind`, and `guard.refused()` is unchanged.
- **#8, the three wrong submits.** The single-batch fixture reproduces one wrong submit (to 0), not three. The saved run's three came from the model retrying across calls, and that was not reproduced.
- **#10, other runners.** `webwright/rerun` in `roder-app-server` and hand-run scripts do not write `run_exit.json`, so a run without one passes the `script_exit` check. The app-server `webwright/prepare` and the task executor keep an existing manifest silently.
- **#11, `docs/jev-browser.md:1740`.** It still says "use another browser tool". That is a paraphrase and stays accurate, and the file is outside the docs lane of that item.
- **#17, the chooser half.** The cover name now reaches the Jev digest and the fallback prompt. The failure-only, same-page-only filter on `Variant::Effect` and the cover-name field in `recent_actions` are not done: the chooser request stays byte-identical, and the report rates that part low confidence until the A/B runs.
- **#6(d).** The cross-call digest line (three consecutive non-done calls ending on the same page fingerprint) was skipped: it needs the fingerprint carried into `JevRunResult` and `CallRecord`.
- **Unmeasured thresholds in the Jev work.** The twin-row confidence 0.7, the pair limit 4, the stale limit 3, the wait limit 6 and the unusable-reply cap 2 come from the report and from fixtures. None was measured on a live model, and a false trip costs one fallback, not a failed task.

### Needs attention

**Decisions for the integrator**

- **Item #7 touches four crates outside its lane.** The one-method overrides in `roder-ext-anthropic`, `roder-ext-gemini`, `roder-ext-xai` and `roder-extension-host` must land with the `roder-api` and `roder-core` change. Without them the default (false) would strip screenshots from Anthropic, Gemini, SuperGrok and Codex.
- **RoderCloud.** It reports `image_input` false but maps requests through the Responses mapper. It is left at the default false, so it no longer receives tool-result images. Confirm that is intended. A provider with no registered engine keeps today's behaviour. In the second round the dependents check read the code of RoderCloud, Vertex, Claude Code, Cursor and the process host, and the correctness review read RoderCloud's: none maps tool-result images (RoderCloud flattens its input to text), so the default false is right for them today. An engine that later starts forwarding images must override the method.
- **Decision-service errors and the fallback.** Decided by the owner on 2026-10-09 (fallback: yes). An `error` with `stop_cause: decision_unusable` now falls back with the trigger `decision_unusable`; every other `error` does not. Three things to know. A declined confirm still stops this fallback, as it stops the blocked triggers. An undecodable body, which includes a truncated event stream and a proxy's HTML page served with HTTP 200, is now an unusable reply, so a persistent gateway page costs up to 6 POSTs (3 replies, 2 sends each) and then falls back, where before it failed after 2. Such a reply is recorded with unknown usage, so the run's whole decision token sum reads `unknown` (the call counts stay exact). The fallback's standing instructions still say Jev "could not make progress with the few actions it has", which is slightly off for a service failure; the opening message states the real cause.
- **Handoff statuses are no longer tool errors.** The first `needs_input`, `needs_confirmation` or `access_denied` result has `is_error` false. Providers and UIs that colour a result by `is_error` will render these differently. `data.outcome_class` carries the distinction, and an identical repeat is an error again.
- **`Agent task failed: ` prefix (`bu-1a`).** This is flagged as an error for the agent tool only. The task did not list it. Drop it for the strict eight-string set if preferred.
- **Webwright manifest (`ww-10`).** "Preserve the manifest" is implemented literally. A changed task, mode, start URL, browser or headless value is not applied, and only the `prepare_workspace` tool text reports it. The app-server `webwright/prepare` (`crates/roder-app-server/src/webwright.rs`, around line 65) goes through the same `create()`, so a second prepare with a different mode, start URL, browser or headless value is ignored and its response gives no sign of it.
- **One changeset line is still owed.** The final fix pass changed `roder-ext-jev` `src/fallback/prompt.rs` (the fallback's opening message labels Jev's reason as untrusted page text) and, as instructed, did not touch `.changeset/`. Add a patch line for it to a `roder-ext-jev` changeset, for example `.changeset/roder_ext_jev_cover_name.md`. `.changeset/` holds 28 files; `roder-ext-mcp` has its own, `roder_ext_mcp_rpc_error.md`, because `cl-bu` edited that crate.
- **Public shapes in `roder-ext-jev` moved without aliases**, as the repository rule asks. `JevControl.checked` is a typed `Option<bool>` and a toggle has no `value` (the old `[checked]` marker collided with a button whose value attribute was "checked"). The eval row has no `pass` field; `verdict_ok`, `truth_ok` and `false_green` replace it, and the paired-live eval's `telemetry.repeat` is now the typed `repeat`, so a script that reads the old key must change. `JEV_EVAL_STRICT=1` keeps its meaning (every run must pass) when no baseline file exists and now also fails on a mid-run edit, a false green or zero rows; with a baseline it gates on regressions instead of all-or-nothing. New: `JevStopCause::{DecisionUnusable, Looped, Unsettled}`, `JevRunResult.{suppressed_click, omitted}` and `JevActionRecord.covered_by`. The dependents check found no crate outside `roder-ext-jev` that references `JevStopCause`, `JevStatus` or `JevRunResult`.
- **`success_condition` wording changed (`jev-12`).** The digest header reads "condition met" or "condition not met" where it used to say the predicates passed and verify only themselves. The `status` values `passed` and `failed` are unchanged. A parser of the old header wording must follow.
- **Edits outside a lane.** `cl-bu` edited `roder-ext-mcp` (the typed `McpRpcError`; it adds no dependency and changes no message text) because a JSON-RPC error and a timeout could not otherwise be told apart without matching English text. `jev-4` made a one-line change in the OpenAI Decisions adapter, `crates/roder-ext-jev/src/decisions.rs`, outside `decide/`, because that adapter is the default path of that backend.

**Reviewer findings still open.** Three reviewers (correctness, trust, rules) each returned "ship" on the first round, and a second review of the second round produced two findings that the final pass fixed (the fallback reason label above, and the report's status section, which still described only the first round). Everything the first round's reviewers listed has been closed, except the following.

- `crates/roder-ext-chrome/src/direct/computer.rs`: a screenshot withheld because a typed password is still on the page returns a non-error result with a placeholder. Core's consecutive-failure stop never sees it, so blind batches could repeat. The result does name the reason.
- `crates/roder-ext-browser-use/src/tools.rs`: `shown_selects` is replaced outside the server's operation lock, so with parallel tool calls an older page's select set can overwrite a newer one. The effect is a missed or an extra refusal.
- `crates/roder-ext-browser-use/src/observation.rs`: the report now comes first and the 24,000-byte cut keeps the head, so a very long report (for example one that echoes typed text) can push the whole observation off the end. A cap on the report item would fix it.
- `crates/roder-ext-webwright/src/tools.rs`: `PrepareWorkspaceTool` reads the manifest before `create()`, and `create()` no longer rewrites an existing one. A corrupt `webwright.json` now makes `prepare_workspace` fail for good, where `create()` used to overwrite it.
- `crates/roder-api/src/transcript.rs`: `tool_result_computer_notes` and the display payload keep the first 8 notes and drop any further ones without a count. Chrome stays inside the cap and counts its own omissions ("… N more notes were left out."), so this shows only for another writer of `computer_notes`.
- Size rule (500 lines): `crates/roder-ext-browser-use/src/state_view_tests.rs` has 504 lines, `crates/roder-ext-chrome/src/observed.rs` 503, and `crates/roder-ext-chrome/src/tools.rs` 532 (521 at HEAD). `crates/roder-ext-openai-responses/src/response_replay.rs` has 508 (506 at HEAD). The first round's other oversized files were split in the second round: `fake_server.rs` is now 40 lines, `extension_observation.rs` 82, `native_notes.rs` 70, and `computer_tests.rs` 339.

Minor findings of the second round's three reviewers that the final pass did not fix. The handoff, `select.rs`, `browser_loss.rs`, `computer_notes.rs`, `include!` and docs items were checked against the tree when this section was written and still hold. The two Jev guard items are the reviewers' statements and were not re-read.

- `crates/roder-ext-jev/src/handoff.rs`: a repeat is checked only against the immediately previous call. A caller that alternates two different handoffs is never marked `repeated_handoff`, so core's five-in-a-row stop is not reached; the turn's 1,024-round cap bounds it, and every call is a billed Jev run. Keeping the last few handoff signatures per session would close it.
- `crates/roder-ext-chrome/src/direct/select.rs`: `describe` puts an option's label and value in double quotes without stripping `"` (each is cut to 40 characters in the page script), so a page can close the quote and fake further entries. The ambiguous-option refusal also lacks the "(untrusted page text)" label that the no-match and disabled-option refusals carry.
- `crates/roder-ext-browser-use/src/browser_loss.rs`: `BROWSER_LOST` says the tabs and logins are gone but not that the action sent just before the failure may already have run (a timeout, a killed server, or an action that succeeded and then failed the page read). The model may repeat a submit or a delete in the fresh browser.
- `crates/roder-ext-chrome/src/direct/computer_notes.rs` keeps its own `MAX_NOTES` (8) and `NOTE_CHARS` (160), copies of `roder-api`'s `MAX_COMPUTER_NOTES` and `COMPUTER_NOTE_CHARS`. Nothing links them, so they can drift and `roder-api` would then cut the notes.
- The Chrome test files split with `include!` (`tests/extension_observation/*` and `tests/native_notes/*`) are outside `cargo fmt --check`, because rustfmt does not follow `include!`. They were formatted by hand.
- Jev twin-row guard (`jev-5`): a goal that really needs both of two same-labelled commitment controls, for example two identical "Add to cart" buttons, can end `done` with the second click not made when the call confidence is below 0.7. The result discloses it (`suppressed_click`, the "Not clicked:" line and the Next sentence), but a caller that trusts `done` without reading them takes it as success. 0.7 was not measured on twin rows.
- Jev loop caps (`jev-6`): six idle waits end a run that is waiting on a slow server behind a static page, and a page with a clock never repeats a pair, so it is left to the budgets. The stale cap now behaves the same way: the owner decided on 2026-10-09 that it does not mask digits, so a page whose only change is digits is progress, and a form with a digits-only ticker whose every decision is stale no longer ends at the third one. It runs to the 120-call budget (a spent budget falls back) or to the timeout (which does not). A `looped` or `unsettled` stop after a dismissed confirm does not carry the "Jev never accepts a confirm" sentence in `stopped_because`.
- Stale numbers and descriptions in docs outside the lanes of this work: `docs/jev-browser.md` lines 1180 and 1226 say the keyless corpus lists and passes 40 tasks (it is 53; line 1336 of the same file says 53). `docs/browser-computer-use-audit.md` says "passed 52/52" at lines 123 and 251, and describes `success_condition` at line 207 as having only `url_contains` and `text_contains`. `docs/app-server/api.md` does not mention that a native-computer item's `input` can carry `computer_notes` (at most 8 strings of 160 characters).

Closed in the second round, for the record: `covered_focus` scrubbing, the 200-line budget, the cancel guard on JSON-RPC errors, the "action N+1 of N" and "action 1 of 0" headers, the chrome `SKILL.md` "element boxes" line, line 120 of `docs/browser-computer-use-audit.md`, the `jev_tab_select` note in `docs/jev-browser.md`, and the Webwright first-error label.

**Pre-existing failures, not caused by this work.** No test failed in any crate in the runs the tables above give (the two timing failures described above passed on rerun). All of the following exist on master `ef5a4b44` or in files this work did not touch, and none was fixed.

- `fmt`: `roder-core` fails `cargo fmt --check` on `crates/roder-core/src/instructions.rs` (one hunk) and `crates/roder-core/tests/agent_loop.rs` (four hunks).
- `clippy`, `roder-api`: `too_many_arguments` at `crates/roder-api/src/catalog/deepseek.rs:99` and `crates/roder-api/src/catalog.rs:1491`, and `large_enum_variant` on `ThreadItem` at `crates/roder-api/src/thread.rs:200`. The `roder-api` run also reports `items_after_test_module` in `computer.rs:74` and `useless_conversion` in `extension.rs:1318`. These make the workspace command fail whatever the other crates do. They were seen under toolchain 1.95.0, the one the guarded helper pins. They were not compared against a pristine master build.
- `clippy`, `roder-core`: 14 locations, among them `runtime.rs:4920` (`too_many_arguments`) and `tool_advertisement.rs:48` (`items_after_test_module`), none in this work's code. Dependencies of other crates add more: `roder-ext-git` (46 errors), `roder-tools` (3) and `roder-ext-runner-sprites` (5).
- `clippy`, `roder-ext-chrome`: `collapsible_if` at `src/direct/act.rs:370`, `too_many_arguments` on `computer_mouse` at `src/direct/computer.rs:327`, and `manual_range_patterns` at `tests/computer_use.rs:201`.
- `clippy`, `roder-ext-openai-responses`: `nonminimal_bool` at `src/computer.rs:122` in `validate_computer_request`. A reviewer suggested `request.model.provider == PROVIDER_OPENAI || (!advertised && ids.is_empty())`.
- `clippy`, `roder-ext-anthropic`: `dead_code` on `KeySourceGuard` (`src/provider.rs:517`) and `clear_key_sources` (`src/provider.rs:528`) in the test target.
- `clippy`, `roder-ext-gemini`: `manual_map` at `src/provider.rs:142` and `collapsible_if` at `src/provider.rs:281`.
- `clippy`, `roder-ext-xai` and `roder-extension-host`: `crates/roder-supergrok-auth/src/lib.rs:294` (`needless_question_mark`) and `crates/roder-extension-host/src/marketplace/codex.rs:20` (`items_after_test_module`).
- `cargo check` warnings, `roder-app-server`: the unused import `crate::client::AppClient` in `src/stats.rs`, unused imports in `tests/remote_app_client.rs` and `tests/process_extension_python_provider.rs`, and `focus_tab` never used when `tests/acp_native_computer.rs` includes the chrome example's `support.rs` through `#[path]` (`focus_tab` is not in that file's diff).

**Disk guard.** The first build of `bu-1a` was refused by the guard (about 190 MB free, exit 98). It stopped, waited until about 1.7 GB was free, then built, with no bypass. The Jev baseline was killed by the guard before it finished, so the first complete Jev run was the one that reported 539 passed. No integration run hit the guard; free space stayed near 6 GiB. The second round and the final pass built one crate at a time through the same guarded helper; free space was about 3.5 GiB at the end and the guard never fired. The dependents check saw free space fall from 5.1 GB to 3.9 GB, and its watchdog never fired either.

**Not run.**
- All ignored live tests: `browser-use` `tests/live.rs` (2, which need `uvx`, network and Chrome; the extended live fixture test is part of that set), `openai-responses` live (2), `anthropic` live (4), `gemini` live (1), `xai` live (1), `webwright` live smoke (1) and the 12 ignored `jev` tests (live Decisions and fallback runs, the live eval tier, the MiniWoB++ benchmark, a Codex sign-in test, two tests that open a Chrome window on screen, and one measurement). No `JEV_API_KEY` or other key was used.
- The `roder-app-server` tests, including the ACP native-computer fixture, because that crate was only type-checked (`cargo check --tests`, also with `--examples` and `--features e2e-tests`), not built into test binaries. The same holds for `roder-tui`, `roder-evals` and the `roder` CLI crate: type-checked only, with no test run. The Webwright rerun and verify flow in it, which relies on the "no `run_exit.json` passes" rule, was therefore not exercised.

`environments/AGENTS.md`, named in the user `CLAUDE.md`, does not exist in this worktree. Only the root `AGENTS.md` was available.

### Notes for line numbers

The report was written against commit `5e727cee`. Master has since moved: #109 (the Decisions backend and Jev changes) and #119 (Cua). The worktree is at `ef5a4b44`. Every `R:` line number above this section is stale, so locate code by symbol and grep.

- The Jev missing-key error cited at `runner/mod.rs:92` now lives in `crates/roder-ext-jev/src/runner/decision_backend.rs`, and there are two: one for `JEV_API_KEY` and one for `JEV_DECISION_PROVIDER=openai`.
- Line numbers quoted in this section are working-tree numbers unless stated. Where a pre-existing finding is quoted at HEAD, the number differs because this work edited the file above it. For example `roder-ext-gemini/src/provider.rs` shifted by 5 lines (137 to 142, 276 to 281), and `roder-ext-openai-responses/src/computer.rs:122` was line 67 at HEAD.

Claims in the report that did not hold as written in the current tree:

- **#7.** The report says only the Responses replay forwards `__view_image`. Anthropic (`media.rs`) and Gemini (`media.rs`) also forward it, and Codex and SuperGrok wrap the Responses engine. Vertex, chat-completions and the other providers do not. The trait method also takes the model, because SuperGrok image support is per model.
- **#3.** The "six-case `desktop_select` test" became ten cases.
- **#1.** Indexes in the pinned release are backend node ids, so the stale-index renumbering premise does not hold. Typing into a select is not a plain dead call: it clears the select, may or may not pick the option, and reports success either way.
- **#8.** `data.computer_notes` was dropped by `tool_display_payload` in `roder-api`, so the first-round replay had to parse the Notes block out of the result text. The second round keeps a bounded `computer_notes` array in the display payload and the replay reads that (`cl-notes`). The report did not verify that `computer_call_output` accepts text beside the screenshot, and the API takes only the screenshot there. The notes therefore go in a separate user message, as the error sidecar already does.
