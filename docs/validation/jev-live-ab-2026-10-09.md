# Live validation: Jev changes in RoderAI/roder#126

Branch `pz/browser-use-agent-improvements-779594` @ `9912e167` vs `origin/master` @ `ef5a4b44`. Date 2026-10-09.

## Verdict

- **No regression seen versus master, confidence medium.** All four A/B tiers (live corpus, session corpus, MiniWoB++, paired Decisions vs Jev) ran to completion with both arms concurrent. No difference was significant in either direction (every McNemar/Fisher p between 0.125 and 1.0).
- **The one branch-only failure of note** is `delayed_spa` in 1 of 3 live-corpus runs: the model's own first reply was BLOCKED at confidence 0.33 on a request byte-size-identical to the passing runs. No new guard was involved (inferred; n=3 cannot separate it from a small branch effect).
- **Paired `vision` was numerically behind in the first run (68/72 vs 71/72, p=0.37); a repeat measurement settled it.** Five more repeats on both arms, pooled to 8 per task, give 185/192 on each arm (95% interval about ±4 points). The first-run gap did not replicate (new run 117/120 vs 114/120), and every decision request the branch sent was also sent by master at the same step, so the differences are hosted-model variance. See "Repeat measurement of the vision variant". **Ahead on the branch:** paired `jev` 176/192 vs 166/192 pooled, all of it `icon_by_picture` 8/8 vs 0/8, which the new twin-row guard fixes.
- **Fallback after an exhausted decision error: works live.** 3/3 runs: three unusable replies, status `error` / stop_cause `decision_unusable`, the real `codex/gpt-6-sol (low)` fallback ran and the task graded `truth_ok`. The negative control (HTTP 401) made 0 fallback calls and exactly 1 POST. Scope limit: the decision service in that test is a local mock, not hosted Jev.
- **Secrets:** no key-shaped string found in rows, logs or this report. A literal grep for `sk-` matches only the substring in `task-values` (318 lines), so its raw count is not 0; `Bearer` and `Authorization` count 0 in every tier.

## What was run

| Item | Value |
|---|---|
| Branch / master | `9912e167` / `ef5a4b44` (clean detached worktree) |
| Decision model | `JEV_MODEL=jev-latest`, resolved to jev-1.13.0 on both arms |
| Paired Decisions model | gpt-6-luna (plus jev-1.13.0) on both arms |
| Text model | live corpus: task-values (no hosted text calls); MiniWoB++: inception/mercury-2.5 via OpenRouter, same on both arms |
| Fallback model | `codex/gpt-6-sol (low)` via Codex sign-in (fallback test only; fallback off in all A/B tiers) |
| Concurrency | both arms concurrent per tier; `JEV_REQUIRE_CHROME=1` |
| Preflight | 5 probes (live_corpus on both arms, miniwob, fallback, paired) all passed; no 401/402/429/5xx; no memory kill |
| HTTP errors | none in any tier; no watchdog exit (97/98) |

**Method deviation.** The tests were not run through `cb.sh`. Master and branch compile to the same test binary path (`deps/roder_ext_jev-891f4d2a603c20e5`) and cargo's mtime check made `cb.sh` execute the other tree's binary (measured: 499 vs 675 tests filtered). Each arm ran its own saved binary, built from its own tree, through `runbin.sh` and `withkeys.sh`. Arm identity was confirmed by the pin line (`9912e167`), corpus size (53 vs 52 tasks) and filtered-out counts. This also means "run both arms through `cb.sh`" is unsafe as written.

**Not run (by design or limit):**
- Anything larger than the named tiers, extra seeds or repeats.
- `JEV_EVAL_SAVE_BASELINE` (would write into the read-only tree).
- A fallback run against the hosted Jev service. The fallback test uses a mock decision service.
- A fallback success rate across the corpus (one task, `contact_form`, n=3).
- The session corpus test writes no rows file and records no steps or tokens, so guards are not observable there.

## Live corpus (53 tasks branch, 52 master; 3 runs per arm)

