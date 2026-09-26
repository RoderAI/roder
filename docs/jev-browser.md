# Jev browser tool

`jev_browse` is a Roder tool provider that runs a bounded browser goal against
Chrome over CDP. It returns the executed action trace and observed final page
through Roder's normal tool result. The page content is untrusted; a `done`
result is an agent claim that should be checked against the observed page.

The implementation is a Rust port of
[Jev Ultrafast](https://github.com/browser-use/jev-ultrafast) (MIT), pinned to
revision `1231850a`, and runs inside the Roder binary: no Python, no `uv`, and
no `browser_harness` daemon.

Set `JEV_API_KEY` in the Roder process environment. This is passed to upstream
Jev as `TYPESAFE_API_KEY`. Alternatively, configure provider `jev` through
`providers/configure` or `[providers.jev] api_key` in Roder config. Roder never
passes the key as a command argument or writes it into tool output.

```json
{"jsonrpc":"2.0","id":1,"method":"providers/configure","params":{"provider":"jev","api_key":"<jev-key>"}}
```

`JEV_MODEL` selects the TypeSafe decision model and defaults to `jev-latest`.

## The browser

The task runs in a foreground tab by default: Roder activates it so the work is
visible, and leaves the final page open so the result can be checked. Pass
`foreground: false` for a background tab that is closed when the task ends.

Roder resolves the Chrome DevTools endpoint itself:

1. `BU_CDP_URL` or `BU_CDP_WS`, when set, is passed through untouched — use this
   to attach to a specific or remote CDP browser.
2. Otherwise Roder reuses a DevTools endpoint already listening on
   `127.0.0.1:9222` (`JEV_CDP_PORT` overrides the port).
3. Otherwise Roder starts a visible Chrome on that port with its own profile in
   `<config-dir>/jev-chrome`, and waits for a debuggable page before starting
   the task. That browser is left running for later tasks, so its window and
   signed-in sessions persist.

A `wait` action holds the page for `JEV_WAIT_MS` (default 800, clamped to
100-10000) before the next observation. Upstream sleeps 100ms, which is shorter
than most applications take to answer a click, so a delayed reply — an
opponent's move, a spinner, a debounced search — would otherwise be observed as
"nothing changed".

`JEV_CHROME_BINARY` selects the browser executable; `JEV_CHROME_AUTOSTART=0`
turns auto-start off, so a missing endpoint becomes an error instead. The tool
result reports what happened under `browser`: `cdp_url`, `foreground`, and
`launched_by_roder`.

## How the port is organised

| Module | Ported from | Responsibility |
| --- | --- | --- |
| `cdp.rs` | `browser_harness.cdp` | One websocket to Chrome, flat sessions, request/response by id |
| `page.rs` | `browser.py` | Open the tab, observe, hit-test and dispatch input, fingerprint |
| `space.rs` | `model.action_space` | The indexed action space |
| `decide.rs` | `model.choose` | The TypeSafe request body, validation, retries |
| `text_helper.rs` | `model.field_text` | The field-value request and its strict reply contract |
| `agent.rs` | `agent.py` | The tick loop, budgets, statuses, stall detection |
| `prompts.rs` | `questions.py` | Instruction text, vendored verbatim |

Two scripts must run inside the page and stay JavaScript, vendored verbatim
under `src/assets/`: `snapshot.js` (the observation) and `act.js` (hit-testing
an observed node before input), plus `settle.js` from upstream's post-input
wait.

### Parity with upstream

`tests/fixtures/` holds output recorded from the upstream Python, and the unit
tests assert the port reproduces it: the action space including key and target
order, the decision request body byte for byte under compact serialization, all
ten `validate_choice` verdicts, the text-helper body for every provider
reasoning shape, `field_context`, and the observation fingerprint — the same
SHA-256, produced through a CPython-compatible `json.dumps`.

Regenerate the fixtures against a new upstream revision with:

```bash
uv run --no-project --with "jev-ultrafast @ git+https://github.com/browser-use/jev-ultrafast.git@<rev>" python crates/roder-ext-jev/tests/generate_fixtures.py
```

Failing parity tests after that are the point: they say exactly what upstream
changed.

## Typing: the text model comes from Roder

Jev asks a small OpenAI-compatible chat-completions model for the value of any
field it types into. Roder serves that from its own harness and resolves it in
this order:

1. `JEV_TEXT_MODEL_API_KEY` (or `OPENROUTER_API_KEY`), with
   `JEV_TEXT_MODEL_BASE_URL` and `JEV_TEXT_MODEL`, when set explicitly.
2. The model of the turn that called the tool, when its provider speaks
   chat-completions and Roder holds its key — so a typing task uses the model
   you are already running.
3. The first configured chat-completions provider Roder has a key for:
   `deepseek`, `synthetic`, `openrouter`, `xai`, `fireworks`, `openai`.

`JEV_TEXT_MODEL_REASONING` overrides the reasoning knob Jev sends. The tool
result reports the choice under `text_model` as `{model, source}`, where source
is `explicit`, `turn-model` or `roder-provider`.

Providers on native non-OpenAI transports cannot serve this helper: OAuth
harnesses (`claude-code`, `supergrok`, `codex`), Anthropic and Gemini's own
APIs, and Cursor, whose provider path is a protobuf AgentService rather than
chat completions. With none available, `text_model` is `null` and a task that
needs to type fails upstream rather than guessing a value; goals that only
click and read are unaffected.

## Policy and arguments

In Roder's default policy mode, each goal needs approval before Jev starts.
Plan mode denies it. The tool accepts `url`, `goal`, optional `foreground`
(default true) and optional `timeout_seconds` (1 to 300, default 120). It
returns `status`, final `url`, `title`, visible text, executed actions, elapsed
time, model call counts, `observed_elements` (how many targets Jev could see on
the final page), `stopped_because` (set when a run stopped early), and the
`browser` and `text_model` provenance above.

This tool is registered with Roder's inference runtime. The Codex app-server
backend currently runs Codex's own tool set and does not advertise Roder tool
providers through Codex dynamic tools.

## What Jev can and cannot reach

Jev builds its action space from one selector (`snapshot.js`):

```text
a[href], button, input, textarea, select, summary, [contenteditable="true"],
[role="button"|"link"|"checkbox"|"radio"|"switch"|"tab"|"menuitem"|
 "menuitemradio"|"option"|"gridcell"|"combobox"|"textbox"|"searchbox"|
 "spinbutton"]
```

An element also has to survive `checkVisibility({checkOpacity, checkVisibilityCSS})`,
not be `:disabled`, `aria-hidden` or `inert`, and have its centre point inside
Jev's emulated 1120x780 viewport. So three common cases are simply unreachable,
and all three report `observed_elements` low or zero rather than an error:

- Controls built from bare `<div>` or `<td>` with no role or label. Most public
  tic-tac-toe boards are like this.
- Visually hidden but focusable controls, such as a board of screen-reader
  checkboxes drawn with `opacity: 0`.
- Anything below the fold until Jev scrolls to it.

When a page exposes nothing Jev can target, the tool says so and names the
selector. Report that and pick another page or another browser tool — do not
build a local page to make a goal pass, which hides the limitation instead of
recording it. `jev_browse` itself imposes no restriction on which sites may be
visited beyond the policy approval.

Upstream's README lists frames, shadow roots, canvas, uploads, pop-up tabs
and nested scrolling as out of scope. Two more behaviours worth knowing:

- Jev observes the page as soon as it loads, so a page whose content is still
  being fetched can return `blocked` on the first attempt and succeed on a
  retry.
- The run stops at 60 executed actions or 120 model calls, upstream's budgets.
  Roder still returns the observed trace with `stopped_because` set, because the
  partial trace is the useful part.
- Jev decides for itself when to wait, and mostly prefers clicking. On a page
  that ignores input while it is busy — a game during the opponent's turn — it
  can keep clicking, see no change, and then choose a control that looks like
  progress. A move-history list is the worst case: clicking an entry rewinds the
  page, so the run loops until the action budget. Raising `JEV_WAIT_MS` buys
  more consecutive successful actions but does not change that choice, which
  belongs to the upstream decision model.
