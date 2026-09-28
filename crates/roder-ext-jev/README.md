# roder-ext-jev

`roder-ext-jev` is the goal-directed browser tool for [Roder](https://roder.sh).

## What It Does

Exposes a `jev_browse` tool that runs a bounded browser task against Chrome over
the DevTools Protocol: given a starting URL and an observable goal, it observes
the page, chooses one operation at a time, executes it, and returns the action
trace with the observed final page.

The implementation started as a Rust port of
[Jev Ultrafast](https://github.com/browser-use/jev-ultrafast) (MIT) at
revision `1231850a`. This crate now owns its in-page scripts, prompts and test
fixtures and diverges from upstream on purpose; see
[Divergences from upstream](#divergences-from-upstream). The agent loop,
indexed action space, decision contract and text helper keep upstream's shape,
and fixtures first recorded from the upstream Python pin them. Where Jev
diverges, those fixtures are re-recorded deliberately and the golden tests
stay.

It also vendors [DuckDuckGo Autoconsent](https://github.com/duckduckgo/autoconsent)
16.40.0 (MPL-2.0), unmodified, in `src/assets/autoconsent/` with its
`LICENSE` and a `SOURCE.txt` naming its npm source and SHA-256, which a test
pins. That file alone is MPL-2.0 (the crate's licence is `MIT AND MPL-2.0`);
Jev's code that injects it and reads its result is its own. The repository's
notice is `third-party/duckduckgo-autoconsent/`.

## Behaviour

- Roder reuses a running Chrome DevTools endpoint or starts a visible Chrome on
  its own profile.
- Controls in open shadow roots and same-origin frames are reached, and a tab
  an action opens is followed.
- The operator can limit the origins a task visits (`JEV_ALLOWED_ORIGINS`),
  its actions (`JEV_MAX_ACTIONS`) and its time (`JEV_MAX_SECONDS`).
- Tasks run in a foreground tab by default, leaving the final page on screen.
- Typing is served by Roder's own configured chat-completions provider.
- JavaScript dialogs are answered: alerts accepted, confirms and prompts
  dismissed, and each is reported.
- Page content is untrusted: a `done` result is an agent claim to be checked
  against the observed page.
- Cookie banners are refused by default (`JEV_REFUSE_COOKIE_BANNERS=0`
  turns it off): DuckDuckGo's autoconsent, injected into every document,
  refuses the consent platforms it knows, and Jev's own `consent.js` a clear
  banner it does not. An irreversible-action gate, off by default
  (`JEV_CONFIRM_IRREVERSIBLE=1`), ends a run `needs_confirmation` before a
  purchase, payment, send, publish or delete unless the call sets
  `authorize_irreversible`. Neither adds a model call, and the gate off, the
  decision request is byte-for-byte unchanged.
- Password and one-time-code fields can be typed into. What such a field
  holds is never read (the observation says only whether it is filled), and
  what Jev types there is recorded as `[secret]` and scrubbed from what the
  page shows afterwards.

Requires `JEV_API_KEY`. In Roder's default policy mode each goal needs approval
before the browser starts; plan mode denies it. The goal is sent to the hosted
decision service and to the text model and is stored in the transcript, so a
password or one-time code placed in the goal goes there too. An embedding host
can supply such values through a `JevTextValueResolver` instead (an opaque
reference in, the value out at execution), which keeps them out of the goal,
the requests and the trace. Without a resolver, the text helper takes a
password or code only when it is written in the goal word for word; a
one-time code is rarely known in advance, so the run ends `needs_input`
naming the field.

See [`docs/jev-browser.md`](https://github.com/RoderAI/roder/blob/master/docs/jev-browser.md)
for configuration and known limitations.

## Divergences from upstream

Jev keeps upstream's one decision round trip per step and adds no model
calls. Everything else below is a deliberate, Jev-owned change;
[`docs/jev-browser.md`](https://github.com/RoderAI/roder/blob/master/docs/jev-browser.md)
has the reasoning and measurements.

**Settling.** `track.js`, `settle.js` and `ready.js` replace upstream's
two-frame or 50 ms settle with a quiet clock kept inside the page: a read waits
for 200 ms without mutation or input and for no visible loading indicator
(ignored after 1.5 s), capped at 2.5 s, a cap also held on the Rust side. A
poll that runs late after a renderer stall looks once more before it trusts
the clock. After input, a step costs at least 200 ms (before: about 30 ms), and
a page that never goes quiet pays the cap on every step and on open. Opening a
tab waits for `interactive` (up to 15 s) instead of polling for `complete`.
An evaluate that loses its document is a stale page, not an error that ends
the run. For up to 1.2 s, quiet is not enough after two kinds of input: a fill
into a field that offers suggestions (a combobox, `type=search`,
`aria-autocomplete`, or one that controls or owns another element) waits for
options (visible ones other than those shown before the text was typed, so a
debounced list still showing the last query's options is waited past), and a
click on a control that announced a popup (`aria-haspopup`,
`aria-expanded="false"`, `aria-controls` or `aria-owns`, not yet expanded)
waits for it to expand or for the element it controls to show. Upstream gave
only a combobox fill 200 ms. A popup that never opens costs the 1.2 s.

**Observation (`snapshot.js`, `context.js`).**
- A checkbox or radio drawn by its label (the input transparent, not
  rendered, or under 2 px) is offered once, through the label, with the
  input's `checked`; an anchor with no `href` but an `onclick` is offered as a
  button. Upstream offered neither.
- Native `date`, `datetime-local`, `month`, `week` and `time` inputs are
  offered as text fields carrying `input_type`; upstream dropped them. The key
  is not sent to the decision model.
- Controls above and below the viewport are kept and flagged
  `offscreen: true`, unless no scroll can reach them (above the document,
  past its end, or inside a fixed layer). A control in a box that scrolls on
  its own (an app shell's pane, where the document does not scroll) is
  reached when it is within that box's content. Controls off to the side are
  still dropped. Unreached controls count in `omitted_actions`.
- A control cut off by an ancestor nothing can scroll is dropped: a collapsed
  panel (`height: 0; overflow: hidden`) or `overflow: clip`. A hidden-overflow
  box with room, such as a carousel, is kept, because `act.js` scrolls it.
- Actions are sorted by distance from the viewport centre, capped at 100
  offscreen and 250 in all, and pagers and load-more controls always survive
  the caps (fastbrowse's rules). The caps count controls, so a select's
  options count once. Upstream cuts at 250 actions in document order.
- The freshness marker uses on-screen actions only, and guards are computed
  only for the actions kept.
- `context.js` adds an optional `context` naming the card, row, section or
  table column of controls that read alike, and the section of an offscreen
  control. The marker is built before it runs, and `fresh()` never runs it.
- The step fingerprint leaves out offscreen actions and `context`, so changes
  below the fold cannot hide a stall. An upstream-shaped observation hashes
  exactly as before.
- The guard records `aria-disabled` as whether it is `"true"`, so hydrating
  it to `"false"` does not make a decision stale; names skip `script`,
  `style`, `noscript` and `template` children; `aria-labelledby` resolves in
  the element's shadow root before the document.
- Generic clickables are offered as `button` (or `option`, for an item of a
  floating list): an element with a click, mouse or pointer listener (read
  with DevTools' `getEventListeners`, so `observe()` and `fresh()` evaluate
  with the command-line API), an `onclick` handler, a `tabindex` of 0 or more,
  or the outermost element of a pointer-cursor region. Not one inside a
  control, one larger than a third of the viewport, one whose text all
  belongs to the controls it holds (or that holds more than four), nor an
  inner element reading the same as its clickable parent. `treeitem` and
  `menuitemcheckbox` join the roles. `act.js` treats these as controls when
  hit testing. An offscreen one is named only once the caps keep it. A
  `tabindex` alone counts only on an element with no `role` that is not a
  `pre`, and "inside a control" crosses shadow roots (an icon element in a
  button drawing in its own shadow root is the button's inside).
- A field is never named by its own content (a textarea's text, a select's
  options, an editable box's or an ARIA textbox's, searchbox's or
  combobox's text). One with no label, title or placeholder is named by the text
  around it: the nearest of four ancestors that holds no other field, is not
  a field-and-button group, and has one or two runs of text outside controls,
  60 characters at most (a label with no `for`, a header cell in its row,
  text before or after it). An unnamed element is named by its picture's
  file stem when that reads as up to four words (`delete.png`, `#icon-star`).
- Text fields' values (never a secret's) are page text, where the field
  is; text a scroll box has scrolled out of view is not.
- A password input, an input masked with `-webkit-text-security`, and one
  whose `autocomplete` ends in `password` or `one-time-code` is a secret
  field: offered as a fill with `input_type` `password` or `one-time-code`
  and `filled` (whether it holds anything), its `value` always empty. Its
  content is never read into the observation, page text, guard, freshness
  key or effects, and a field once seen as a secret stays one (a "show
  password" toggle). Enter is offered in it once filled. Upstream, and Jev
  before, never offered them.
- A native select ignores opacity for visibility (one made transparent over
  a styled box), and a `display: contents` element is visible when its
  parent is and measured by its children, so a card-wide link and text in
  such a wrapper are reached.
- Strings are cut on whole characters and returned well formed, and the
  DevTools reader reads a lone surrogate escape as U+FFFD; a guard keeps
  only primitive values.
- Text fields carry `input_type` (`text`, `email`, `textarea`...).
- A select offers its current option too.
- Boxes that scroll inside themselves (overflow `auto` or `scroll`, on
  screen, at most ten) are offered as scroll targets, each with `scrolled`
  (`top`, `bottom` or tenths).
- Enter is offered in the field Jev last typed into, while it has focus and
  holds text; Escape while a popup, combobox list, dialog or floating list
  is open.
- `context.js` reads a short row whole ("Bob Lunch"), adds a second and third
  line when twins still read the same, and names unnamed twins laid out as a
  grid by row and column.
- `tree.js` walks open shadow roots (after their host) and visible
  same-origin frames (after the frame) for controls, page text, field values
  and the Escape check. Page text reads a shadow tree in place of its host's
  children and slotted nodes where their slot is. Frames are placed in
  window coordinates by their
  content box, scaled for a transform, and clip what they hold. A shadow host
  is named by its shadow content, and the shadow inside an offered control is
  not offered again. `act.js` hit-tests through `shadowRoot.elementFromPoint`
  and into frames, and focus is followed through both. The quiet clock
  watches every root walked. Closed shadow roots and cross-origin frames are
  not reached. Upstream walked the document only.

**Tabs (`page/tabs.rs`).** After every action settles, Jev lists the
browser's targets once and adopts the newest page one of its tabs opened
(`target="_blank"`, `window.open`). It repeats the tab setup and reads that
tab from then on, recording `opened_tab` on the step; the run moves to it only
once that setup succeeds. A tab that closes itself returns the run to its
opener. Every tab Jev owns is closed with the
page. Upstream read such a click as a no-op and left the tab open.

**Effects (`effects.rs`).** Each step records `effect`, what it visibly did
("went to …", "Size value: Small -> Large", "showed 2 controls: …",
"nothing visible changed"), next to `page_changed`; controls are paired by
node only within one document. It is on
`JevActionRecord` for embedders and not sent to the decision model yet (see
the docs).

**Operator limits (`scope.rs`, `runner/ceilings.rs`, `usage.rs`).**
`JEV_ALLOWED_ORIGINS`, narrowed by a call's `allowed_origins`, ends a run
`blocked` on any page outside it and is named in the approval request.
`JEV_MAX_ACTIONS` replaces the 60-action budget (model calls stay twice
that), and `JEV_MAX_SECONDS` caps `timeout_seconds`. The result carries
summed decision and text-helper `usage`, `"unknown"` where a provider did not
report a count; a billed call whose answer could not be used counts too.
Upstream has none of these.

**Launch (`chrome.rs`).** After the 9222 probe, a Chrome on Jev's profile is
found through its `DevToolsActivePort`. A launch uses
`--remote-debugging-port=0`, polls the child every 100 ms, sends stderr to
`jev-chrome/chrome-stderr.log`, and no longer sleeps 750 ms. Launches on the
profile are serialised (a process-wide lock and `jev-chrome.launch-lock`)
and re-check the profile once they hold the lock; the port file is never
deleted, and a launch that hands off to Jev's own running Chrome uses it. A profile Jev
creates starts with the password manager off.

**Statuses.** A run that stops early says why: `budget_exceeded` (60 actions
or 120 model calls), `timed_out`, `needs_input` (no text model, the text
helper's `{"text": null}`, or a password or code the goal does not hold;
the reason names the field), `unavailable` (a provider still failing after its
retries) or `error`, where upstream raises and Jev used to report `ready`;
with the gate on, `needs_confirmation`.
`blocked` also covers a start page that did not load. The tool result adds a
fixed `next_step` sentence for every status but `done`. Statuses serialize in
snake case.

**Setup.** `timeout_seconds` covers the whole task (starting Chrome,
connecting, loading, the first observation), cut to the host's remaining
deadline, and a phase that runs out ends `timed_out` naming it; upstream timed
only the loop. `Page.navigate`'s `errorText` is read: transient `net::ERR_…`
errors are retried twice (not `ERR_NAME_NOT_RESOLVED` or `ERR_ABORTED`), then
the task ends `blocked` with "could not load (…)" before any decision. The tab
is closed on every failure after it was created, including a deadline during
the attach, and a close is repeated until Chrome drops the tab; a close of a
tab Chrome already dropped succeeds. The start URL must parse as `http(s)` with a host.

**Endpoint.** `JEV_CDP_URL` (http(s) or a ws(s) browser websocket, opened
directly) replaces upstream's `BU_CDP_URL` and `BU_CDP_WS`, which the port
never honoured; they are not read. An http(s) address's path and query are
kept for the `/json/version` lookup. A non-loopback endpoint is reported only
as scheme and host.

**Dialogs (`cdp.rs`).** Upstream never handled JavaScript dialogs, so an
`alert()` or `confirm()` from a click blocked the renderer and the run ended
30 s later on an opaque timeout. Every DevTools call now answers the
`Page.javascriptDialogOpening` events it reads while it waits (the Page
domain is enabled on open): `alert` and `beforeunload` are accepted, `confirm`
and `prompt` dismissed. The next observation lists them as `dialogs` and as
lines such as `[confirm dialog, dismissed by Jev] Delete it?` before the page
text, after the fingerprint is taken, so a dismissed confirm is news for the
model but not progress; the loop records them on the step before them. A run
that ends `blocked` after a dismissed confirm or prompt says so in
`stopped_because`, telling the caller to take that step itself. No dialog
controls are offered to the model.

**Stopping (`agent.rs`).** Two rules found by the live eval corpus, neither
adding a model call:
- DONE straight after a covered attempt ends the run as blocked. Nothing was
  dispatched, and the model answered DONE there at 0.67 confidence.
- A repeat of the click that just changed the page, chosen with operation
  confidence under 0.7, ends the run as done without clicking. The model
  splits between that click and DONE once the page confirms the first one
  (0.43 to 0.66 across runs), while a click meant to repeat stayed above 0.75.
  The threshold was set on 13 tasks and held on the 18-task corpus in four
  live runs; re-check it when the corpus grows.

**Irreversible-action gate (`irreversible.rs`), opt-in.** Off unless
`JEV_CONFIRM_IRREVERSIBLE=1` or `JevEngineConfig::with_irreversible_gate`.
On, the same decision request also carries one Noul per action that may
commit something: every `PRESS_ENTER` target, then each click whose label
holds a commitment word as a whole word (`pay`, `order`, `checkout`,
`send`, `submit`, `delete`, `confirm`, `cancel` and 14 more), at most 8.
Each is keyed `irreversible_click_<index>` or `irreversible_enter_<index>`
and asks whether that element "would make a purchase or payment, send a
message, publish, delete, or make another change that cannot be undone";
it names the element by index and carries neither the goal nor the page.
When the chosen action is on the shortlist and its answer is above 0.5, or
missing or invalid (the gate fails closed), the run ends
`needs_confirmation` with nothing dispatched and `stopped_because` naming
the control, unless the call set `authorize_irreversible` and the call
confidence is at least 0.90. A call that sets it needs approval even in
accept-all mode. The shortlist, the question shape and both thresholds
(fastbrowse's) are unvalidated against the hosted model: the service
answered HTTP 402 while this was built. fastbrowse asks about every click
in a second request; this asks about a shortlist in the same one, so a
committing control whose label has none of the words is not gated.

**Cookie banners (`autoconsent/`, `refuse.js`, `consent.js`,
`page/consent.rs`), on by default.** Off with `JEV_REFUSE_COOKIE_BANNERS=0`
or `JevEngineConfig::with_cookie_banner_refusal(false)`. On, Jev's Chrome
runner injects DuckDuckGo's autoconsent (vendored unmodified, as fastbrowse
does) with `Page.addScriptToEvaluateOnNewDocument` before navigating, on
every tab it owns, adopted tabs included (for an adopted tab's already
loaded document it is also evaluated once). It runs in its own opt-out mode
and hides a known platform's banner until it has refused it. Unlike
fastbrowse, Jev's `autoconsent_setup.js` holds its heuristics to `reject`
right after it initialises: at its own `tier2` they pressed the lone Accept
of `cookie-accept-only.html`. `refuse.js` then orders the two: while autoconsent is
opting out, Jev waits for it (up to 3 s); an opt-out it reports is recorded
once per document, "refused cookie banner: <platform> (autoconsent)", and
the fallback does not run there; while its prehide hides an element, the
fallback waits for a later read; otherwise the fallback runs, once per
document, and if it clicks, autoconsent's action for that document is
cleared, so exactly one of them acts. Autoconsent's own work in frames is not
recorded. It costs 30 to 135 ms more to open a page (debug build, medians
of two runs, 10 to 28,600 elements), 0 to 70 ms on the first read of it and 0 to 45 ms on
a read while its detection still retries (about 10 s), and makes the
`roder` binary 445,824 bytes larger. The fallback, before the first observation of each
document and after it settles, is an in-page script (Jev's own) that looks
for exactly one visible dialog or banner region (a dialog role or element,
a fixed or sticky box, or a box named for cookies or consent) whose text
mentions cookies or consent and
that holds a button whose whole label is a refusal ("Reject all",
"Decline", "Only necessary", "Refuse", and German, French, Spanish,
Italian, Dutch and Portuguese equivalents), and clicks it. It never clicks
an accept or settings button, a link that navigates, or anything outside
the region, and does nothing when two regions qualify or the best refusal
is on two buttons. The loop records it as a step of kind `cookie_banner`,
"refused cookie banner: Reject all" (the label is page text), that costs no
model call and counts toward no budget or stall. It adds 1 to 10 ms to the
first read of a document without a banner (10 to 28,600 elements) and
under 1 ms to later reads. It reaches open shadow roots, but not frames,
closed shadow roots or a banner drawn after the page first went quiet.

**Acting (`act.js`, `page/act.rs`).** A click or a fill moves the pointer to
the hit-tested point, waits for a presented frame, and checks in one
evaluate that the control's own state (its guard without the surrounding
text) is unchanged and the point still hits it; a point that moved is
followed for up to 1 s before `StaleObservation` ("The target did not stop
moving"). Only then are press and release sent, so hover handlers run first
and a layout shift cannot send the press to what took the target's place.
Upstream pressed at once with no pointer move. A hidden tab, which draws no
frames and holds a pointer move's reply for 5 s, is pressed directly. This
costs a click about 30 ms (median 2 to 3 ms before). `act.js` itself is now a
pure hit test, and the page scroll is a script `window.scrollBy` (upstream
wheeled at a fixed point, scrolling whatever box sat there); choosing a select
option is `select.js`, which reports a
choice the page put back ("The page kept \"Medium\" instead of
\"Large\".") as a refused step instead of failing. A target that is not
fully visible is scrolled into view first. Up to five points on up to four viewport-clipped rects are hit
tested, and a point counts only when no other control sits between the hit
and the target. When none does, the target is scrolled into view inside its
own scroll containers and tried again, and a nested control that fills the target (a
combobox's own input) is accepted. A covered target returns the public
`Covered` error; the loop records it as a step with `page_changed: false`,
`text: null` and `covered: true` (serialized only when true), so three in a
row end the run `blocked`. Upstream checks only the centre, and treats a
covered target as a stale page. A scroll box is scrolled by script, by four
fifths of its height and at least 320 px, and a box that did not move is a
refused step. Enter focuses its field (without selecting it) and sends a
key press with `\r`; Escape goes to whatever has focus. Upstream had
neither.

**Typing (`fill.js`, `page/fill.rs`).** Upstream selected a field's content
with the platform's select-all accelerator, which editors and grids
intercept, and never checked what landed. A fill now follows focus from the
clicked field to a visible, enabled, editable field covering the click point
(an editor the click opened over it), waiting for up to 0.6 s of quiet;
focuses and selects it by script (`select()`, or a range for
`contenteditable`); inserts the text; and checks that the field holds exactly
that text. It types once more only when focus has since moved to another
field. A native date or time input gets its ISO value through the value
setter, with `input` and `change`. A field that does not keep the text is a
refused step ("The field shows …"), not an error. The check compares what
the field keeps of the text: typed into a single-line input, a line break
becomes a space (and an email or URL input trims the ends), so a multi-line
value that landed is not refused, a field that reformats the text (a phone
mask) kept it when its letters and digits match, and a date or time is
compared as the moment it names. A fill whose press navigated is a refused
step, not a stale decision. This costs a fill about
150 ms (median 4 to 5 ms before), most of it the hand-off's 100 ms of quiet.

**Refused actions.** `JevBrowser::act` returns a `JevActOutcome`; a refusal
is recorded on the step (`refused`, shown in the tool result when set) and
the run goes on, so a page that keeps refusing ends through the stall rule.
It is not sent to the model, which sees the page as it stands.

**Decision request.** `offscreen` and `context` join the element and target
keys, and TARGET gains two sentences, one for each. `input_type` (except a
native date or time input's: told `date`, the model clicked the field to
open a picker that never appears) and `scrolled` join them too, with no
prompt change. Three operations appear only when offered:
`SCROLL_REGION_DOWN` and `SCROLL_REGION_UP` (targets: the scroll boxes) and
`PRESS_ENTER` (target: the field Jev typed into), plus a `PRESS_ESCAPE`
control. The state carries today's
`date` first, in fastbrowse's words ("2026-09-27 (Sunday). In one month:
October 2026; in two months: November 2026"); live corpus runs with it passed
18/18 three times, like the two without, though the mean call confidence was
0.78 against 0.80. Each operation offers at most 255 targets. `prompts.json`,
`choose_request.json` and `action_space.json` were re-recorded for these;
`fingerprint.json` is still upstream's output.

**Text helper.** The field context adds the same `date` after `goal`, the
field's `input_type` and `context`, a native date or time field's ISO
`format` (such as "YYYY-MM-DD (ISO 8601, such as 2026-09-25)"), and
`other_fields`: up to 20 other text fields and selects with their label,
kind, context and current value (1,000 characters each), so a value can be
copied from a textarea and a field placed in its form. `field_context.json` and
`field_text_requests.json` were re-recorded for the date and again for
`other_fields`.

**Tab.** A foreground task's tab is shown when it is created, before the page
loads; upstream's port showed it only after the first observation. A
background task's tab is never brought forward. Measured on a windowed
Chrome, a background tab with focus emulation (which Jev sets) reports
`visible`, runs frames and 50 ms timers on time; without it, it is hidden,
runs no frames, and a 50 ms timer takes 1 s.

**Transport.** One `reqwest::Client` per transport instance is reused, not a
new one per call. Decisions get six attempts of 15 s each, with waits of 0.5,
1.5, 4, 8 and 8 s, on 408, 429, 500, 502, 503, 504, 520 to 524 and 529 and on
failed connections, honouring `retry-after` up to 10 s. A timeout, a dropped
reply, a body that is not JSON, or a 500, 504 or 524 may already have been
billed, so it is sent again at most once. No retry starts that would outlast the run, so the
provider's failure is reported rather than the run's timeout. The text helper
uses the same rules with three attempts. Upstream made three attempts on 429,
503 and 529 with a 25 s timeout, and one text-helper attempt. An undecodable
reply now fails as "Invalid TypeSafe response; no action executed.", and a
422 carries its trimmed body.

**Secret fields (`secret.rs`, `agent/typing.rs`).** A value typed into a
password or one-time-code field is recorded as `[secret]` (no length) on the
step, in the text-helper record, and so in the decision request's and the
text helper's recent actions, the tool result and eval rows. The fill check
for such a field runs in the page and returns only whether the text stayed,
and a refusal does not quote the field. The loop also replaces the typed
value (4 characters or more) with `[secret]` in everything it reads from the
page afterwards (address, title, text, labels, values, contexts, dialogs,
refusals, `stopped_because`), for a page that shows it back. Such fields are
left out of the text helper's `other_fields`. The text helper answers for a
secret field only with a value written in the goal; anything else ends the
run `needs_input` naming the field. The irreversible-action gate treats
Enter in, or a submit click after, a filled password like any listed
action.

**Public API.** New `JevOriginScope`, `JevUsage`, `JevCallUsage`,
`JevTokenCount` and `JevBilled`; `JevEngineConfig::with_max_actions`, `with_max_duration` and
`with_scope`; `JevRunResult.usage`; `JevActionRecord.opened_tab` and
`effect`. New `Covered` error; `JevActionRecord.covered`,
`target_confidence`, `refused` and `dialogs`; `JevDecision.target_confidence`,
`model` and `call_confidence()`; `JevDecisionRecord.target_confidence` and
`model`. `JevBrowser::act` returns the new `JevActOutcome`, and an
observation may carry `dialogs` (the new `JevDialog`).
`JevBrowser::activate` and `JevEngine::activate` are gone: the built-in
runner shows its tab when it creates it, and a hosted browser shows its own.
`JevStatus` is `#[non_exhaustive]`, serializes in snake case and gains
`BudgetExceeded`, `TimedOut`, `NeedsInput`, `Unavailable`, `Error` and
`NeedsConfirmation`; the new `JevStop` error lets a hosted client, transport
or resolver end a run with one of them. For the gate and banner refusal:
`JevEngineConfig::with_irreversible_gate`, `with_irreversible_authorized`
and `with_cookie_banner_refusal(bool)` (on unless given `false`); `JevDecisionClient::choose_gated` and
`JevBrowser::refuse_cookie_banner`, both provided methods (a client that
keeps the default asks nothing, so the gate stops at every shortlisted
action it chooses; a browser that keeps it refuses nothing);
`JevDecision.irreversible` and `JevDecisionRecord.irreversible`; and the
tool's `authorize_irreversible` argument. These are breaking changes, with
no compatibility shims.

**Tests.** The scripted agent-loop fakes live in `tests/support/`, and the
real-DOM harness and eval corpus in `src/fixture_harness/`, rather than in
`tests/injected_engine.rs`. Fixture pages under `tests/fixtures/pages/` are
Jev's own test inputs, not upstream recordings.

## Hosted Browser Adapters

`JevEngine` exposes the bounded loop independently from the built-in Chrome
connection. A hosted runner can implement `JevBrowser` with its supervised
browser lease, use `JevTypeSafeDecisionClient` (or provide its own
`JevDecisionClient`), and optionally provide a `JevTextValueResolver`:

```rust,no_run
use std::{sync::Arc, time::Duration};
use roder_ext_jev::{
    JevBrowser, JevDecisionClient, JevEngine, JevEngineConfig, JevRunResult,
    JevTextValueResolver,
};

async fn run_hosted(
    browser: Box<dyn JevBrowser>,
    decisions: Arc<dyn JevDecisionClient>,
    values: Arc<dyn JevTextValueResolver>,
) -> anyhow::Result<JevRunResult> {
    let config = JevEngineConfig::new("Verify checkout", decisions)
        .with_text_resolver(values);
    let mut engine = JevEngine::start(browser, config).await?;
    Ok(engine.run(Duration::from_secs(120)).await)
}
```

The browser adapter owns navigation, action execution, screenshots, video,
and tracing. Its `act` returns a `JevActOutcome`, refused when the page did
not keep the action's effect, and its observations may list the JavaScript
dialogs it answered under `dialogs`; the loop records both on the step. A text resolver may return an opaque value reference for the
browser supervisor to resolve at execution time, keeping secret material out
of JEV observations and results. `JevTypeSafeDecisionClient::with_transport`
also lets a hosted runner send the upstream request through its own
authenticated relay while the extension retains question construction and
response validation.

## End-to-End Evals

`tests/fixtures/evals/tasks.json` is a small corpus of browser tasks on local
fixture pages (`tests/fixtures/pages/`). Each task has a natural-language
`goal`, the field `values` that goal supplies, an `expect` block graded on
what a user could observe afterwards (final status, stop reason, final URL and
title, visible text, the recorded form POSTs, and DOM values read from the
final document), and a `script`: the plan a keyless decider plays, plus the
trace that plan must produce (actions, model calls, covered and page-changed
flags). A fixture server on `127.0.0.1:0` serves the pages and records every
POST; Chrome is a throwaway headless instance on a temporary profile. The
keyless tier runs tasks one at a time in one Chrome, while the live tier gives
each task its own Chrome.

| Task | Covers |
|---|---|
| `contact_form` | three fills and a submit; the values reach the POST |
| `search_autocomplete` | typing a query, picking its suggestion, searching |
| `select_dropdown` | a native select option, then submit |
| `pagination` | two pager clicks to open an item on page 3 |
| `below_the_fold` | an offscreen target clicked with no scroll step |
| `covered_target` | a target under a modal backdrop stops cleanly, unclicked |
| `twin_by_context` | one of eight identical buttons, chosen by its card |
| `table_row_by_context` | one of four identical links, chosen by its row |
| `delayed_spa` | an app that renders each view late, behind a spinner |
| `mid_step_navigation` | the page navigates mid-decision; the stale act is refused |
| `stall_detection` | three clicks that change nothing end as `blocked` |
| `step_budget` | a run that keeps progressing stops at 60 actions |
| `missing_value` | a value the goal lacks stops the run before any POST |
| `confirm_dialog` | a delete's confirm is dismissed; the run ends `blocked`, saying why |
| `menu_button` | a menu that arrives 500 ms after its button is clicked, then an item |
| `styled_checkbox` | a filter whose hidden checkbox only its label can toggle |
| `date_field` | a native date input set to its ISO value, then submitted |
| `focus_handoff` | typed text follows focus to an editor opened over the field |
| `icon_by_picture` | a div-and-span mail list: an unnamed trash icon in the right row |
| `unlabelled_fields` | a form whose fields are named only by the text around them |
| `scroll_region` | terms that unlock "I agree" once their own box is scrolled to the end |
| `enter_to_search` | a search form with no button, submitted by Enter in its field |
| `escape_popup` | a suggestion list covering the next button until closed or used |
| `shadow_component` | product cards drawn in open shadow roots; one card's button by its context |
| `frame_form` | a signup form, with a native date field, inside a same-origin iframe |
| `new_tab` | a report link that opens a new tab; the run follows it and acts there |
| `off_origin` | a link to another origin ends a scoped run as `blocked`, saying why |
| `gate_pay_now` | with the gate on, an unauthorized run stops before Pay now, undispatched |
| `gate_delete_account` | the gate stops before Delete account; keyless, with no valid answer (fails closed) |
| `gate_authorized_delete` | an authorized, confident run gets past the gate; the delete is posted |
| `gate_add_to_cart` | a harmless Add to cart is not asked about and goes through with the gate on |
| `cookie_banner_refusal_off` | refusal turned off: a consent banner covers the target, `blocked` |
| `cookie_banner_refused` | by default, `consent.js` clicks the banner's Reject all, uncovering the target, with no model call |
| `cookie_accept_only` | a banner that only offers Accept is left alone |
| `cookie_no_banner` | a page with no banner (cookies as food, a Decline button) is left alone |
| `cookie_platform_refused` | autoconsent refuses a known platform's banner (WP Cookie Notice for GDPR markup) |
| `cookie_platform_off` | refusal turned off: nothing injected, the platform's banner covers the target |
| `login_form` | a username and a password from the resolver, submitted by Enter; graded on the POST |
| `one_time_code_resolved` | a one-time code from the resolver, typed and posted |
| `one_time_code_missing` | no code anywhere: `needs_input` naming the field, nothing typed |

`icon_by_picture` to `escape_popup` cover generic clickables, field naming,
scroll boxes and keys. With `jev-1.13.0` only `unlabelled_fields` of them
passes live: the model never presses Enter or Escape,
does not pick the suggestion, stops two scrolls short of the end of the
terms, and deletes Bob's second mail after the first. They stay in the
corpus as the record of those gaps. The last seventeen have not run live
yet: the decision service returned HTTP 402 to every request while they
were added. The `gate_` tasks turn the gate on themselves
(`confirm_irreversible`, `authorize_irreversible` in `tasks.json`), the
`_off` cookie tasks turn refusal off (`refuse_cookie_banners: false`; every
other task runs with it on, autoconsent injected), and a plan step may give
the gate's answer as `irreversible`. A task's expected POST may name
`secret_fields`, whose values a failure never quotes.

**Keyless tier.** Runs in `cargo test` with no model key: the scripted plan,
the real agent loop, the real page scripts and real Chrome. It checks the
browser layer and the loop, not model quality.

Every Chrome-backed test (this tier and the real-DOM fixture tests) passes
without running when no Chrome is found, and the run says so once on the
terminal. Set `JEV_CHROME_BINARY` to point at one; an explicit value that is
not an executable file fails the tests. Set `JEV_REQUIRE_CHROME=1` (CI's own
`CI` variable does the same) to fail instead of skip: CI should run with it
and Chrome installed.

```sh
cargo test -p roder-ext-jev keyless_corpus -- --nocapture
JEV_EVAL_TASKS=contact_form,delayed_spa cargo test -p roder-ext-jev keyless_corpus -- --nocapture
```

**Live tier.** The hosted decision model on the same pages, graded by the
same `expect` blocks (the plan-specific `script` checks are not applied). It
is `#[ignore]`d and skips without a key:

```sh
TYPESAFE_API_KEY=… cargo test -p roder-ext-jev live_corpus -- --ignored --nocapture
TYPESAFE_API_KEY=… JEV_EVAL_VARIANTS=goal_in_state,handoff_nouls \
  cargo test -p roder-ext-jev live_corpus -- --ignored --nocapture
```

- `TYPESAFE_API_KEY` (or `JEV_API_KEY`) is the decision key; `JEV_MODEL` pins
  a model version, else `jev-latest`.
- Field values come from each task's `values`, so a run measures the decision
  model alone; `JEV_EVAL_TEXT=model` uses the configured text helper instead.
- `JEV_EVAL_VARIANTS` turns on request variants from the design audit, for A/B
  runs: `no_context` (strip `context` and `offscreen`, the baseline for twin
  disambiguation), `structured_criteria` (`{what, not_for}` operation
  criteria), `goal_in_state` (the goal once in `state.task`, rules keyed),
  `effect` (each recent action's effect after `page_changed`; MiniWoB++
  honours this one too), and three shadow variants whose answers are recorded but gate nothing:
  `handoff_nouls`, `none_target` and `irreversible_nouls` (the production
  gate's own questions, plus whether the goal authorizes a commitment). All
  keep one request per step.
- `JEV_EVAL_CONFIRM_IRREVERSIBLE=1` runs every task with the gate on, to
  measure what turning it on by default would cost, and
  `JEV_EVAL_REFUSE_COOKIE_BANNERS=0` every task that does not set it with
  banner refusal off, to measure what the default costs; each row names
  them under `variants`, and live rows add each decision's `irreversible`
  answer.
- `JEV_EVAL_DUMP=1` prints every observation's actions and text, to see
  what the model was offered.
- `JEV_EVAL_TASKS` narrows the run, `JEV_EVAL_CONCURRENCY` (default 2) sets
  how many tasks run at once, and `JEV_EVAL_STRICT=1` fails the test when a
  task fails instead of only reporting it.

Both tiers print a pass/fail table and write one JSONL line per task to
`target/jev-evals/` (`keyless.jsonl`, or `live-<unix seconds>.jsonl`) with the
status, steps, model calls, wall time, pass/fail and failures; live rows add
per-step confidence, the answering model, request size, input tokens and the
shadow answers. To add a task, add a page if none fits and an entry to
`tasks.json`; a unit test checks every entry parses and names a real page.

### Public benchmark: MiniWoB++

`miniwob_corpus` runs the hosted model on
[MiniWoB++](https://github.com/Farama-Foundation/miniwob-plusplus) (MIT),
pinned at `33c3b4ddef8c6eb67c57a29663d844b1eda7e614`. The benchmark is not
vendored: clone it anywhere and point `JEV_MINIWOB_DIR` at the checkout. Only
its `miniwob/html/` folder is served, from `127.0.0.1:0`, to the throwaway
headless Chrome; none of its Python runs.

```sh
git clone https://github.com/Farama-Foundation/miniwob-plusplus
git -C miniwob-plusplus checkout 33c3b4ddef8c6eb67c57a29663d844b1eda7e614
TYPESAFE_API_KEY=… JEV_MINIWOB_DIR=$PWD/miniwob-plusplus JEV_MINIWOB_ATTEMPT_UNSUPPORTED=1 \
  cargo test -p roder-ext-jev miniwob_corpus -- --ignored --nocapture
```

`tests/fixtures/evals/miniwob_tasks.json` lists all 130 tasks at that commit:
129 are run and `text-transform` (a distorted-text CAPTCHA) is excluded. Each
is labelled `supported` or not, with the reason Jev cannot express it (a
coordinate click, a drag, a hover, an answer only drawn in a picture, and
so on); 88 of the 129 are supported, 36 of them relabelled when generic
clickables, field naming and scroll boxes made them expressible, and
`enter-password`, `login-user` and `login-user-popup` when password fields
became fillable (not yet run: HTTP 402). An unsupported task is reported as such and
counts as a failure in the overall score; with
`JEV_MINIWOB_ATTEMPT_UNSUPPORTED=1` it is also run, to check the label.

Each episode opens `/miniwob/<task>.html`, waits for the task to load, seeds
`Math.random` with `Math.seedrandom('<seed>')`, sets `core.EPISODE_MAX_TIME`
to 1,000,000 ms, makes `core.startEpisode` a no-op (so the START cover never
returns), calls `core.startEpisodeReal()` and `core.clearTimer()`, and hides
the reward display, click canvas and cover. They are hidden, not removed:
`core.endEpisode` writes to the display after latching the reward. The goal
is `core.getUtterance()`. The harness stops the run once `WOB_DONE_GLOBAL`
is set, as BrowserGym ends an episode, and success is
`WOB_RAW_REWARD_GLOBAL > 0`. Typed values come from the configured text
model, since they change with the seed; the model is recorded on every row.

- `JEV_MINIWOB_SEEDS` (default `0-4`): a range or a comma list.
- `JEV_MINIWOB_TASKS=a,b` narrows the run.
- `JEV_MINIWOB_MAX_STEPS` (default 25) caps decisions per episode and
  `JEV_MINIWOB_TIMEOUT_S` (default 90) caps each episode's loop.
- `JEV_MINIWOB_CONCURRENCY` (default 4) sets how many episodes run at once;
  each worker has its own Chrome.
- `JEV_MINIWOB_DUMP=1` prints every observation's actions and text.

Rows stream to `target/jev-evals/miniwob-<unix seconds>.jsonl`, one per
episode (task, seed, supported, success, raw reward, reward reason, Jev
status, steps, model calls, text calls, wall time, stop reason, the executed
trace and the first observation's actions). The printed summary gives the
overall score, the score over supported tasks, per-task success rates and the
failure causes.