| Metric | Branch | Master | Recorded baseline (2026-09-28, older model/code) |
|---|---|---|---|
| Episodes passed, all tasks | 158/159 (53/53, 52/53, 53/53) | 156/156 (52/52 x3) | 41/44 + 7/8 = 48/52 |
| Episodes passed, 52 shared tasks | 155/156 | 156/156 | n/a |
| False greens | 0 of 159 | not recorded by master code | n/a |
| Discordant pairs | 1 (delayed_spa n2) | 0 | exact McNemar p=1.0 two-sided |
| Model calls, shared tasks, 3 runs | 595 | 600 | n/a |
| Decision input tokens, shared tasks | 1,226,049 | 1,234,734 (-0.7% on branch) | n/a |
| Steps, shared tasks | 460 | 465 | n/a |
| Test-reported wall per invocation (s) | 61.9 / 52.8 / 63.3 | 80.7 / 81.8 / 76.2 | n/a |

**Regression:** `delayed_spa`, branch 2/3 vs master 3/3. Run 2 stopped at step 0, 1 model call, first reply BLOCKED at 0.33 (stop_cause `model_blocked`, `stopped_because` null). The request was 5227 bytes with 2 controls offered, as in every passing run. The same first call is low-confidence on both arms (CLICK e1 at 0.47 / 0.43 on branch, 0.40 / 0.47 / 0.39 on master). Not a new guard.

**Gains:**
- `rotating_frame_no_progress`: both arms 3/3 pass, blocked; branch stops at 5 steps / 5 calls vs master 6 / 6 (8,426 vs 10,233 input tokens per run). Cost gain from the changed stall rule, same outcome.
- `icon_by_picture_twin` (branch-only task): 3/3, 1 step / 2 calls. No master comparison.

Every gate_* and blocked / needs_input / access_denied task matched master's outcome in all runs.

## Session corpus (8 tasks x 2 invocations)

| Metric | Branch | Master | Recorded baseline |
|---|---|---|---|
| Episodes passed | 14/16 | 14/16 | none for this corpus |
| Per-invocation summary | 7/8, 7/8 | 7/8, 7/8 | n/a |
| Discordant pairs | 0 | 0 | exact McNemar p=1.0 |
| Printed test time (s) | 17.10 / 15.97 | 16.94 / 15.11 | n/a |
| Model calls / tokens | not reported by this tier | not reported | n/a |

**Regressions:** none. The same task, `search_then_submit_session`, failed at call 2 with status "blocked" != "done" in all four invocations. Pre-existing on master. The grader's other call-2 checks appear to have held, so the page seems to have submitted correctly and only the reported status was wrong (inferred from source; cause not diagnosed).

**Evidence is weak:** 2 calls per task, no steps or stop causes in the output, and the test exits 0 even with a failed task (it never asserts).

## MiniWoB++ (129 tasks x seeds 0-4 = 645 episodes per arm)

| Metric | Branch | Master | Recorded baseline (2026-09-28) |
|---|---|---|---|
| Headline (unsupported = failures) | 362/645 = 56.1% | 358/645 = 55.5% | official 368/645 (57.1%) |
| As attempted (own reward) | 380/645 = 58.9% | 376/645 = 58.3% | 384/645 (59.5%) |
| Supported only | 362/440 = 82.3% | 358/440 = 81.4% | n/a |
| Unsupported, attempted | 18/205 = 8.8% | 18/205 = 8.8% | n/a |
| Paired: both pass / both fail | 369 / 258 | 369 / 258 | n/a |
| Discordant: only this arm passes | 11 | 7 | exact McNemar p=0.481 |
| Run errors | 6 (all text-helper) | 11 (10 text-helper, 1 invalid decision response) | n/a |
| Decision model calls | 1746 | 1790 | n/a |
| Executed steps | 1425 | 1491 | n/a |
| Wall, test-printed | 467.3 s | 430.5 s | n/a |

**Regressions (branch-lower episodes):** `multi-layouts` s1, s3 (3/5 vs 5/5) and `multi-orderings` s2 (4/5 vs 5/5), all "Text helper returned no valid field value" from mercury-2.5. `email-inbox-nl-turk` s3 (4/5 vs 5/5), same error. Also `click-tab-2-medium` s3 (4/5 vs 5/5, first click chose "Tab #2" instead of the target link), `copy-paste` s3 (typed value differed in a trailing space; flips both ways across seeds) and `circle-center` s4 (mirror image of s3). None involves a guard stop reason.

**Offsets:** 5 master episodes lost to the same text-helper error that cost the branch 4. Branch also won `login-user` s3, `generate-number` s4, `phone-book` s3 (variance, no guard).

**Cost:** decision calls -44 (-2.5%) on the branch, entirely from daily-calendar; excluding it, 1696 vs 1665 (+1.9%). Text-helper calls 246 vs 255.

## Paired Decisions vs Jev (24 tasks x 3 repeats + 4 held-out)

