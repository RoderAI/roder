# USPS browser comparison — October 28, 2026

This comparison motivated the semantic observation and context-budget changes in
PR #210. It also identified extra parent-agent work that remains to be addressed.
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
The parent consequently used repeated scroll/screenshot pairs. A readable
viewport text observation remains a proposed improvement, not a completed fix.

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

## Next experiment, not yet run

Delegate the entire task, including product verification and service caveats, in
one `jev_browse` call. Let the parent inspect the returned evidence and investigate
only missing or contradictory facts. Add readable viewport text with existing
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
