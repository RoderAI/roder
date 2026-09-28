## 0.3.0 (2026-09-28)

### Breaking Changes

#### Harden Jev's browsing: settling, reach, twin context and retries

Jev now owns its in-page scripts, prompts and fixtures, and diverges from
upstream Jev Ultrafast on purpose (the crate README lists every divergence).
Still one decision round trip per step.

- Reads wait for the page to go quiet (200 ms without mutation or input, no
  visible loading indicator, capped at 2.5 s) instead of two animation frames.
- Controls above and below the viewport that a scroll can reach are offered as
  `offscreen`; lists keep their pagers under the action caps; `act.js` scrolls
  to its target, hit-tests five points and reports a covered target as the new
  `Covered` error, which counts towards the stall rule.
- Controls that read alike carry a `context` naming their card, row, section or
  table column.
- Controls cut off by a collapsed or `overflow: clip` ancestor are no longer
  offered.
- DONE straight after a covered attempt ends blocked, and an unsure repeat of
  the click that just took effect ends done without clicking again.
- The decision and text-helper transports reuse one client, retry transient
  failures with backoff and `retry-after`, resend a possibly billed request at
  most once, and never retry past the run's deadline.
- A run that stops early says why: new statuses `budget_exceeded`,
  `timed_out`, `needs_input`, `unavailable` and `error` replace the `ready`
  such runs used to report, and the tool result adds a `next_step` hint for
  every status but `done`.
- `timeout_seconds` now covers the whole task, including starting Chrome and
  loading the page, and is cut to the host's remaining tool deadline; a setup
  phase that runs out ends `timed_out` naming the phase.
- A start page that fails to load is retried on transient `net::ERR_…` errors,
  then ends `blocked` ("could not load (…)") without a decision. Jev's tab is
  closed on every failure after it was created, including Chrome's error
  pages, which used to leak.
- `JEV_CDP_URL` attaches to a browser by http(s) address or ws(s) websocket;
  the tool result shows a non-loopback endpoint only as scheme and host.
- The start URL must be an http(s) URL with a host.
- Snapshot names skip script and style text, `aria-labelledby` resolves in a
  shadow root first, and hydrating `aria-disabled` to `"false"` no longer
  makes a decision stale.
- JavaScript dialogs no longer stall a run for 30 s: alerts and
  `beforeunload` are accepted, confirms and prompts dismissed, each is
  reported before the next observation's page text and on the step before
  it, and a run left `blocked` by a dismissed confirm says so.
- A click or fill moves the pointer first and presses only once the
  unchanged target stays under it (for up to 1 s), so hover handlers run and
  a layout shift cannot redirect the press; about 30 ms per click.
- After a fill into a field that offers suggestions, or a click on a control
  that announced a popup, the settle waits up to 1.2 s for the options or the
  popup.
- Fills select by script instead of the select-all accelerator, follow focus
  to an editor the click opened, verify the text stayed, type once more into
  an editor that took focus late, and set native date and time inputs to
  their ISO value; about 150 ms per fill. A fill or select the page refuses
  is recorded on the step, not an error.
- Styled checkboxes and radios are offered through their label, and anchors
  without `href` that handle clicks as buttons.
- The decision request's state and the text helper's field context carry
  today's date, and a native date field's ISO format.
- A foreground task's tab is shown when it is created rather than after the
  first observation.
- Breaking: `JevBrowser::act` returns the new `JevActOutcome`; observations
  may carry `dialogs` (new `JevDialog`); `JevActionRecord` gains `refused`
  and `dialogs`; `JevBrowser::activate` and `JevEngine::activate` are
  removed.
- Breaking: `JevActionRecord` gains `covered` and `target_confidence`;
  `JevDecision` gains `target_confidence`, `model` and `call_confidence()`;
  `JevDecisionRecord` gains `target_confidence` and `model`.
- Breaking: `JevStatus` is `#[non_exhaustive]`, serializes in snake case, and
  gains `BudgetExceeded`, `TimedOut`, `NeedsInput`, `Unavailable` and `Error`;
  runs that ended on a budget, timeout, missing value or provider failure no
  longer report `Ready` (or `Blocked` for the action budget). New public
  `JevStop` error carries a status out of a decision client, transport or
  text resolver.
- Breaking: `BU_CDP_URL` and `BU_CDP_WS` are no longer read (they never
  worked); set `JEV_CDP_URL` instead.
