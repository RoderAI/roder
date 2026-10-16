# Jev 1.14 with screenshots: faster than visual Decisions, not more accurate

On the 24-task confirmation set, **Jev 1.14 with viewport screenshots passed
66/72 (91.7%)**. Visual OpenAI Decisions passed **71/72 (98.6%)** in the same
run. Jev was faster: median wall time **1,536 ms vs 1,862.5 ms**, and median
decision latency **112 ms vs 229 ms** per call. On four held-out tasks, Jev 1.14
with screenshots passed **9/12**; visual Decisions and both text-only Jev
models passed **12/12**.

Screenshots made up for Jev 1.14's text-only regression in total, though not
task for task. Jev 1.14 text-only passed **59/72** compared with **66/72** for
Jev 1.13. Adding images brought 1.14 back to 66/72, but at **2.5× the input
tokens per call**. The images gained `scroll_region`, which 1.13 fails, while
1.14 still failed `icon_by_picture`, which 1.13 passes. Images did not improve
held-out accuracy and caused three new held-out failures.

This does not support switching Roder's opt-in OpenAI visual backend to Jev
1.14 with images. It also does not support moving the Jev default from 1.13
to 1.14 text-only, which is what will happen when `jev-latest` moves. Image
input remains opt-in (`JEV_IMAGES=1`). These are 28 local fixture tasks; they
do not establish behaviour on arbitrary websites.

## Confirmation: 24 tasks, three repetitions, four variants

| Variant | Passed | Rate | Median wall time | Median decision latency | Browser decisions | Input tokens | DONE with failed outcome |
|---|---:|---:|---:|---:|---:|---:|---:|
| Visual Decisions (`gpt-6-luna`) | **71/72** | **98.6%** | 1,862.5 ms | 229 ms | 215 | 415,662 | 0 |
| Jev 1.13 text (`jev-1.13.0`) | 66/72 | 91.7% | 1,255 ms | 98 ms | 209 | 381,598 | 3 |
| Jev 1.14 text (`jev-preview` → `jev-1.14.0`) | 59/72 | 81.9% | **1,175 ms** | **88 ms** | 199 | 363,003 | 8 |
| Jev 1.14 + screenshots | 66/72 | 91.7% | 1,536 ms | 112 ms | 229 | 1,024,760 | 3 |

Wall time is the median over all attempts, including failures. Decision latency
is the median client-side time of one provider call. Both measure local
browser runs on a shared development machine; neither isolates API latency.
The image path returned **no errors**: **271/271** image requests were
accepted, with no 400 or 413. No variant had errored rows. Only Decisions
refused questions (9 `type_text_target`, 1 `click_target`), which were unused
targets the adapter ignores. The six `needs_confirmation` and three
`needs_input` stops in every variant are expected stops for the gate and
missing-value tasks.

### Paired results

| Comparison (subject vs control) | Both passed | Subject only | Control only | Median paired Δ wall time, both passed |
|---|---:|---:|---:|---:|
| Jev 1.14 + screenshots vs visual Decisions | 66 | 0 | 5 | **−284.5 ms** |
| Jev 1.14 + screenshots vs Jev 1.14 text | 59 | 7 | 0 | +119 ms |
| Jev 1.14 + screenshots vs Jev 1.13 text | 63 | 3 | 3 | +118 ms |
| Jev 1.14 text vs Jev 1.13 text | 59 | 0 | 7 | −14 ms |
| Visual Decisions vs Jev 1.13 text | 65 | 6 | 1 | +395 ms |

Jev 1.14 with screenshots passed no attempt that visual Decisions failed. On
the 66 attempts both passed, it was faster in the median by 284.5 ms.
See [paired.json](paired.json).

## Held-out validation: 4 tasks, three repetitions

| Variant | Passed | Median wall time | Median decision latency | Browser decisions | Input tokens |
|---|---:|---:|---:|---:|---:|
| Visual Decisions | **12/12** | 1,552.5 ms | 212 ms | 27 | 45,876 |
| Jev 1.13 text | **12/12** | 1,126 ms | 99 ms | 27 | 43,344 |
| Jev 1.14 text | **12/12** | **1,085.5 ms** | **84 ms** | 27 | 43,344 |
| Jev 1.14 + screenshots | 9/12 | 1,174 ms | 104.5 ms | 42 | 159,875 |