Branch ran 4 variants (original, effects, vision, jev); master ran 2 (vision, jev). Pass = verdict_ok && truth_ok on the branch, `pass` on master (same check set).

| Metric | Branch | Master | Recorded baseline (2026-10-08) |
|---|---|---|---|
| Confirmation, jev (72) | 66/72 | 62/72 | 62/72 |
| Confirmation, vision (72) | 68/72 | 71/72 | 68/72 |
| Confirmation, original (72) | 53/72 | not run on master | 51/72 |
| Confirmation, effects (72) | 64/72 | not run on master | 62/72 |
| Held-out (12 each): original / effects / vision / jev | 10 / 11 / 12 / 12 | vision 12, jev 12 | 12 / 10 / 12 / 12 |
| Pooled vision+jev, 168 matched | 158/168 (94.0%) | 157/168 (93.5%) | n/a |
| Discordant (branch-only / master-only) | 4 / 3 | | exact McNemar p=1.000 |
| False greens, confirmation set (288 on branch) | 22 (original 9, effects 7, jev 4, vision 2) | jev 4, vision 0 (derived from failure strings) | n/a |
| Decision calls, whole tier | 976 (336 episodes) | 477 (168 episodes) | n/a |

**Regressions (vision variant, both traced to hosted-model variance):**
- `frame_form` 1/3 vs 3/3. Rep2: same first request, different first reply (TYPE_TEXT e3 vs e1); trajectories diverged. Later requests differ (recent_actions order, one screenshot byte), and the branch's DONE 0.46 request has no master counterpart. The branch answered DONE 0.46 and the form was never submitted (false green); master's rep2 value is CLICK 0.37. Rep3: requests byte-identical at all 5 decisions; CLICK Subscribe at 0.82 vs 0.35 on master; form had posted, then four Subscribe clicks with "nothing visible changed" ended the run via the pre-existing `stalled` rule.
- `twin_by_context` 2/3 vs 3/3. Rep1 false green (Desk Lamp added twice): second-click confidence 0.75 on the branch vs 0.51 / 0.32 / 0.49 on master; 0.75 is above the existing 0.7 repeat threshold, so master's rule would have done the same on the same reply.

**Gains:**
- `icon_by_picture` (jev) 3/3 vs 0/3. Master's three runs executed the second delete at 0.52 / 0.63 / 0.57 and ended blocked with mail3 missing. The branch held the unsure twin click (0.64 / 0.57 / 0.52) and ended done. Caused by the twin-row guard.
- `below_the_fold` (jev) 3/3 vs 2/3: master's miss was an unusable first reply; the branch got 0 unusable replies in 210 requests, so the retry did not run. Treat as variance.

**Pre-existing, identical on both arms:** jev `enter_to_search` 3/3 fail, false green 3/3 on both arms; jev `scroll_region` 3/3 fail on both arms but only 1 of 3 is a false green (2 of 3 end blocked, `model_blocked` on the branch). Total jev false greens stay 4 = 1 + 3 on each arm. Vision `icon_by_picture` 1 miss.

## New guards in the wild

| Stop cause / guard | Live corpus (159 ep.) | MiniWoB++ (645 ep.) | Paired, branch (336 ep.) | Session corpus (16 ep.) | Cost |
|---|---|---|---|---|---|
| looped (same-pair cap) | 0 | 5 (all daily-calendar, unsupported) | 0 | not observable | 0 episodes lost; 75 calls saved (50 vs 125); master also failed all 5 |
| unsettled (stale cap) | 0 | 0 (inferred from rows) | 0 | not observable | none |
| wait cap | 0 | 0 | 0 | not observable | none |
| decision_unusable | 0 | 0 | 0 (0 of 210 jev requests errored) | not observable | none |
| Changed stall rule (idle waits) | `rotating_frame_no_progress` 5 vs 6 steps | not measurable | no WAIT decisions on branch | not observable | cheaper stop, same outcome |
| Twin-row / unsure-repeat suppression | not visible in rows | not reproducible from the rows (no stop_cause or per-decision operation in MiniWoB++ rows) | 3 jev `icon_by_picture` episodes held and passed | not observable | gain: 3 episodes |
| Frontier fallback | off | off | off | not configured | not exercised in A/B tiers |

Observable guard cost across the 159 + 645 + 336 episodes: **0 episodes**. The stale cap did not shorten the known `social-media-all` / `social-media-some` 25-decision cap burns (6 branch episodes vs 5 on master: all 3 vs 2, some 3 vs 3); the rows do not show why.

