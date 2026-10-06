# Decisions validation — 2026-10-06

## Proven locally

`mise exec -- cargo test -p roder-ext-jev --lib decisions -- --test-threads=1`
passed 6 tests; 2 live tests were ignored as intended.

Coverage: Decisions HTTP request/response mapping, malformed response rejection,
usage retention on rejected responses, irreversible predicates, credential-error
redaction, bounded screenshot choice with a blocked outcome, and real Chrome
native computer execution. The native test captured a screenshot, supplied a
scripted Decisions reply, executed the selected coordinate click through
`DirectSession::run_computer`, and verified `window.canvasOk === true`.
The model reply in this test was mocked; it does not prove model quality.

Distribution metadata now names `JEV_DECISION_PROVIDER` and `OPENAI_API_KEY`,
and the OpenAI secret capability matches the extension manifest. Release config
and whitespace checks passed.

## Not yet proven

The explicit live run of both `decisions_live_browser_corpus` and
`decisions_live_computer_canvas` failed at credential preflight. Neither the
process environment nor Roder's OpenAI provider configuration supplied an API
key. No authenticated Decisions request was made in this validation run.

Completion requires running both live tests with a configured OpenAI API key,
checking the browser JSONL outcome report and the native canvas success marker,
and addressing any API-contract or model-behavior failures they reveal.
See [setup and commands](../openai-decisions-browser.md#live-validation).
