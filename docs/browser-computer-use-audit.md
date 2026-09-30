# Browser computer-use audit — 2026-09-29

Status: **audit complete; repairs committed locally**. Runtime evaluations
and all workspace packages passed in the clean runs described below. This
report distinguishes primitive correctness, model task outcomes, profile
isolation limits and intermittent full-run failures.

Native Responses follow-up — 2026-09-30: the OpenAI `computer` adapter now
advertises the native tool, executes ordered action batches, and returns matching
`computer_call_output` screenshots. The new ACP/real-Chrome protocol eval covers
all nine action types. Its decision endpoint is scripted; a live OpenAI model
run still needs an API key. See [native computer use](native-computer-use.md)
for implementation locations, configuration and reproducible evidence.

**User scope:** additional sensitive-action consent was explicitly excluded on
2026-09-29. Existing permissions and approval modes remain in place. Consent
parity is not claimed or included in the remaining implementation gates.

Roder base: `a395cf0d49122fc1f6d76f15fc2c8ca2cadb28a4`.
Branch: `pz/browser-computer-use-audit` in the attached audit worktree.

## OpenAI requirements

OpenAI supports code execution and existing UI function/MCP interfaces. Existing
browser tools can keep their interface; adopting the native `computer` protocol
is optional. The runtime must preserve its actual browser session, return fresh
observations, preserve screenshot resolution or map scaled coordinates, apply
permission rules, bound execution, support cancellation, and verify UI outcomes.
[Computer-use guide](https://developers.openai.com/api/docs/guides/tools-computer-use)

Its safety controls include an isolated environment with site/action allowlists,
untrusted page content, and confirmation before consequential actions—including
typing sensitive data. These controls need enforcement in the application.
[Run safely](https://developers.openai.com/api/docs/guides/tools-computer-use#run-safely)

## Tool inventory and implementation locations

| Provider | Tools | Implementation |
| --- | --- | --- |
| Chrome bridge | `chrome_tabs_list`, `chrome_tab_open`, `chrome_tab_activate`, `chrome_tab_close`, `chrome_tabs_group`, `chrome_navigate`, `chrome_page_snapshot`, `chrome_page_text`, `chrome_highlight`, `chrome_select`, `chrome_debug_attach`, `chrome_debug_detach`, `chrome_screenshot`, `chrome_click`, `chrome_type`, `chrome_keypress`, `chrome_scroll`, `chrome_console_read`, `chrome_network_read`, `chrome_eval`, `chrome_recording_start`, `chrome_recording_stop` | [`tools.rs`](../crates/roder-ext-chrome/src/tools.rs), [`session.rs`](../crates/roder-ext-chrome/src/session.rs); controller contract in `roder-api/src/chrome.rs`. Browser-extension implementation is in the separate `/Users/pz/w/roder-web-extention` repository. |
| Desktop fallback | Chrome tool names; supported navigation, tab list, snapshot, screenshot, eval, click, type, scroll, keypress | [`desktop_cdp.rs`](../crates/roder-ext-chrome/src/desktop_cdp.rs) delegates UI operations to the shared direct tool implementation. |
| Direct CDP | `look`, `screenshot`, `click`, `hover`, `drag`, `type`, `key`, `scroll`, `select`, `navigate`, `wait`; exposed to Jev fallback as `jev_tab_*` | [`direct/`](../crates/roder-ext-chrome/src/direct/mod.rs); persistent tab binding supplied by the owner. |
| browser-use MCP | `browser_use_navigate`, `browser_use_get_state`, `browser_use_click`, `browser_use_type`, `browser_use_extract_content`, `browser_use_get_html`, `browser_use_screenshot`, `browser_use_scroll`, `browser_use_go_back`, `browser_use_list_tabs`, `browser_use_switch_tab`, `browser_use_close_tab`, `browser_use_agent`, `browser_use_list_sessions`, `browser_use_close_session`, `browser_use_close_all` | [`catalog.rs`](../crates/roder-ext-browser-use/src/catalog.rs), [`server.rs`](../crates/roder-ext-browser-use/src/server.rs); external pinned `browser-use[cli]==0.13.10` server started through uvx. |
| Jev | `jev_browse` and its session-bound fallback | [`tools.rs`](../crates/roder-ext-jev/src/tools.rs), `session/`, `runner/`, `fallback/`; external decision/text models, local CDP execution. |
| OpenAI model replay | function calls/results and screenshot input blocks | [`response_replay.rs`](../crates/roder-ext-openai-responses/src/response_replay.rs), [`request_budget.rs`](../crates/roder-ext-openai-responses/src/request_budget.rs). |

### Codex implementations

`/Users/pz/w/codex` is on upstream **main**, not master, at
`90abcfac02665ad882853a04155591cd863b2ca7` after the requested fast-forward pull
and a final refresh (the initial pull was `b1e72963c3b71a9265a551e54beff078384efed9`).
The public checkout contains plugin integration, lifecycle, approval and rendering
hooks, including `codex-rs/core-plugins/src/executor_hooks_tests.rs`; it does not
contain the installed CUA package implementations listed below.

Installed implementation locations verified on this machine:

- `/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/cua-repl`
- `/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/cua`
- `/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/browser-desktop`
- `/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/sky`
- `~/.codex/plugins/cache/openai-bundled/unified-computer-use/26.928.20755/.mcp.json`
- `~/.codex/computer-use/Codex Computer Use.app/Contents/MacOS/SkyComputerUseService`

Concrete installed entry points (paths relative to the `@oai` directory above):

| Component | Entry point |
| --- | --- |
| MCP JavaScript runner | `cua-repl/bin/cua-repl.mjs`; `cua-repl/dist/lib/js/oai_js_cua_repl/src/index.js` |
| Unified API | `cua/dist/lib/js/oai_js_cua/src/index.js`; `tinysky_alt/globals.js`; `tinysky_browser/create_tinysky_browser.js` under that source directory |
| Browser client/service | `browser-desktop/scripts/browser-client.mjs`, `browser-service.mjs` |
| Native client/service | `sky/dist/project/cua/sky_js/src/index.js`, `service.js`; native `SkyComputerUseService` executable listed above |

The SDK versions are `cua-repl` 0.1.0, `cua` 0.2.5, `browser-desktop` 0.1.1,
and `sky` 0.7.5. The plugin cache refreshed during the audit; the current
manifest is 26.928.20755, and the current source locations were rechecked.

The exposed tools are `cua_repl.js` and `cua_repl.js_reset`, with a hidden lifecycle
`turn_ended` hook. Native/browser operations live behind the initialized `cua` runtime.
The four installed package manifests do not declare an open-source license;
installation is not evidence that these implementations are open source.

### Codex primitive inventory

The installed `@oai/cua/docs/tinysky-alt-core-cua-repl.md` describes:

- Discovery/binding: `getState`, `getApp`, `listApps`, `listWindows`, `getBrowser`,
  `createBrowserTab`, `getTab`, `listBrowsers`, `listTabs`.
- Observations: `getAXState`, `getScreenshot`, `getAXStateAndScreenshot`.
- Target actions: `click` (button/click-count options), `drag`, `scroll`,
  `selectText`, `setValue`, `performSecondaryAction`.
- App/tab input: `paste`, `pressKey`, `typeText`.
- Browser navigation/lifecycle where supported: `goto`, `back`, `forward`,
  `reload`, `close`, `markDeliverable`, `markHandoff`.
- Browser-specific APIs include `tab.playwright` locators/DOM snapshots,
  `tab.dom_cua.get_visible_dom`, `tab.ax` and screenshots. Availability depends
  on the backend: DOM-only tabs reject native input wrappers and use locators.

Public configuration/integration source remains in the Codex checkout:
`codex-rs/config/src/{browser_use,computer_use,browser_computer_use_requirements}.rs`,
`codex-rs/app-server-protocol/src/protocol/v2/{browser_use_config,computer_use_config}.rs`,
`codex-rs/core-plugins/src/executor_hooks_tests.rs`, and the TUI
`history_cell/computer_activity.rs` / `thread_transcript/computer_groups.rs`.
These files configure, gate, hook and render the tool; the installed packages
above implement the browser/native actions.

## Findings, repairs, and evidence

| Requirement | Finding and current evidence | Status |
| --- | --- | --- |
| Preserve observations with screenshots | Responses previously replaced tool text with image-only output and dropped image detail. It now keeps text, image, detail, and call id. Byte-budget shedding now removes old images while retaining their observation text. Provider mapping and request-budget regression tests pass. | Repaired |
| Preserve screenshot resolution and coordinates | browser-use, direct CDP, and Chrome bridge screenshots use `original`. The direct screenshot at DPR=2 is 800×513 image pixels for an 800×513 CSS viewport. (0,0) succeeds; negative coordinates fail; right-click reports the correct button mask. | Repaired, browser evaluated |
| Genuine UI input | Desktop fallback previously used DOM `.click()`, value assignments and synthetic keyboard events. It now uses shared CDP primitives. Browser fixtures observe `isTrusted=true` for click, text input and key events, including Tab moving focus. | Repaired, browser evaluated |
| Fresh state and reliable targeting | Desktop actions and navigation return actual page text/refs. Missing and ambiguous targets fail. Ref identity survives DOM insertions; refs from a previous document fail. Typing rejects a non-editable target before clicking or inserting text. | Repaired, browser evaluated |
| Browser-use session ownership | One process previously served every thread. Threads now own distinct lazy servers and private profile/download/file directories. Separate processes alone were insufficient: the pinned upstream server defaults to a shared profile. Real pinned-runtime tests now prove thread cookie isolation and persistence within each thread. | Repaired, real runtime evaluated |
| Browser-use action/observation ordering | Calls within a thread are serialized; actions are followed by `browser_get_state` with screenshot. Fresh state precedes the action report so report text cannot crowd it out first. Real pinned-runtime click outcomes are independently checked through the page HTML. | Repaired, real runtime evaluated |
| Cancellation | In-flight browser-use cancellation stops the owned process tree; the next call starts fresh. Direct tools track held input and screenshot masks before sending commands, recover through a separate connection with a five-second cap, and await recovery before reusing a retained session. Real-browser tests cancel a drag and a key with a delayed acknowledgement, cancel a masked screenshot, and inject an error during a drag. Key cancellation retains the original session and immediately resumes it. Recovery is best effort if Chrome is unreachable or the runtime exits. | Repaired, browser evaluated |
| Untrusted observations | Existing markers remain on reads. Desktop eval results and tab titles/URLs are labeled; action observations from browser-use are labeled even when an error is present. Direct helper state and permission probes now execute in a named CDP isolated world. A fixture poisons the page's `window.__roderDirect`; genuine state and input still work. | Improved and browser evaluated; paired extension evaluated |
| Outcome verification | Jev fixture graders check actual page/DOM outcomes and recorded fixture POSTs; successful final model text alone does not determine a pass. The corpus passed 52/52, including 5 fallback tasks. | Deterministic harness evaluated |
| ACP permission and result contract | Public `session/new`/`session/prompt` tests assert permission requests, call identity, inputs, completed/failed tool updates, observed page text and final `end_turn`. Rejection executes zero actions. | 6 ACP tests passed |
| Site/action restrictions | Desktop has an optional exact-origin ceiling, stable numeric tab identity, and a binding per thread. Missing tab ids fail. Direct tools recheck current origin before each read/input, including external navigation and after a stop. Browser-use has an optional exact-host operator ceiling; direct navigation is rejected before transmission and an agent call cannot widen it. Restrictions remain opt-in. Redirect checks on direct tools stop subsequent interaction after the destination has loaded; they are not a network firewall. | Repaired and browser evaluated within configured scope; paired extension evaluated below |
| Execution bounds | browser-use agent calls default to 50 steps and accept only 1–100. Direct commands have a 30-second response timeout; waits/repeats/drag steps are bounded. | Implemented |

## Live model evaluation

The complete fixture corpus used the hosted decision model, resolved by server
telemetry to **`jev-1.13.0`**: **49/52 outcome graders passed (94.2%)**. Typed values
were supplied by the fixtures; this does not measure the text-helper model.
Expected blocked/budget/confirmation outcomes are included in the corpus.

- `icon_by_picture`: deleted the wrong mail rows, including a row the grader
  required to remain; it ended blocked.
- `scroll_region`: did not successfully scroll/unlock and submit the terms form.
- `enter_to_search`: reported `done` although no search POST occurred. This is
  direct evidence that a final success claim is insufficient.

A separate seven-task run with **Codex `gpt-6-sol`, low reasoning** enabled as
fallback passed 4/7 combined graders. The fallback ran on five tasks, with four
passing their post-fallback grader: region scrolling, canvas, hover and keyboard
tasks. Wrong-row deletion remained failed. Search reported `done`, so fallback
did not run. The drag task also reported `done` where this fixture expected
handover; this is separate from the shared direct drag primitive's verified
input behavior. This selected run is not comparable to the full corpus's rate.

- [Complete live-model rows](../evals/reports/browser-computer-use/2026-09-29/live-jev.jsonl).
- [Focused fallback rows](../evals/reports/browser-computer-use/2026-09-29/live-fallback-focused.jsonl).

## Paired Chrome extension repairs

The separate MV3 repository was repaired in an isolated Git worktree:
`/Users/pz/.codex/worktrees/browser-extension-audit/roder-web-extention`, branch
`pz/computer-use-primitives`, based on `7c77daa`. Extension commit: `607afa7`. Roder repair commit: `8c6ca4c3`. Unknown work in its main
checkout was preserved. These changes must be installed with the Roder audit
branch; the installed extension was not replaced by this evaluation.

- Click, type, keypress and wheel scrolling now use tab-targeted Chrome CDP
  `Input` commands. Editable targets and actual focus are checked first.
- Select remains a semantic DOM selection with an independently checked value
  and explicit `eventsTrusted: false` provenance. macOS native select popups
  did not respond to tab-targeted CDP keys in the loaded-extension evaluation.
  Code/function interfaces permit this higher-level operation; it is not
  claimed to be a native keyboard gesture.
- References use a cryptographic document nonce and isolated-world maps. Forged DOM attributes,
  stale references, ambiguous selectors and covered targets fail.
- Actions serialize through the resulting fresh, untrusted observation. Queued
  approvals retain a concrete tab, origin and document, with current settings
  and permission checked again before input.
- Screenshots use tab-targeted CDP rather than `captureVisibleTab`, support
  validated viewport crops, and preserve CSS coordinate mapping. The genuine
  DPR2 capture is 800x600; its filled fake password is masked in opaque black.
- Rust dispatch cancellation/timeout sends `command/cancel` to its original
  extension client. Running input and screenshot work is aborted, held input
  released, masks removed, and pending approvals rejected. Disconnect cancels
  pending/running work. Cleanup remains best effort when Chrome is unreachable.
- Debugger reads and streamed events recheck the attached origin's permission;
  typed values and eval expressions are no longer copied into command logs.
- Content/pair scripts now build as self-contained IIFEs for the MV3 classic
  content-script loader. The service worker remains an ES module.

Loaded MV3 extension evaluation: **17/17 checks**, through the real WebSocket
bridge and Chrome APIs in a throwaway Chromium profile. Includes trusted input,
Enter submission, nested wheel scrolling, inactive-tab screenshot/crop/masking,
stale/forged references, readonly/ambiguous rejection, parallel action ordering, Shift plus printable keys, ordinary insecure HTTP,
cancellation after browser-applied input/capture, document changes, active-tab
switches, and permission revocation while approval is queued. Its 24 existing
unit tests, TypeScript check and production build pass.

Artifacts and reproducible harness in the extension worktree:

- `scripts/eval-computer-use.mjs` (`pnpm eval:computer-use`).
- `output/playwright/extension-eval.json`.
- `output/playwright/extension-viewport.png` (visually inspected).
- `docs/computer-use-primitives.md`.

## Runtime completion verification

`jev_browse` now supports optional caller-defined `success_condition` predicates:
`url_contains` and `text_contains`. The runtime reads actual browser state again
after Jev reports DONE. Failure changes the status to blocked with
`outcome_mismatch`, allowing the existing bounded fallback to continue. A
fallback success claim is checked against fresh UI state again; a failed or
missing observation cannot establish success. Both result data and the calling
model's text state whether verification passed, failed, or was not requested.

Real-browser session tests cover premature search DONE, premature counter DONE,
verified progress, successful fallback recovery, and rejection of a fallback's
own false DONE. Input validation rejects unknown fields and oversized strings.

These checks verify only the supplied URL/text predicates. They are not proof
of an entire natural-language goal or a server transaction; fixture graders
still independently check DOM outcomes and POSTs. Without conditions, done is
explicitly a model claim. The 49/52 live-model result above predates this new
completion gate and does not demonstrate a new success rate for it.

## Evaluation artifacts and scope

- [52 deterministic Jev task rows](../evals/reports/browser-computer-use/2026-09-29/keyless.jsonl).
- [DPR2 viewport capture](../evals/reports/browser-computer-use/2026-09-29/viewport-dpr2.jpg), visually inspected: the filled password field is covered by an opaque black rectangle.
- [Screenshot dimensions and masking count](../evals/reports/browser-computer-use/2026-09-29/screenshot.json).
- [`computer_use.rs`](../crates/roder-ext-chrome/tests/computer_use.rs): local HTTP fixture and isolated headless Chrome; no user profile or account.
- [`fake_server.rs`](../crates/roder-ext-browser-use/tests/fake_server.rs): MCP protocol, process cleanup, cancellation, per-thread ownership and fresh observation tests.
- [`acp_browser.rs`](../crates/roder-app-server/tests/acp_browser.rs): public ACP request/notification boundary with allow and reject outcomes.
- `roder-ext-chrome/tests/support/cancellation.rs`: real Chrome behind a
  one-shot faulty CDP relay; browser-applied input is verified before cancellation.
- `roder-ext-chrome/tests/support/scope.rs`: stable tab id after a new tab opens,
  direct/off-origin navigation, redirect reporting, denied subsequent input and
  eval, and recovery to an allowed site.
- `roder-ext-browser-use/tests/support/live_fixture.rs`: real pinned server,
  separate cookies per thread, own-thread persistence, click outcome and domain
  rejection before an HTTP request.

Reproduce from the repository root:

```sh
mise exec -- env RODER_REQUIRE_CHROME=1 cargo test -p roder-ext-chrome --test computer_use -- --nocapture
mise exec -- cargo test -p roder-ext-browser-use
mise exec -- cargo test -p roder-ext-openai-responses
mise exec -- env JEV_REQUIRE_CHROME=1 cargo test -p roder-ext-jev keyless_corpus -- --nocapture
mise exec -- cargo test -p roder-app-server --features e2e-tests --test acp
```

The deterministic 52/52 result is distinct from the live-model results above.
Both real pinned browser-use tests passed, including tool-schema comparison.
The first workspace build failed when the shared Cargo target files disappeared
during compilation (`could not parse/generate dep info`, `No such file or directory`).
The isolated targeted rerun passed: browser-use 24 unit + 9 integration tests,
Chrome 28 unit + 1 browser evaluation, Responses 114 unit tests. The isolated full workspace run reached app-server e2e and failed 3 checks
(126 passed, 1 ignored): `providers_clear_removes_api_key`,
`runners_methods_list_select_status_and_delete_destination`, and
`tools_list_discovers_configured_web_search_without_secret_material`. A clean
worktree at the original base revision reproduced the same 3 failures and the
same 126/3/1 counts. These are baseline failures in this environment; the full
workspace gate remains unverified past that package. The ACP suite passed again
within the isolated run (6/6).

Later workspace runs found an old Jev handover test using `(0,0)` as an omitted
point; its caller was updated to canonical null coordinates. A core mailbox
interrupt test failed during a workspace run and passed its isolated rerun.
A Jev fixture Chrome was SIGKILLed during startup; that test passed its isolated
rerun. All workspace packages except app-server/core/Jev passed in the final
remaining-package run. The Jev broad run reached 446 passes with the single
Chrome-startup failure; these results do not constitute a green full-workspace
run. Final targeted suites passed: browser-use 25 unit + 9 integration,
Chrome 28 unit + 1 real-browser evaluation, Responses 114 unit.

The pinned upstream implementation also only instantiates OpenAI for its MCP
LLM tools. An Anthropic key alone is no longer reported as sufficient. Content
extraction initializes from the private configuration; the configured OpenAI
key is included there with private permissions and removed on shutdown. No
real OpenAI extraction/agent call was made in the pinned-server evaluation.

## Deployment and evaluation limits

- These are local audit-branch commits, not landed or deployed changes. Roder and
  its paired extension must be updated together.
- Browser-use creates an isolated owned profile. The Chrome extension controls
  the profile to which it is paired, and an external CDP endpoint can belong to
  an existing profile. Use a dedicated agent profile when isolation is required;
  a plugin cannot retroactively isolate a user's existing logged-in browser.
- Site/action ceilings remain operator-configured. Redirects may load before
  the next permission check; the restrictions stop subsequent input/inspection
  and are not a network firewall.
- Masking uses sensitive-field heuristics. It is not a guarantee that arbitrary
  pages cannot show secrets elsewhere.
- Native macOS select-popup keyboard behavior is not claimed. Select is labeled
  as a semantic DOM operation and its actual value is verified.
- No new sensitive-action consent was added, per user scope.
- Live model decisions remain fallible; the new optional completion gate verifies
  specific UI predicates. The model corpus result remains 49/52, not 100%.

The clean app-server run passed **129 tests, one ignored**, with a temporary
`RODER_CONFIG_DIR` and ambient credential variables removed from the child
process. Earlier identical baseline failures therefore reflected this test
environment's credentials/configuration, not a browser regression. Final verification used a private `.roder` subdirectory, preserving the
canonical config-path shape expected by the TUI test without changing HOME.

Final package coverage passed **3,809 tests, 67 ignored**, across clean runs:
core 346; the workspace excluding core/Jev 2,964; Jev 499 (452 unit + 47
integration). Jev doc tests passed separately. The browser fixtures required
real Chrome. Extension production build, TypeScript check and 24 unit tests
also passed, with 17/17 checks against a loaded MV3 extension.

This is green package coverage, not a single green full-workspace invocation.
Full invocations encountered the previously observed core mailbox timing failure
and a process-host cancellation-event race; both suites passed independently.
An earlier temporary config directory without the `.roder` suffix conflicted
with the TUI's config-path assertion; the final canonical private layout passed.
No unrelated runtime or tests were changed to obtain those results.

[Structured validation summary](../evals/reports/browser-computer-use/2026-09-29/workspace-validation.json).