All three Jev-with-images failures were on the held-out search task. Each
time, it typed the query and pressed Enter correctly. It then chose WAIT six
times on the results page and stopped `blocked` at the decision budget,
instead of answering DONE. The screenshot it received was fresh and showed
the results heading. Text-only 1.14 answered DONE on the same page with
confidence 0.91–0.94. The other three held-out tasks passed 3/3 with images.

## Where behaviour differs

| Task | Visual Decisions | Jev 1.13 | Jev 1.14 | Jev 1.14 + screenshots |
|---|---:|---:|---:|---:|
| `scroll_region` (terms box must be scrolled first) | 3/3 | 0/3 | 0/3 | **3/3** |
| `date_field` | 3/3 | 3/3 | 1/3 | 3/3 |
| `delayed_spa` | 3/3 | 3/3 | 2/3 | 3/3 |
| `frame_form` | 3/3 | 3/3 | 2/3 | 3/3 |
| `icon_by_picture` (unnamed trash icon) | 2/3 | **3/3** | 0/3 | 0/3 |
| `enter_to_search` (no submit button) | **3/3** | 0/3 | 0/3 | 0/3 |
| Held-out search (Enter, then confirm results) | 3/3 | 3/3 | 3/3 | **0/3** |

- **Screenshots help Jev 1.14 where layout matters.** With the picture,
  Jev 1.14 scrolled the terms box and passed `scroll_region` 3/3. Neither
  text-only Jev model passed it: they clicked Cancel or answered BLOCKED. Images
  also recovered 1.14's text-only misses on `date_field`, `delayed_spa` and
  `frame_form`.
- **Jev 1.14 is worse than 1.13 on text.** It lost seven paired attempts to
  1.13 and won none. All three `icon_by_picture` attempts deleted the right
  email and then answered BLOCKED instead of DONE; 1.13 answered DONE. The
  screenshot did not change that. 1.14 text-only also had five model DONE
  answers on unfinished tasks; 1.13 had three.
- **Neither Jev model presses Enter when a decoy is offered.** On
  `enter_to_search`, every Jev variant answered DONE without submitting. Those
  are all the DONE-with-failed-outcome rows for 1.13 and for 1.14 with images.
  Visual Decisions pressed Enter 3/3.
- **Visual Decisions' one miss** was `icon_by_picture` repetition 1. It
  answered BLOCKED after the deletion, like Jev 1.14.

## Latency and cost

| Variant (confirmation) | Median decision latency | p90 decision latency | Median model time per attempt | Median screenshot time per attempt | Input tokens per call | TypeSafe output tokens |
|---|---:|---:|---:|---:|---:|---:|
| Visual Decisions | 229 ms | 295 ms | 642 ms | 70.5 ms | 1,933 | — |
| Jev 1.13 text | 98 ms | 127 ms | 282.5 ms | 0 | 1,826 | 23,515 |
| Jev 1.14 text | 88 ms | 141 ms | 245.5 ms | 0 | 1,824 | 22,379 |
| Jev 1.14 + screenshots | 112 ms | 158 ms | 334 ms | 72 ms | 4,475 | 25,908 |

Adding a 1120×780 screenshot added about **24 ms** to each Jev 1.14 call.
Each call carried about **2,650 more input tokens**, for the image and the
candidate rectangles. Screenshot capture added about **70 ms** per attempt.
Jev with images also used more decisions on this set: 229 vs 199 for 1.14
text-only. Jev's per-call latency remains about half of Decisions'. Visual
Decisions' all-attempt median wall time was **1,862.5 ms**, below the
**2,312 ms** of [October 8](../2026-10-08/README.md); this run's endpoint was
faster.

Totals for the confirmation and held-out runs:
- TypeSafe: **2,015,924 input** and **80,052 output** tokens across the three Jev variants. Jev 1.14 with images used **1,184,635** of those input tokens.
- OpenAI Decisions: **461,538 input** tokens, with 0 output tokens reported.