- Elements with a click listener (read with DevTools' `getEventListeners`),
  an `onclick` handler, a `tabindex` or a pointer cursor are offered as
  buttons, or as options in a floating list, unless they sit inside a control,
  cover more than a third of the viewport or only wrap other controls; an
  unnamed one is named by its picture's file. MiniWoB++ went from 200 to 357
  of 645 episodes over all the changes below, 36 tasks became expressible,
  and a 28,600-element page's snapshot went from 58 ms to about 130 ms.
- Fields are never named by their own content; an unlabelled one is named by
  the short text beside it (a label with no `for`, a header cell in its row).
  Text fields carry `input_type` (withheld from the decision model for native
  date inputs), their values are page text (never a password's), a select
  offers its current option, and text a scroll box hides is not page text.
- The text helper is told the field's `context` and kind and the other
  fields' values (`other_fields`).
- Boxes that scroll on their own are offered as `SCROLL_REGION_DOWN` and
  `SCROLL_REGION_UP` targets with a `scrolled` position; `PRESS_ENTER` in the
  field Jev typed into and a `PRESS_ESCAPE` control are offered while useful
  (the current model has not chosen either).
- Twin contexts read short rows whole and name unnamed grid cells by row and
  column.
- A multi-line value typed into a single-line input is no longer reported as
  refused.
- Controls in open shadow roots and same-origin iframes are observed, named,
  read as page text and hit-tested where they are drawn (through
  `shadowRoot.elementFromPoint` and frame coordinate mapping); closed shadow
  roots and cross-origin frames are still out of reach.
- A tab opened by an action (`target="_blank"`, `window.open`) is followed:
  Jev adopts it, reads it from then on and records `opened_tab` on the step;
  every tab Jev owns is closed with the page.
- Each step records `effect`, what it visibly did, next to `page_changed`
  (not sent to the decision model yet).
- `JEV_ALLOWED_ORIGINS`, narrowed by a call's new `allowed_origins`, ends a
  run `blocked` on a page outside it and is named in the approval request;
  `JEV_MAX_ACTIONS` sets the action budget and `JEV_MAX_SECONDS` caps
  `timeout_seconds`. The result carries summed decision and text `usage`
  (`"unknown"` where a provider did not report a count).
- Chrome is found through its profile's `DevToolsActivePort` after the port
  probe and launched with `--remote-debugging-port=0`; a Chrome that exits
  fails at once, its stderr goes to `jev-chrome/chrome-stderr.log`, the
  750 ms startup pause is gone, and a profile Roder creates starts with the
  password manager off.
- Breaking: `JevRunResult` gains `usage` (new `JevUsage`, `JevCallUsage`,
  `JevTokenCount`); `JevActionRecord` gains `opened_tab` and `effect`;
  `JevEngineConfig` gains `with_max_actions`, `with_max_duration` and
  `with_scope` (new `JevOriginScope`); observations may carry `opened_tab`.
  Code that builds these structs with literals must add the fields.
- Breaking: a Chrome Roder starts no longer listens on 9222 (or
  `JEV_CDP_PORT`) but on a port it picks, advertised in the profile's
  `DevToolsActivePort`.
- Launches on Jev's Chrome profile are serialised (a process-wide lock and a
  lock file beside the profile) and never delete `DevToolsActivePort`: a
  launch that hands off to a Chrome already running on the profile uses that
  Chrome, and parallel tasks start one Chrome.
- An http(s) `JEV_CDP_URL` with a path or query is looked up at
  `<path>/json/version?<query>`; DevTools messages holding half an emoji no
  longer fail to decode.
- A fill whose press navigated is recorded as a refused step; a tab an action
  opened becomes current only once its setup succeeds; closing a tab Chrome
  already dropped succeeds; a deadline during the new tab's attach closes it;
  500, 504 and 524 are resent at most once; a huge `JEV_MAX_ACTIONS` no
  longer overflows.
- Billed calls whose answer could not be used (a decision that failed
  validation, a text helper reply without a value) count in `usage` and
  `model_calls` (new public `JevBilled`).
- Effects no longer pair controls by node id across a navigation or tab.
- The snapshot reaches controls in an app shell's scrolling pane,
  `display: contents` links and text, a transparent native select, text
  directly in a shadow root and slotted text in place; an icon element drawn
  in a shadow root inside a button no longer becomes a phantom control that
  covers the button; `tabindex` alone no longer makes a slider, tab panel or
  code block a button; editable boxes and ARIA text boxes are not named by
  their content; masked secrets (`-webkit-text-security`, password or
  one-time-code autocomplete) are secret fields like passwords; a select's options
  count once against the action caps; strings are cut on whole characters.
- A suggestion list still showing the last query's options is waited past;
  a field that formats typed text (a phone mask) or normalises a datetime is
  no longer refused; page scrolls move the page, not the scroller under a
  fixed point.
- The tool no longer tells callers to put credentials in the goal.
- Adds scripted agent-loop tests, a real-Chrome fixture harness and a
  40-task end-to-end eval corpus with a keyless tier and an opt-in live tier.
  `JEV_REQUIRE_CHROME=1` (or `CI`) makes the Chrome-backed tests fail rather
  than skip when no Chrome is found.
- Opt-in irreversible-action gate, off by default (`JEV_CONFIRM_IRREVERSIBLE=1`
  or `JevEngineConfig::with_irreversible_gate`): the same decision request
  also asks a Noul about each Enter and each click whose label names a
  commitment (at most 8), and a run whose chosen action may not be undone
  (P above 0.5, or no valid answer) ends with the new status
  `needs_confirmation`, nothing dispatched, unless the call sets the new
  `authorize_irreversible` argument and the decision's call confidence is at
  least 0.90. A call that sets `authorize_irreversible` needs approval even in
  accept-all mode. Thresholds and question shape are fastbrowse's and
  unvalidated against the hosted model. With the gate off the decision
  request is unchanged.
- Cookie-banner refusal, on by default (`JEV_REFUSE_COOKIE_BANNERS=0` or
  `JevEngineConfig::with_cookie_banner_refusal(false)` turns it off): Roder
  now bundles DuckDuckGo's autoconsent 16.40.0 (MPL-2.0, unmodified, with its
  licence; SHA-256 pinned by a test; notice in
  `third-party/duckduckgo-autoconsent/`), injected into every document of
  every tab Jev owns, adopted tabs included, before it loads, in its own
  opt-out mode, with its heuristics held to refusing (its own `tier2`
  default also presses a lone "Accept" or "OK"). Jev's own `consent.js` stays as the fallback for banners
  autoconsent does not know, clicking an unambiguous banner's reject button
  once per document, never an accept or settings button. They never both act
  on one document: Jev waits for an opt-out autoconsent is running, and the
  fallback clears autoconsent's action when it clicks. Either refusal is
  recorded as a `cookie_banner` step with no model call. Injecting it costs
  about 30 to 55 ms more to open a small page and 70 to 135 ms on a large
  one, 0 to 70 ms on the first read, and makes the `roder` binary 445,824
  bytes larger.
  The crate's licence expression is now `MIT AND MPL-2.0`.