## Live fallback after an exhausted decision error

New test `decision_fallback_live` (uncommitted; one `mod` line in `fixture_harness/mod.rs` plus a new 499-line file). It drives the real hosted-decision HTTP client against a local mock that returns undecodable 200 bodies, with the real fallback model wrapped in a counter. Request-count derivation: 2 POSTs per decision (one resend) x 3 decisions = 6 POSTs.

| Run | Jev status / stop_cause | Mock POSTs | Fallback tool calls | Fallback model calls | Tokens in / out | Fallback elapsed | Graded |
|---|---|---|---|---|---|---|---|
| 1 | error / decision_unusable | 6 | 5 | 6 | 14985 / 234 | 9753 ms | verdict_ok, truth_ok |
| 2 | error / decision_unusable | 6 | 4 | 5 | 12278 / 219 | 48790 ms | verdict_ok, truth_ok |
| 3 | error / decision_unusable | 6 | 5 | 6 | 15051 / 238 | 12641 ms | verdict_ok, truth_ok |

All runs: Jev decisions 3, Jev actions 0, fallback trigger `decision_unusable`, fallback label `codex/gpt-6-sol (low)`, 0 tool errors, 0 grader misses, task `contact_form`.

**Negative control (HTTP 401):** status `error`, stop_cause `stopped`, stopped_because contains "HTTP 401", no `fallback` key, fallback model asked 0 times, exactly 1 decision POST, 0 form POSTs reached the site.

**Preflight corroboration:** `canvas_fallback` with `JEV_EVAL_FALLBACK=model` went blocked -> fallback -> done (4185 ms). That reaches the fallback via a blocked verdict, not the exhausted-decision path; it only shows the live fallback model and sign-in are usable.

**Reviewer: approve.** Re-ran fmt (exit 0), clippy on tests (0 warnings) and keyless tests (2 passed, 1 ignored). Verified the request-count derivation and the grading against the corpus `Expect::grade`. Five minor issues:
1. The decision service is a mock; the result says nothing about live Jev decision quality.
2. `real_fallback()` mutates process env with `unsafe set_var`, so a broad filter (`--ignored live`) could contaminate other live tiers. Run only with the exact test name.
3. The `gpt-6-sol` label check runs after the calls are spent.
4. `UNUSABLE_REPLIES = 3` duplicates the loop's private `ASKS_AGAIN + 1`; `call_outcome` is valid only for tasks like `contact_form`.
5. Style: the file is at 499 lines (the 500-line guidance).

## Repeat measurement of the vision variant (run before merging)

The first paired run left `vision` numerically behind (68/72 vs 71/72, p=0.37) and attributed it to hosted-model variance from rows, not from a repeated measurement. This section is that repeated measurement: 5 more repeats of the `vision` and `jev` variants on the same 24 confirmation tasks, both arms concurrent from their saved binaries (branch `9912e167`, master `ef5a4b44`), pooled with the first run for 8 repeats per task per arm (192 attempts per variant per arm). A decision rule was fixed before the run: proceed if no branch-only failure involves a new guard and the pooled vision difference has its 95% lower bound at or above -6 points.

| Variant | Scope | Branch | Master | Difference | 95% interval (stratified MH; Newcombe) |
|---|---|---|---|---|---|
| vision | new run (120) | 117/120 | 114/120 | +2.5 pp | [-1.4, +6.4]; [-2.8, +8.2] |
| vision | first run (72) | 68/72 | 71/72 | -4.2 pp | [-8.6, +0.3]; [-12.1, +2.8] |
| vision | pooled (192) | 185/192 | 185/192 | 0.0 pp | [-3.2, +3.2]; [-4.1, +4.1] |
| jev | new run (120) | 110/120 | 104/120 | +5.0 pp | [+3.5, +6.5]; [-3.0, +13.1] |
| jev | pooled (192) | 176/192 | 166/192 | +5.2 pp | [+3.9, +6.6]; [-1.1, +11.6] |

