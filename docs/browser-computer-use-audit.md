# Browser computer-use audit — 2026-09-29

Status: **in progress**. Primitive and observation repairs are implemented and
have targeted test evidence. Safety parity is not yet established; the open
items below remain part of the task.

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
`b1e72963c3b71a9265a551e54beff078384efed9` after the requested fast-forward pull.
The public checkout contains plugin integration, lifecycle, approval and rendering
hooks, including `codex-rs/core-plugins/src/executor_hooks_tests.rs`; it does not
contain the installed CUA package implementations listed below.

Installed implementation locations verified on this machine:

- `/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/cua-repl`
- `/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/cua`
- `/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/browser-desktop`
- `/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/sky`
- `~/.codex/plugins/cache/openai-bundled/unified-computer-use/26.924.22138/.mcp.json`
- `~/.codex/computer-use/Codex Computer Use.app/Contents/MacOS/SkyComputerUseService`

The exposed tools are `cua_repl.js` and `cua_repl.js_reset`, with a hidden lifecycle
hook. Native/browser operations live behind the initialized `cua` runtime.
The four installed package manifests do not declare an open-source license;
installation is not evidence that these implementations are open source.

## Findings, repairs, and evidence

| Requirement | Finding and current evidence | Status |
| --- | --- | --- |
| Preserve observations with screenshots | Responses previously replaced tool text with image-only output and dropped image detail. It now keeps text, image, detail, and call id. Byte-budget shedding now removes old images while retaining their observation text. Provider mapping and request-budget regression tests pass. | Repaired |
| Preserve screenshot resolution and coordinates | browser-use, direct CDP, and Chrome bridge screenshots use `original`. The direct screenshot at DPR=2 is 800×513 image pixels for an 800×513 CSS viewport. (0,0) succeeds; negative coordinates fail; right-click reports the correct button mask. | Repaired, browser evaluated |
| Genuine UI input | Desktop fallback previously used DOM `.click()`, value assignments and synthetic keyboard events. It now uses shared CDP primitives. Browser fixtures observe `isTrusted=true` for click, text input and key events, including Tab moving focus. | Repaired, browser evaluated |
| Fresh state and reliable targeting | Desktop actions and navigation return actual page text/refs. Missing and ambiguous targets fail. Ref identity survives DOM insertions; refs from a previous document fail. Typing rejects a non-editable target before clicking or inserting text. | Repaired, browser evaluated |
| Browser-use session ownership | One process previously served every thread. Threads now own distinct lazy servers; each thread reuses its own process. Fake MCP integration tests compare browser PIDs. | Repaired, MCP evaluated |
| Browser-use action/observation ordering | Calls within a thread are serialized; actions are followed by `browser_get_state` with screenshot. Fresh state precedes the action report so report text cannot crowd it out first. | Repaired, MCP evaluated; real pinned server recheck outstanding |
| Cancellation | Cancelling an in-flight browser-use call stops its owned process tree. A subsequent call starts fresh. The integration test checks both old-PID death and a different replacement PID. | Repaired for browser-use; direct input cleanup still open |
| Untrusted observations | Existing markers remain on reads. Desktop eval results and tab titles/URLs are labeled; action observations from browser-use are labeled even when an error is present. | Improved; separate extension enforcement review open |
| Outcome verification | Jev fixture graders check actual page/DOM outcomes and recorded fixture POSTs; successful final model text alone does not determine a pass. The corpus passed 52/52, including 5 fallback tasks. | Deterministic harness evaluated |
| ACP permission and result contract | Public `session/new`/`session/prompt` tests assert permission requests, call identity, inputs, completed/failed tool updates, observed page text and final `end_turn`. Rejection executes zero actions. | 6 ACP tests passed |
| Site/action restrictions and sensitive transmission | Jev's irreversible gate is off by default; its label shortlist and direct typing gate do not cover all sensitive transmission. Desktop uses `OpenGuard`. browser-use domain restrictions are optional. These do not establish the guide's required runtime controls. | Open |

## Evaluation artifacts and scope

- [52 deterministic Jev task rows](../evals/reports/browser-computer-use/2026-09-29/keyless.jsonl).
- [DPR2 viewport capture](../evals/reports/browser-computer-use/2026-09-29/viewport-dpr2.jpg), visually inspected: the filled password field is covered by an opaque black rectangle.
- [Screenshot dimensions and masking count](../evals/reports/browser-computer-use/2026-09-29/screenshot.json).
- [`computer_use.rs`](../crates/roder-ext-chrome/tests/computer_use.rs): local HTTP fixture and isolated headless Chrome; no user profile or account.
- [`fake_server.rs`](../crates/roder-ext-browser-use/tests/fake_server.rs): MCP protocol, process cleanup, cancellation, per-thread ownership and fresh observation tests.
- [`acp_browser.rs`](../crates/roder-app-server/tests/acp_browser.rs): public ACP request/notification boundary with allow and reject outcomes.

Reproduce from the repository root:

```sh
mise exec -- env RODER_REQUIRE_CHROME=1 cargo test -p roder-ext-chrome --test computer_use -- --nocapture
mise exec -- cargo test -p roder-ext-browser-use
mise exec -- cargo test -p roder-ext-openai-responses
mise exec -- env JEV_REQUIRE_CHROME=1 cargo test -p roder-ext-jev keyless_corpus -- --nocapture
mise exec -- cargo test -p roder-app-server --features e2e-tests --test acp
```

This is primitive and deterministic harness evaluation, **not a measured success
rate for a live model**. Ignored live/network tests have not been run. The first workspace build failed when the shared Cargo target files disappeared
during compilation (`could not parse/generate dep info`, `No such file or directory`).
The isolated targeted rerun passed: browser-use 24 unit + 9 integration tests,
Chrome 28 unit + 1 browser evaluation, Responses 114 unit tests. A full workspace
run, including app-server e2e features, is running with
`CARGO_TARGET_DIR=.target-browser-audit` and is not yet reported as passed.

## Remaining completion gates

1. Enforce site and action scope across browser-use, Desktop fallback, Jev and
   the separate Chrome extension; verify redirect/off-origin and tab-switch cases.
2. Confirm sensitive typing before the first input event. Make consequential
   action checks effective by default and verify ambiguous commitment labels,
   cookie consent, forms and subframes. A whole autonomous-agent approval does
   not prove step-specific consent for its hidden internal actions.
3. Verify direct-tool cancellation releases held mouse buttons/modifiers and
   removes screenshot masks; exercise failure partway through a drag.
4. Recheck the real pinned browser-use server, including screenshot/state shape,
   session profile isolation and operation ordering; fake MCP evidence proves
   Roder's protocol behavior, not the upstream runtime's behavior.
5. Run live-model fixture evaluation with observed outcome graders and compare
   supported primitives, outcome rate, limits and consent outcomes.
6. Finish workspace/app-server gates, rerun changed browser fixtures after the
   last changes, and validate the release changeset against the committed branch.
