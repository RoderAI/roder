# Native OpenAI computer use

Roder supports the Responses API's native `computer` tool through the OpenAI
provider. The Chrome extension bridge, browser-use MCP, Jev and Codex's installed
CUA SDK are separate tool interfaces. Their evaluations do not establish native
Responses protocol support.

## Wire contract

The provider advertises exactly:

```json
{"tools":[{"type":"computer"}],"parallel_tool_calls":false}
```

A completed `computer_call` contains `call_id` and an ordered `actions` array.
Roder executes that batch once and sends a fresh screenshot with the same id:

```json
{
  "type": "computer_call_output",
  "call_id": "cu_example",
  "output": {
    "type": "computer_screenshot",
    "image_url": "data:image/jpeg;base64,...",
    "detail": "original"
  }
}
```

Supported actions: `click`, `double_click`, `drag`, `move`, `scroll`, `keypress`,
`type`, `wait`, and `screenshot`. Native mouse button `wheel` maps to Chrome's
middle button. Drag follows every supplied `{x,y}` point. Mouse modifiers and
their native nullable representation (`keys: null`) are supported. Mouse input and
keyboard chords use physical CDP key events; macOS editing shortcuts also send
Chrome editing commands. `type` appends at the current caret; replacement
requires selecting the existing text. `wait` pauses for two seconds.

A Mac page ignores Control+A: only Command chords carry the editing commands, so
text typed after it lands after the old text. When the page reports a Mac
platform and focus is in an editable field, Control+A, C, V, X and Z are sent as
Command+A, C, V, X and Z, and Control+Shift+Z and Control+Y as Command+Shift+Z.
The result's notes say so (`Ctrl+A was sent as Cmd+A on macOS`), once the key
step has come back without error: an action that failed does not also claim its
chord as sent. Other chords, non-Mac pages and non-editable focus are sent as
given. The remap does not suit a page that binds Control+A to an Emacs-style
action.

