# USPS browser comparison — October 28, 2026

This comparison motivated the semantic observation and context-budget changes in
PR #210. The original trials identified extra parent-agent work; the whole-task follow-up
below implements and measures the resulting changes.
These are single trials with different browsing strategies, not a controlled
estimate of model or harness speedup.

## Task and build provenance

Starting at <https://www.usps.com/ship/priority-mail.htm>, compare standard,
unmodified Priority Mail Flat Rate boxes for a rigid 10 × 8 × 4 inch, 8 lb item
sent within the contiguous United States at Post Office retail prices. Compare
Small, both Medium configurations, Large and Board Game boxes; open the selected
product page and report weight, tracking, insurance and delivery caveats. Do not
order, add to cart or sign in. Return ordinary prose with evidence links.

The observed answer was Medium Flat Rate Box 1, with interior dimensions
11 × 8½ × 5½ inches and a $25.80 retail price. Prices and service details here are
historical observations, not a current shipping quote.

Live candidate runs used commit `18fc3930` (bounded Jev requests and formatter
changes). The subsequent shared-observation implementation at `552372ba` was
validated with fixtures and real-browser integration tests, but was **not** the
binary used for these live measurements. Roder's parent model was Terra 6.1 low;
the direct Terra arm used regular Chrome tools without Jev delegation.

## Timing

| Arm | Total¹ | Browser interval² | In browser tools³ | Outside tools³ | Browser calls | Screenshot calls / time |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Codex | 39.447 s | 23.968 s | 12.321 s | — | 7 | 2 saved captures / not isolated |
| Roder Decisions | 96.629 s | 74.533 s | 34.489 s | 62.140 s | 17 | 6 / 16.442 s |
| Roder Jev | 74.949 s | 62.881 s | 22.032 s | 52.917 s | 20 | 7 / 9.464 s |
| Roder direct Terra | 19.891 s | 11.570 s | 3.228 s | 16.663 s | 6 | 0 |

1. Roder totals include CLI startup and the final answer. Codex's 39.447 seconds
   run from the instruction timestamp to the final answer. Its originally shown
   **27-second** timer started after dispatch and stopped before the final answer.
   Neither total nor browser interval alone makes the execution policies equal.
2. First website-browser tool invocation through last website-browser result.
3. Roder tool time uses the union of tool execution intervals, deduplicating start
   and completion records. Remaining time combines model, network and orchestration
   work; these traces cannot attribute it to inference alone. Codex tools sometimes
   batch browser state, screenshot and file work, so pure capture time is unknown.

The Decisions inner browser loop took **7.867 seconds** for 10 decisions and 7
actions; the surrounding `jev_browse` call took 8.773 seconds. Jev's inner loop
took **1.951 seconds** for 4 decisions and 2 actions; its `jev_browse` call took
3.030 seconds. Most elapsed time therefore occurred outside the delegated loop.

| Arm | Reported input tokens | Cached input tokens | Output tokens |
| --- | ---: | ---: | ---: |
| Codex | 684,558 | 563,200 | 1,308 |
| Roder Decisions | 780,485 | Not aggregated | 1,015 |
| Roder Jev | 692,863 | Not aggregated | 1,948 |
| Roder direct Terra | 172,303 | 141,568 | 602 |

Roder delegated-arm input totals combine parent and driver usage: Decisions
546,113 + 234,372; Jev 654,139 + 38,724. These are cumulative reported usage across
requests, not unique context sizes. Experimental Jev completed without context
overflow or fallback in this candidate run. Direct Terra reported zero reasoning
tokens. Token totals reflect different trajectories and capabilities.

## Findings

The Roder evaluation wrapper explicitly required `jev_browse` for the first
comparison, followed by independent manual product and caveat verification.
Codex did not have that requirement. This forced a second pass and prevents
attributing the observed gap solely to either driver.

