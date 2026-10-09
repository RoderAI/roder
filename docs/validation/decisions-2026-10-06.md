# Decisions validation — 2026-10-06

## Live API results

Both live tests passed against OpenAI's authenticated Decisions endpoint using
`gpt-6-luna`, on throwaway local Chrome fixture pages. The final combined run
completed in 22.38 seconds. No fallback model was used.

| Scenario | Outcome | Browser actions | Decisions calls |
|---|---|---:|---:|
| Contact form | Submitted; expected form POST and thank-you page verified | 4 | 5 |
| Dropdown | Selected and submitted; expected order POST verified | 2 | 3 |
| Below the fold | Observed target clicked; completion verified | 1 | 2 |
| Payment gate | Stopped for confirmation before executing payment | 0 | 1 |
| Native computer canvas | Screenshot selected the correct coordinate click; `run_computer` executed it and `window.canvasOk` became true | 1 | 1 |

The screenshot request reported 1,128 input tokens, zero output tokens and
confidence 1.0. The browser corpus uses fixture-provided text values to isolate
decision quality; it does not validate text generation. The native computer
test validates selection among two host-supplied clicks plus `blocked`, not
unrestricted generation of coordinates or arbitrary desktop control.

Evidence: [test output](decisions-2026-10-06-live.txt) and
[browser outcome rows](decisions-2026-10-06-live.jsonl).

## Issues found and fixed

Live requests revealed that Decisions rejects choice questions containing only
one option (HTTP 400, `array_below_min_length`). The adapter now omits those
target questions and resolves the sole target locally after the model selects
the operation. A regression test checks both the request and action mapping.

One earlier contact-form attempt returned an unsupported answer type and stopped
without executing that decision. A focused retry and the final combined suite
passed. Unsupported responses still fail closed; this small successful sample
does not establish a production reliability rate.

## Local checks

The focused Decisions suite passed seven tests, including HTTP contract,
malformed response rejection, billed usage retention, irreversible predicates,
credential-error redaction, bounded screenshot selection with a blocked outcome,
single-target handling, and real native computer execution with a mocked reply.
Live tests are separate and require explicit opt-in. Release metadata and
whitespace checks pass.

The key was retrieved from the Roder app's Composal secrets and configured in
`~/.zshrc` with mode 0600. Its value is absent from this report and the repository.
See [setup and commands](../openai-decisions-browser.md#live-validation).