This implements the current batched contract. See the
[OpenAI computer-use guide](https://developers.openai.com/api/docs/guides/tools-computer-use)
and [integration recipes](https://developers.openai.com/api/docs/guides/tools-computer-use-integration).

## Enable a browser

Supply a running Chrome browser DevTools endpoint and an initial page URL:

```sh
export RODER_COMPUTER_USE_CDP_URL=http://127.0.0.1:9222
export RODER_COMPUTER_USE_URL=https://example.com
```

Use Roder's `openai` provider with an OpenAI API key and a model supporting this
tool. ChromeExtension registers the native contributor when the endpoint is
configured. It requires the initial URL because screenshots contain the page
viewport, including neither browser tabs nor the address bar. Optional
`RODER_DESKTOP_ALLOWED_ORIGINS` supplies the existing origin ceiling. See
[`desktop_scope.rs`](../crates/roder-ext-chrome/src/desktop_scope.rs) for its
configuration format.

Hosts embedding Roder can instead register
`ComputerToolContributor::new(Arc::new(ComputerCdpBinding::new(endpoint, url)))`
or supply their own `DirectBinding` to the contributor. The native API schema
and action types are exported from `roder_api::computer`.

The runtime advertises `computer` only to `openai`. The provider also rejects
native history routed to another provider. A live probe of the signed-in Codex
endpoint returned HTTP 400, `Unsupported tool type: computer`; Codex account
authentication cannot run this eval.

## Screenshots from the other browser and desktop tools

`chrome_*`, `jev_tab_*`, `browser_use_*`, `cua_*` and `view_image` return a
screenshot under `__view_image` in the tool result, and most of their text says
it is attached. Only an engine whose `InferenceEngine::tool_result_image_input(model)`
is true puts that image in front of the model: the OpenAI Responses family
(OpenAI, Codex, OpenRouter, Fireworks, custom Responses providers, and xAI models
the catalog says take images), Anthropic and Gemini. The default is false.

For any other engine the runtime sends the request without the image and ends
that result with one line, `screenshot not shown: this model cannot receive
images in tool results`. It does not offer the tools whose only product is an
image (`chrome_screenshot`, `browser_use_screenshot`, `jev_tab_screenshot`,
`view_image`), and it charges no image tokens when it estimates the prompt for
compaction. `computer` keeps its own rule above.

The stored transcript is not changed, so a thread moved to an engine that can see
the image shows it again, and history that called a tool the current engine is
no longer offered still replays. The browser tools still request a screenshot on
every action; skipping that request on these engines is not done yet.

## Execution and continuation

- A browser binding owns one page target per Roder thread. Switching the user's
  active tab cannot retarget a running batch. Each batch and its screenshot are
  serialized under the same lease. Targets share the supplied browser profile
  and cookies; supply an isolated browser when profile isolation is required.
- Coordinates use viewport CSS pixels. Capture dimensions preserve that mapping
  even at device pixel ratio 2. Empty screenshot data is an error.
- The retained direct session releases held input and removes screenshot masks
  after errors or cancellation. Its next call waits for recovery. Cleanup is
  best effort if Chrome disconnects or the process exits.
- Malformed action shapes send no input. A later action failure stops the batch
  and returns a fresh screenshot of partial progress. The provider includes a
  diagnostic beside that screenshot. The Chrome executor never returns a call
  without a picture: when none can be taken (the page shows a password the call
  typed, Chrome returned an empty capture, the cleanup before the call failed,
  or no browser could be reached) the result carries a fixed 288 × 96 PNG
  reading `SCREENSHOT UNAVAILABLE`, `screenshot_unavailable: true`, and a note
  giving the reason. When only the picture is missing the call is not an
  error: the actions ran. A result without any image from another source still
  stops continuation; it is never substituted with a function result or plain
  text.
- A navigation or a new tab ends the batch when an input action follows it.
  The actions after it were planned for a page that is gone, so they are not
  run. The notes name the input actions among them (their kind and position,
  never the text they would type); a `wait` or `screenshot` skipped with them
  is not named, and `unrun_actions` counts input actions only, so
  `requested_actions` minus `completed_actions` can be larger. Stopping early
  is not an error. A navigation followed only by `wait` or `screenshot`
  actions does not stop the batch: they run.
- HTTP replay preserves native calls and pairs their outputs once. WebSocket
  continuation sends `previous_response_id` and only new screenshot output.
  Request-budget shedding cannot replace a native screenshot with text.
- ACP exposes the batch in `rawInput`, retains the native `call_id` as the tool
  id, and reports completed or failed execution. A completed tool call is an
  execution result; the model must still observe and verify its goal.
- Existing policy modes apply. Plan mode denies input; screenshot and wait are
  read-only. The eval uses explicit Bypass mode, as requested. No additional
  sensitive-action consent layer was added.

### Result notes

A result that holds anything notable carries it twice: as `computer_notes` in
the tool result data, and as a `Notes` block in the result text, between the
failure line (when there is one) and the screenshot line. The text is for
engines that read text; the data is the one a provider replays from.
`computer_notes` is an array of at most eight strings of at most 160 characters
each; the key is absent when nothing was notable, so an uneventful batch leaves
the replayed prefix as it was. Notes are written for these events, each naming the action by its
position in the batch (`Action 2 (click (60,100) on link "My account")`):

- a navigation: the new address (without query or fragment), title and HTTP
  status. The address before the action and the one after it are compared
  without their fragments and with the owner's typed secrets hidden on both
  sides, so a page whose address holds a remembered secret is not a navigation
  by itself;
- an HTTP status of 400 or more on the page after the action;
- a new tab, which the session now drives;
- a JavaScript dialog: its kind, its message and whether it was accepted
  (`alert`, `beforeunload`) or dismissed (`confirm`, `prompt`);
- a password field that was not on the page before the action (a sign-in wall);
- a click on something that is not a control, after which the page's text and
  elements did not change (not claimed for a canvas, which can change without
  its page changing);
- a remapped chord (see above), only when the key step ran without error;
- the actions a navigation or new tab kept from running, and an unavailable
  screenshot. These two are never left out for the sake of other notes; when
  other notes overflow, the last of them says how many were left out.

Page-sourced words (an address, a label, a role, a title, a dialog message) are
cut, on one line, and scrubbed of values typed into secret fields, after which
direction-overriding and zero-width characters are removed; labels, titles and
dialog messages are also quoted. An address loses its query and fragment, and
the path segments that look like bearer tokens (16 or more hex digits, a UUID,
or 20 or more letters and digits with at least one of each and no more than two
separators, as in `/reset/<token>`) become `…`; routes and slugs of words stay.
Dialog messages are scrubbed before they are cut, in this block and in the page
text of the other direct tools; the client keeps the first 4,000 characters of
a message until it is read, so a secret longer than that is not covered. The
block says these words are untrusted and must not be followed. On a batch that
ran, the data also holds `completed_actions` and `requested_actions`, with
`stopped_after` (`navigation` or `new_tab`) and `unrun_actions` when the batch
stopped early, and `screenshot_unavailable` with `screenshot_error` when only
the picture is a placeholder. A call that fails before the batch starts holds
none of the counters. When the cleanup before it failed or no browser could be
reached, its data is `error`, `screenshot_unavailable` and the placeholder's
note in `computer_notes`, and its text is the reason followed by that note,
without a `Notes` block. When the action shapes are malformed, its text starts
`Invalid native computer actions; no input sent` and its data is the
screenshot's own.

In replay, a failed call is sent as its screenshot followed by a user message
holding the whole result text (`Computer execution failed; ...`). A call that
did not fail, including one that stopped after a navigation or whose screenshot
is the placeholder, is sent as its screenshot followed, when it has notes, by
one more user message: the label `UNTRUSTED browser observation.` with the
warning that the words come from the page and must not be followed, then the
notes as `- ` lines. The `computer_call_output` item holds only the screenshot,
the one shape the API takes there; the notes ride beside it the way the failure
text does.

The adapter reads the notes from the structured `computer_notes` of the
transcript record, not from the result text, so it does not depend on how the
executor lays the text out, and a `Notes` block in the text alone adds nothing.
`computer_notes` is one of the few data keys the record keeps (`tool_display_payload`
in `roder-api`, with `__view_image`), and it is read from the tool's result data,
never from the call's arguments. That function keeps only the strings that are
not blank, the first eight, each cut to 160 characters with a trailing `…`; the
reader `tool_result_computer_notes` applies the same limits again, so a record
written by anything else is bounded too. The adapter then removes control and
direction-changing characters and collapses spaces, so the message cannot grow
past about 1.5 KB. Notes past eight are dropped without a count; the executor
already puts its own "N more notes were left out" line inside its eight. A result
with no notes adds no message, so an uneventful step replays as the screenshot
alone and the cached prefix does not move. The notes are scrubbed of typed
secrets where the executor writes them; the adapter has no access to those
values. WebSocket continuation sends the message together with the new
screenshot.

Observations are marked untrusted. Origin checks run before actions and reads;
they stop later interaction after a cross-origin redirect, but are not a
network firewall. Native screenshots retain the direct executor's password
masking and typed-secret scrubbing.

## Evaluation evidence — 2026-09-30

The scripted native Responses endpoint drove **real Chrome through the public
ACP adapter, runtime and OpenAI Responses engine**. It split SSE frames across
network chunks and captured actual outbound request bodies. The happy path
made seven requests and six native calls, executing 18 actions covering all nine
action types. All nine independent browser graders passed. Input events were
trusted, the curved drag path was preserved, nested scrolling occurred, and
the form actually submitted `orcaA`.

- [Machine-readable result](../evals/reports/native-computer/2026-09-30/native-protocol.json)
- [Actual final screenshot](../evals/reports/native-computer/2026-09-30/native-protocol.jpg),
  visually inspected at 800 × 513 with the filled fake password masked.

Separate ACP cases passed for partial batch failure and malformed input. The
real-browser cancellation test cancels a chord and a masked screenshot after
Chrome applies the operation, injects a mid-drag failure, then immediately
reuses the same executor and checks that held input and masks are gone. Native
provider tests cover registration, parsing, replay, missing screenshots,
malformed protocol items, budgeting, and a two-round WebSocket transport.

The full workspace run with `--features e2e-tests -- --test-threads=1` passed
3,827 tests in 386 suites, with zero failures and 67 intentionally ignored
tests. It used a temporary Roder configuration and required real Chrome for
browser tests. See [validation summary](../evals/reports/native-computer/2026-09-30/validation.json).

The scripted runs verify local protocol and primitive correctness. A separate
live OpenAI run now validates native API acceptance and the model's basic UI
task through the same ACP/runtime path. It also reads the final browser page
independently and saves a masked screenshot. See the
[live result](../evals/reports/native-computer/2026-09-30/live-openai/report.json)
and [final screenshot](../evals/reports/native-computer/2026-09-30/live-openai/final.jpg).
This single fixture is not a general model task-success benchmark; all-nine
primitive coverage comes from the scripted real-browser evaluation.

The final live run used `gpt-6.1-sol`: **five native calls, 20 actions, zero
failed calls**, completing in 74.8 seconds. It executed two mouse actions whose
modifiers were null, passed the submission grader of that date (see the
correction below), ended ACP with `end_turn`, and left `Filters open` and
`Submitted: orcaA` visible. The actual 800 × 513 screenshot was visually
inspected; its filled password field is masked. The live task exercised click,
keypress, type and screenshot.

**Grader correction (2026-10-09).** That grader passed a run when any trusted
`submit` event carried `orcaA`. The saved live report records four trusted
submits: `penguinorcaA`, `penguinorcaAorcaA`, `penguinorcaAorcaAorcaA`, then
`orcaA`. Three were wrong, and the page text only shows the last one. The grader
now requires exactly one trusted `submit` event, and that event must carry
`orcaA`. Extra, repeated or wrong trusted submits fail the run even when a later
submit is right; events that are not trusted are not counted. By that rule the
saved live run fails. Its `passed: true` is kept unchanged as a record of the
older rule, so read the 74.8 second run above as a wrong-text-then-repair run,
not a clean pass. The scripted and visible-Chrome reports each hold exactly one
trusted submit and still pass. A test regrades all three saved reports without
Chrome or a network. New reports add a `trusted_submits` count next to
`checks`.

The key was retrieved from the OpenAI secret's owning app in the Vex
organization using `com secrets get`. It was held in memory and passed through
the eval process environment, with no key in commands, reports, or repository
configuration. Org listing includes app secrets; reading this particular
secret requires its app scope.

Live execution exposed two runner bootstrap requirements already handled by
the CLI: selecting a Rustls crypto provider and using larger worker stacks.
It also exposed native mouse calls containing `keys: null`; the API type now
accepts that representation, and the real-browser ACP fixture exercises it.

### Reproduce local protocol and browser checks

Run from the repository root with Chrome installed:

```sh
RODER_REQUIRE_CHROME=1 mise exec -- cargo test -p roder-app-server \
  --features e2e-tests --test acp native_computer -- --test-threads=1
RODER_REQUIRE_CHROME=1 mise exec -- cargo test -p roder-ext-chrome \
  --test native_cancellation -- --test-threads=1
mise exec -- cargo test -p roder-ext-openai-responses --lib computer
mise exec -- cargo test -p roder-ext-chrome --test native_grader
RODER_REQUIRE_CHROME=1 mise exec -- cargo test -p roder-ext-chrome \
  --test native_notes
```

`native_grader` needs no Chrome. It regrades the saved reports and synthetic
event lists with the same `grade_events` function the browser evals use.
`native_notes` runs batches through headless Chrome against local pages: the
failing Control+A, type, Shift+A, Enter batch on a page that reports a Mac
platform (one submit, `orcaA`, where the same batch submitted `penguin 🐧orcaA`
before the remap) and on one that does not (sent as Control); a hub page with a
500, a sign-in page, a `target=_blank` link and a `confirm` dialog, each named
in the result; the stop after a navigation, and no stop for a page whose
address holds a remembered secret; a remapped chord that is not claimed when its
action failed; and the placeholder screenshot. The Mac platform is reported with `Emulation.setUserAgentOverride`, so the
tests do not depend on the host operating system.

Set `RODER_NATIVE_EVAL_REPORT` to a JSON file path during the happy ACP test
to save the JSON and adjacent JPEG evidence; create its parent directory first
and filter the test to `native_computer_protocol_all_primitives` to avoid other
cases overwriting it. Override `RODER_CHROME_BINARY` when Chrome
is installed elsewhere. Browser tests fail when Chrome is absent if
`RODER_REQUIRE_CHROME=1`; otherwise they explicitly skip it.

### Run the actual OpenAI model eval

Configure `OPENAI_API_KEY` or the OpenAI provider's key in Roder configuration,
then run:

```sh
RODER_REQUIRE_CHROME=1 mise exec -- cargo run -p roder-app-server \
  --example native_computer_eval
```

This starts an isolated Chrome profile and local fixture, registers only the
native tool, sends the task to `https://api.openai.com/v1`, and grades actual
browser events: a trusted click on Show filters and exactly one trusted form
submit, carrying `orcaA`. Any other trusted submit fails the run, even if a
later one is correct. The default model is `gpt-6.1-sol`;
`RODER_NATIVE_EVAL_MODEL` changes it. Output defaults to
`evals/reports/native-computer/live/report.json`; override its directory with
`RODER_NATIVE_EVAL_OUTPUT`. The runner fails unless a native call occurred,
ACP ended the turn, the independent fixture grader passed, and the final page
still displays the required state. It preserves native call order in the saved
trace and records failed calls. Its startup
requires a key before opening Chrome and does not print the key.

To watch the same native API loop in a visible Chrome window:

```sh
RODER_NATIVE_EVAL_VISIBLE=1 RODER_REQUIRE_CHROME=1 mise exec -- cargo run \
  -p roder-app-server --example native_computer_eval
```

This starts a separate Chrome profile, brings its window to the front, and waits
ten seconds before starting the model. It navigates from a demo start page by
clicking a link and leaves the final page open for ten minutes. Set
`RODER_NATIVE_EVAL_HOLD_SECONDS` to change that inspection period (0–3600).

## Source locations

| Responsibility | Source |
| --- | --- |
| Typed actions and dispatch schema | [`roder-api/src/computer.rs`](../crates/roder-api/src/computer.rs) |
| Notes kept in the transcript record | [`roder-api/src/transcript.rs`](../crates/roder-api/src/transcript.rs) |
| OpenAI-only tool advertisement | [`tool_advertisement.rs`](../crates/roder-core/src/tool_advertisement.rs) |
| Native wire adapter | [`responses/src/computer.rs`](../crates/roder-ext-openai-responses/src/computer.rs), [`tool_definitions.rs`](../crates/roder-ext-openai-responses/src/tool_definitions.rs), [`response_replay.rs`](../crates/roder-ext-openai-responses/src/response_replay.rs) |
| Retained executor and page binding | [`chrome/src/computer.rs`](../crates/roder-ext-chrome/src/computer.rs) |
| Ordered input and capture | [`direct/computer.rs`](../crates/roder-ext-chrome/src/direct/computer.rs) |
| Result notes and the stop after a navigation | [`direct/computer_notes.rs`](../crates/roder-ext-chrome/src/direct/computer_notes.rs) |
| Mac editing chords | [`direct/mac.rs`](../crates/roder-ext-chrome/src/direct/mac.rs), [`direct/keys.rs`](../crates/roder-ext-chrome/src/direct/keys.rs) |
| Placeholder screenshot | [`direct/unavailable.rs`](../crates/roder-ext-chrome/src/direct/unavailable.rs) |
| Full ACP protocol/browser eval | [`acp_native_computer.rs`](../crates/roder-app-server/tests/acp_native_computer.rs) |
| Live OpenAI runner | [`native_computer_eval.rs`](../crates/roder-app-server/examples/native_computer_eval.rs) |

## Release validation corrections

The Chrome extension declares its native `computer` contributor when installed.
`computer` is reserved for the native browser binding; host-supplied external
function tools must use a different name. Cleanup attempts every held input and
mask independently, retains failed releases, and blocks subsequent input until
recovery succeeds. Desktop origin ceilings also disable arbitrary `chrome_eval`;
use the scoped input tools when an origin ceiling is configured.