Direct observations exposed controls and headings, but their text excerpt began
at the start of the whole document. Scrolling could return navigation text and
“Page text unchanged” while relevant product paragraphs were visible elsewhere.
The parent consequently used repeated scroll/screenshot pairs. This motivated the readable viewport text observation implemented in the
whole-task follow-up below.

The parent made 17–20 sequential browser calls. Screenshot calls alone consumed
9.464–16.442 seconds; the longest Decisions capture took 6.429 seconds. Capture,
resize and transport phases were not separately instrumented, so this does not
establish an encoding bottleneck. The direct Terra trajectory read page text and
links, navigated to the product, and read its text in six calls.

## Screenshot inspection

The viewer previously displayed reduced 420-pixel previews. It now links to 49
recovered original saved captures, preserving their bytes and recording SHA-256
checksums. Typical Roder captures are 1625 × 869; the Codex capture is 1174 × 873.
The model screenshot encoding remains unchanged. These parent-tool captures do
not establish the exact image bytes sent by every internal Jev provider request.

## Follow-up execution policy

Delegate the entire task, including product verification and service caveats, in
one `jev_browse` call. Let the parent inspect the returned evidence and investigate
only missing or contradictory facts. Use readable viewport text with existing
secret scrubbing and context limits. Avoid screenshots solely for artifact
collection, and record model, browser capture and orchestration phases separately.

Use the same task, capabilities, evidence policy and timing boundaries across
arms, with fresh sessions and repeated randomized trials. Preserve these results
as historical baselines rather than replacing them with the next measurements.

## Evidence and validation

Local evidence is retained at
`/Users/pz/roder-browser-evals/usps-2026-10-28/`:

- `timing-analysis.json`, `timing-analysis.txt` and `analyze_timing.py`: event-based
  timing calculations and per-tool intervals.
- `timing-findings.json`, `roder-wrapper.txt` and `next-eval-wrapper.txt`: findings,
  original execution policy and proposed replacement.
- `candidate-{decisions,jev,terra}/`: saved API/tool events, session artifacts and
  final replies; `provenance.json` records build provenance.
- `evidence-manifest.json`, `evidence/`, `sessions.zip` and `checksums.sha256`:
  original captures and archived session evidence. Raw session material remains
  local; this report is the repository review artifact.

At `552372ba`, the Jev suite passed 748 tests (14 ignored), Chrome passed 123
unit tests and the real-browser integration passed. Checks covered shared
semantic hierarchy/state/ref behavior, parent handoff, bounded requests and
secret echoes at excerpt boundaries. Formatting, changeset and generated release
configuration checks passed. These validate the implementation, not a live
performance improvement for that final commit.


## Whole-task delegation follow-up

The new wrapper delegates every requirement in one `jev_browse` call and asks the
parent to investigate only missing or contradictory evidence. It no longer
requires a second manual verification pass or screenshots for artifact collection.
Both drivers use Terra 6.1 low for the parent and automatic fallback. The native
arms remain Decisions and experimental Jev with images respectively.

Implementation changes:

- Read rendered viewport text, including passive paragraphs and clipped scroll
  panels; provide bounded document reads with observed links and continuation
  offsets for research without repeated scrolling. Scrub before pagination and
  leave the viewport baseline unchanged.
- Retain bounded, source-linked observations across the delegation and page them
  within Jev's existing request limits. Preserve historical text when compacting
  fallback tool results, while discarding stale controls and older screenshots.
- Treat an uncertain repeated click as incomplete, suppress it, and recover
  within the same call when fallback is configured. It previously reported
  success without establishing completion.
- Return up to 8,000 characters of fallback prose findings, with priority in an
  18,000-character/180-line parent digest. The previous 600-character capture and
  300-character digest lost findings and forced the parent to browse again.
- Use a 12-action native fast-path budget when automatic fallback resolves,
  preserving an explicit `JEV_MAX_ACTIONS` setting. The latest trials recovered
  before reaching that cap, so they do not isolate its performance effect.