- Password and one-time-code fields can be typed into: they are offered as
  fills with `input_type` `password` or `one-time-code` and `filled`, never
  read (not their value, page text, guard, effects or the text helper's
  other fields), the fill check for them runs in the page and returns only a
  boolean, and what Jev types there is recorded as `[secret]` everywhere and
  scrubbed from what the page later shows. The text helper types a password
  or code only when the goal holds it word for word; otherwise the run ends
  `needs_input` naming the field. A `needs_input` from the text helper or a
  missing text model now names the field. The tool description and docs say
  plainly that a secret in the goal goes to the hosted decision service, the
  text model and the transcript, and that a `JevTextValueResolver` avoids
  that. MiniWoB++'s `enter-password`, `login-user` and `login-user-popup`
  are labelled supported (not yet run).
- The text helper writes with GPT-6 Sol (`gpt-6-sol`) at low reasoning
  effort whenever Roder holds a ChatGPT/Codex sign-in, ahead of the calling
  turn's model and the fallback providers, over the Responses API: the
  request is built by `roder-ext-openai-responses`, the reply held to a
  strict JSON schema for `{"text": string | null}` and then to the same
  checks as before, the token taken (and refreshed) from `roder-codex-auth`,
  and a 401 sent once more only when the stored token changed meanwhile.
  `JEV_TEXT_MODEL` alone now picks a catalog model through the provider that
  serves it (`deepseek-chat` forces DeepSeek), and the new
  `JEV_TEXT_MODEL_REASONING` (`none`, `low`, `medium`, `high`) sets the
  effort on either path; an explicit model or effort Roder cannot serve
  fails the call instead of being substituted. A default GPT-6 Sol choice
  whose stored sign-in cannot produce a usable token (refresh refused,
  expired, or its token refused with 401) falls back for the rest of the run
  to the turn's model or the provider list, and says so. The tool result's
  `text_model` names the model that actually wrote values, gains `effort`
  and, for such a stand-in, a `note`; its source may be `codex`. Provider
  rejections (any 4xx but 401) now carry their trimmed body.