An eight-attempt smoke check before the run used about 80,000 more tokens.
TypeSafe bills image tokens as ordinary input tokens. Neither provider's price
is recorded here, so these are token counts, not dollar costs. Tokenizers
differ between providers.

## Implementation

`JEV_IMAGES=1` (default off; other values fail) makes the hosted Jev client
request the viewport screenshot. The engine already captures it for visual
Decisions, and secret-field suppression applies. The client fits the
screenshot to TypeSafe's limits: PNG or JPEG, 280 to 1568 px per side,
16 MiB or less. An image already inside the limits is sent byte for byte. A
larger one is shrunk to 1568 px on its longest side, and a smaller one grown
to 280 px on its shortest, both as JPEG. An aspect ratio that cannot fit is
left out, and that decision is made from text.

The image goes out as one `data_base64` asset under a content-derived UUID. It
is referenced once from `state.viewport_screenshot`, alongside candidate
rectangles, and the request carries `TypeSafe-Danger-Zone-Images: true`. With
images off, the request is unchanged. Every screenshot in this run was a
1120×780 JPEG of 12.7 KB median (44.8 KB max), so none was resized.

The eval accepts `jev@<model>` to pin a TypeSafe model per variant, and adds
the `jev-vision` variant. Questions, criteria and the Baseline Jev profile are
the same in the text and image variants. The two image variants (Decisions
and Jev) send the same screenshot and rectangle evidence.

## Evidence and reproduction

- [Protocol](method.md).
- [Confirmation data](confirmation.jsonl.gz) and [summary](confirmation-summary.json).
- [Held-out data](holdout.jsonl.gz) and [summary](holdout-summary.json).
- [Paired outcomes and latency](paired.json).
- [Paid live reproduction commands](reproduce.sh).

The raw rows contain decisions, effects, provider-reported usage, and both
providers' wire requests and responses. That includes the base64 screenshots
of the throwaway local fixtures, so the files are gzip-compressed. No API keys
or authorization headers are recorded. Every attempt is retained; none was
removed or rerun.

```sh
report=evals/reports/decisions-vs-jev/2026-10-16
p=vision,jev@jev-1.13.0,jev@jev-preview,jev-vision@jev-preview
python3 "$report/analyze.py" --tasks 24 --repeats 3 --providers $p "$report/confirmation.jsonl.gz"
python3 "$report/analyze.py" --tasks 4 --repeats 3 --providers $p "$report/holdout.jsonl.gz"
python3 "$report/paired.py" "$report/confirmation.jsonl.gz" jev-vision@jev-preview:vision
```

The analyzer rejects missing attempts, duplicate attempts, unexpected
repetition counts and unexpected provider sets. It also reads the October 8
data, which reproduces that report's held-out figures.

## Validation

- `cargo test -p roder-ext-jev` passed: library **706 passed, 13 ignored**, and all integration and doc tests passed. This includes the new image tests:
  - fitting and clamping (unchanged, shrunk, grown, rejected aspect ratio or format)
  - the asset and UUID reference
  - a fake-server test of the real hosted client and transport covering:
    - the header
    - a single fitted asset referenced from state
    - no header and an unchanged body when images are off
    - an unfittable image left out
    - a 400 stopping without an action
- `cargo clippy -p roder-ext-jev --all-targets --no-deps -- -D warnings` passed.
- `make fmt-check` passed.

## Caveats

- TypeSafe labels images "Dreamer-ready" and a danger-zone feature. The
  request format and limits may change between releases. TypeSafe says models
  do worse on images than on text.
- `jev-preview` answered as `jev-1.14.0` on every call. Its behaviour may
  change before it becomes `jev-latest`.
- The Jev variants use the Baseline profile, as on October 8, to stay
  comparable with that run. Roder's production Jev uses the Workflow
  profile, which was not measured here.
- This run had more decision calls with images (229 vs 199). The
  WAIT-loop failure on the held-out search shows that images can change when
  Jev considers a task complete.
