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
  diagnostic beside that screenshot. Missing screenshots stop continuation;
  they are never substituted with a function result or plain text.
- HTTP replay preserves native calls and pairs their outputs once. WebSocket
  continuation sends `previous_response_id` and only new screenshot output.
  Request-budget shedding cannot replace a native screenshot with text.
- ACP exposes the batch in `rawInput`, retains the native `call_id` as the tool
  id, and reports completed or failed execution. A completed tool call is an
  execution result; the model must still observe and verify its goal.
- Existing policy modes apply. Plan mode denies input; screenshot and wait are
  read-only. The eval uses explicit Bypass mode, as requested. No additional
  sensitive-action consent layer was added.

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
modifiers were null, passed the independent submission grader, ended ACP with
`end_turn`, and left `Filters open` and `Submitted: orcaA` visible. The actual
800 × 513 screenshot was visually inspected; its filled password field is
masked. The live task exercised click, keypress, type and screenshot.

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
```

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
browser events and the submitted value. The default model is `gpt-6.1-sol`;
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
| OpenAI-only tool advertisement | [`tool_advertisement.rs`](../crates/roder-core/src/tool_advertisement.rs) |
| Native wire adapter | [`responses/src/computer.rs`](../crates/roder-ext-openai-responses/src/computer.rs), [`tool_definitions.rs`](../crates/roder-ext-openai-responses/src/tool_definitions.rs), [`response_replay.rs`](../crates/roder-ext-openai-responses/src/response_replay.rs) |
| Retained executor and page binding | [`chrome/src/computer.rs`](../crates/roder-ext-chrome/src/computer.rs) |
| Ordered input and capture | [`direct/computer.rs`](../crates/roder-ext-chrome/src/direct/computer.rs) |
| Full ACP protocol/browser eval | [`acp_native_computer.rs`](../crates/roder-app-server/tests/acp_native_computer.rs) |
| Live OpenAI runner | [`native_computer_eval.rs`](../crates/roder-app-server/examples/native_computer_eval.rs) |

## Release validation corrections

The Chrome extension declares its native `computer` contributor when installed.
`computer` is reserved for the native browser binding; host-supplied external
function tools must use a different name. Cleanup attempts every held input and
mask independently, retains failed releases, and blocks subsequent input until
recovery succeeds. Desktop origin ceilings also disable arbitrary `chrome_eval`;
use the scoped input tools when an origin ceiling is configured.