- The live eval tier with `JEV_EVAL_TEXT=model` gives secret fields the
  task's own values, as a supervisor's resolver would, and records each text
  call's latency, usage and outcome in the row; `one_time_code_missing`
  accepts `blocked` as well as `needs_input` live (nothing typed or posted
  either way), and task `status` may list several statuses.
- Breaking: `JevStatus` gains `NeedsConfirmation`; `JevDecision` and
  `JevDecisionRecord` gain `irreversible`; `JevDecisionClient` gains the
  provided method `choose_gated` and `JevBrowser` the provided method
  `refuse_cookie_banner`; `JevEngineConfig` gains `with_irreversible_gate`,
  `with_irreversible_authorized` and `with_cookie_banner_refusal(bool)`, and
  banner refusal is on unless that is given `false`.
- The `roder` binary names its TLS crypto backend at start. Both rustls
  backends are compiled in, so a TLS websocket (the Codex provider's, or a
  `wss` `JEV_CDP_URL`) panicked.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.3 (2026-09-26)

### Features

#### Expose an injectable JEV engine

Add public browser, decision-client, and text-value interfaces around the
bounded JEV loop. Hosted runners can now retain browser supervision, evidence
capture, and secret indirection while reusing JEV's action planner. The
existing `jev_browse` tool continues to use the built-in Chrome and provider
adapters.

## 0.1.2 (2026-09-26)

### Fixes

#### Publish today's content under fresh crate versions

`roder-api` 0.1.21, `roder-core` 0.1.19, `roder-app-server` 0.1.17,
`roder-evals` 0.1.2 and `roder-ext-openai-responses` 0.1.10 were published to
crates.io on 2026-09-23 from a branch that predates the Codex agent backend,
the Jev browser tool, the Chrome bridge work and the Grok 4.7 catalog entries.
crates.io versions are immutable, so those numbers cannot carry the current
code and the registry copies do not match the git tags of the same version.

Bump them so the released content reaches crates.io under versions that
describe it. `roder-ext-jev` and `roder-ext-codex-backend` are bumped with
them: both are new crates whose first publish must depend on a registry
`roder-api` that actually contains `roder_api::backend`, which the 0.1.21
registry copy does not.

Adds the package-local READMEs both new crates need to publish.

## 0.1.1 (2026-09-26)

### Features

#### Add Jev Ultrafast as a browser tool provider

Expose a bounded `jev_browse` tool backed by the upstream Jev CDP agent, with
Roder policy approval and a provider-configured `JEV_API_KEY`.

Roder resolves the Chrome DevTools endpoint itself: it reuses one that is
already listening and otherwise starts a visible Chrome on its own profile, so a
browser task no longer fails because nothing debuggable is running. Tasks run in
a foreground tab by default (`foreground: false` for a background tab), and the
typing helper is served from Roder's own harness — the calling turn's model when
it speaks chat-completions, otherwise the first configured chat-completions
provider — instead of requiring an OpenRouter key.

Results now carry `observed_elements` and, when upstream aborts at its action or
model-call budget, `stopped_because` plus the trace collected so far instead of
a bare traceback. A blocked run explains what Jev can target, so a caller can
choose another page rather than substituting a local one. `wait` actions hold
the page for `JEV_WAIT_MS` (default 800ms) rather than upstream's 100ms, and the
text helper's reasoning field follows the provider's accepted shape.

Jev is now a Rust port that runs inside the Roder binary — the agent loop,
action space, decision contract, text helper and a direct CDP client — with
upstream's in-page scripts vendored verbatim. Browser tasks need no Python, no
`uv` and no `browser_harness` daemon. Fixtures recorded from the upstream Python
pin the port's behaviour: the decision request body matches byte for byte, the
observation fingerprint matches hash for hash, and every documented validation
verdict agrees.