### Latest measured runs

| Arm | Total | In parent browser call | Outside tools | Parent calls | Native actions / provider calls | Fallback calls | Input tokens | Output tokens |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Decisions + Terra fallback | 59.870 s | 40.682 s | 19.188 s | 1 | 2 / 6 | 9 | 299,121 | 1,628 |
| Jev + Terra fallback | 72.668 s | 47.641 s | 25.027 s | 1 | 1 / 3 | 15 | 214,040 | 2,522 |

Both final replies correctly selected Medium Box 1 at the observed $25.80 price,
compared all five configurations, verified the product dimensions, and included
the packed weight limit, tracking, insurance conditions and non-guaranteed
service estimate. Both returned ordinary prose with source links and made no
parent screenshot calls or subsequent manual browser calls. Decisions recovered
from `unsettled`; Jev recovered from `uncertain_repeat`. These are successful
combined driver/fallback runs, not native-driver-only completions.

Reported input combines parent, native and fallback usage respectively:
Decisions **41,873 + 172,387 + 84,861**; Jev **41,878 + 31,086 + 141,076**.
Parent cached input was 19,456 and 38,912. Output totals use the same three sources.
Usage is cumulative across requests, not the maximum simultaneous context size.

Compared with the earlier wrapper, Decisions elapsed time fell from 96.629 to
59.870 seconds and input from 780,485 to 299,121. Jev elapsed time changed from
74.949 to 72.668 seconds and input from 692,863 to 214,040. The strongest shared
result is eliminating the second parent browsing pass, with materially lower
reported input. Single trials, changed execution policy, caching and variable
model/network latency do not establish a controlled speedup. The historical
19.891-second direct Terra run remains faster; Codex and direct Terra were not
rerun in this follow-up.

### Intermediate trials retained

| Trial | Total | Parent calls | Input tokens | Finding |
| --- | ---: | ---: | ---: | --- |
| delegation / Jev | 61.818 s | 14 | 453,281 | Uncertain repeat falsely reported success; parent recovered |
| delegation / Decisions | 134.859 s | 9 | 1,202,682 | Native loop and fallback budget exhausted; parent recovered |
| v2 / Jev | 73.869 s | 6 | 318,318 | Same-call fallback succeeded but truncated findings caused parent follow-up |
| v3 / Jev | 77.856 s | 1 | 252,880 | Full findings removed parent follow-up |
| v3 / Decisions | 160.906 s | 1 | 1,986,907 | 60 native actions exposed the need for an earlier recovery budget |

All trajectories remain available, including slower and unsuccessful intermediate
native attempts. They should not be pooled as repeated trials of one build.

### Build and verification

The latest `delegation-v4-{decisions,jev}` runs used the same binary with SHA-256
`85d5744ddfc5151abb41163cfaf1d58ff3732e9aae724b36341548a9a4d21bcf`.
It was built on `02bb1ac8` plus the follow-up source patch; the exact patch and new
files are saved with `delegation-v4-provenance.json`. Subsequent source changes
only removed an unused function argument and updated documentation/formatting;
the measured behavior is unchanged. Wrapper SHA-256:
`199fe6833457a2757565411ce7587aef851eb7e1e981c3cde3b82e7cc897bdd1`.

Latest verification passed **757 Jev library tests** (14 ignored), **91 Jev
integration tests**, **123 Chrome unit tests** and the real-Chrome computer-use
integration. Coverage includes viewport clipping, document pagination and secret
scrubbing, retained multi-page evidence, request budgets, fallback history and
findings, and one-call recovery without repeating a completed click. Formatting,
changeset and generated release configuration checks also pass.

Local `delegation-analysis.json` and `analyze_delegation.py` retain the usage and
interval calculations. Each trial directory contains final prose, execution
records, API transcripts and session records. Source patches and build hashes
accompany each iteration; raw session contents remain outside the repository.