- **Decision: no regression, proceed.** The first-run vision gap did not replicate (the new run favours the branch by 2.5 points). Pooled it is a tie, with the lower bound of every interval above -6 points.
- **The Jev gain is one task.** `icon_by_picture` is 8/8 on the branch and 0/8 on master (the twin-row guard fires 8 of 8; master deletes the second twin). Without it Jev is +1.1 points (168/184 vs 166/184, p=0.86). The stratified interval for the pooled Jev variant is narrow only because that stratum has no variance.
- **The branch changes nothing the model sees in these tasks.** All 1,160 branch decision requests (562 Jev, 598 vision) were also sent by master at the same step; the first request of each same-run pair was byte-identical in 383 of 384 pairs (one 1 px geometry difference), so every difference in outcome is the hosted model answering the same input differently.
- **Hosted-model variance, measured:** among groups of byte-identical requests with at least two attempts, the answer was not unanimous in 13 of 106 vision groups and 8 of 74 Jev groups, always on both arms for vision. For example `twin_by_context` step 2 answered CLICK with confidence from 0.32 to 0.89 on one identical request.
- **No new guard was involved in any branch-only failure** (7 of them, all traced to model variance, the shared DONE heuristic, or the stall rule that exists on master). Across 528 branch attempts no run ended with `looped`, `unsettled` or `decision_unusable`, and the unusable-reply retry never fired; these guards are covered by fixture tests and cost nothing here, but this data does not exercise them.
- **A pre-existing weakness this surfaced (not caused by the PR):** on `twin_by_context` the `vision` variant sometimes answers "add to cart" at confidence above the 0.7 repeat threshold again and again, and the cart ended with 14 to 18 lamps on both arms (master 16 in the same run). Only a decision that falls below 0.7 stops it. A guard that reads confidence cannot bound a model that stays confident.
- **Caveats.** The repeats are of 24 fixture pages, not independent sites; hosted-model drift between the first and the new run is not excluded; a small regression (a few points) is not ruled out. Speed and cost were unchanged within noise (median wall 2.35 s vs 2.23 s vision, 1.93 s vs 1.76 s Jev; mean model calls 3.11 vs 3.17 and 2.93 vs 2.94).

## Caveats

- **Sample size.** 3 repeats x 52 tasks, 2 repeats on sessions, one MiniWoB run, 3 repeats paired. A true branch failure-rate increase of a few percent is not excluded (live corpus Wilson upper bound for 1/156 about 3.5%; Decisions-vs-Jev pooled unpaired Newcombe interval -4.9 to +6.1 pp, while the paired (discordant-pair) CI is about -2.5 to +3.7 pp and the conclusion does not change; MiniWoB paired CI -0.67 to +1.91 pp).
- **Hosted-model variance.** The same input gave 0.35 vs 0.82 confidence at one step. Master vision swung 71/72 against a recorded 68/72, so about 3 episodes of run-to-run spread is normal.
- **Wall time is not a result.** Branch was faster on the live corpus and 8.6% slower on MiniWoB++. Arms shared one Mac, and the branch shares one headless Chrome while master starts one per test. Not isolated.
- **Recorded baselines** come from an older model and code (and a different corpus size); context only.
- **Exit codes mislead.** `live_corpus` and `live_session_corpus` exit 0 with failing tasks. Read the printed summary line.
- **Observability gaps.** Master rows carry no stop_cause, false_green, repeat or fallback fields, so master guard attribution is by status, steps and effects. MiniWoB++ rows have no stop_cause field either.
- **Fallback test scope.** One task, n=3, mock decision service, fallback via Codex sign-in. Latency varied 9.8 s to 48.8 s with the same call counts.
- **Corpus difference.** The branch has one extra task (`icon_by_picture_twin`). Only the 52 shared tasks are compared one for one.

## Recommended follow-ups

- **Nothing blocks the PR on this evidence.** Optionally keep an eye on `delayed_spa` and paired `vision` `frame_form` / `twin_by_context` on the next live run; no guard change is supported by the data.
- **Commit `decision_fallback_live`** after two small fixes from the review: move the `gpt-6-sol` label check right after `real_fallback()` (before spending calls), and tie `UNUSABLE_REPLIES` to the loop constant or comment the dependency. Keep running it by exact test name.
- **Look at `search_then_submit_session`** (blocked at call 2 on both arms, pre-existing). Page state appears correct, so this is a status-reporting defect. Separate from this PR.
- **Make `live_corpus` and `live_session_corpus` fail on a failed task** (or print a non-zero marker). Both exit 0 today.
- **Fix the shared test-binary path hazard** between `CB_DIR` trees before the next A/B (per-tree `CARGO_TARGET_DIR` or a distinct profile/hash), otherwise `cb.sh` can run the wrong tree.
- **If the unsettled cap is expected to help `social-media-*`,** add a test, since the rows do not show why it did not engage there.
