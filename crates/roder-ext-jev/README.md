# roder-ext-jev

`roder-ext-jev` is the goal-directed browser tool for [Roder](https://roder.sh).

## What It Does

Exposes a `jev_browse` tool that runs a bounded browser task against Chrome over
the DevTools Protocol: given an observable goal (and, on a thread's first call,
a starting URL), it observes the page, chooses one operation at a time,
executes it, and returns the action trace with the observed final page. Each
Roder thread keeps one Jev browser session: later calls go on in the same tab
from where the last one stopped.

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
- A thread's calls share one session and tab (see
  [Sessions](#sessions)); the tab is in the foreground by default and stays
  open between calls.
- Typing is served by Roder's own configured chat-completions provider.
- JavaScript dialogs are answered: alerts accepted, confirms and prompts
  dismissed, and each is reported.
- Page content is untrusted: a `done` result is an agent claim to be checked
  against the observed page, which the result text shows (see
  [The result](#the-result)).
- A site that refuses automated access (HTTP 401, 403 or 429, a challenge
  or "unusual traffic" page) ends the call `access_denied` before any
  decision, with the evidence. Jev only reports it; it never tries to get
  around a block.
- Frames of another origin (a booking or payment widget) are read, never
  acted in: their text reaches the model and the result, their buttons are
  not offered (`JEV_FRAME_TEXT=0` turns the read off).
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
- A target covered by a popover, menu, picker or dialog (one Jev opened
  itself, or any laid over the page) is uncovered first: Escape, the layer's
  own close control, or a press outside it, then the action goes ahead in
  the same step (see [Uncovering a target](#uncovering-a-target)).
- When Jev cannot progress, Roder's full browser tools go on in the same tab
  (see [Fallback](#fallback)): inside the call, driven by the session's
  model (`JEV_FALLBACK=auto`, the default), or handed to the caller as the
  `jev_tab_*` tools.

Requires `JEV_API_KEY`, or `JEV_DECISION_PROVIDER=openai` with an OpenAI API key
(see [OpenAI Decisions](../../docs/openai-decisions-browser.md)). Decisions uses
native questions, action-effect history and fresh screenshots when available;
`JEV_DECISIONS_TEXT_ONLY=1` disables screenshots. Recognized secret fields and
previously typed secrets suppress capture. In Roder's default policy mode each goal needs approval
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


## Sessions

Each Roder thread has one Jev browser session (`src/session/`), kept in a
process-wide registry keyed by thread id; a subagent's thread has its own.

- **One tab, reused.** The first call opens a tab and loads its `url`. Later
  calls with `url: ""` go on from the page the tab shows, without reloading;
  a `url` loads in the same tab, and is not reloaded when the tab is already
  there. `tab: "new"` opens a second tab beside the first, `tab: "reset"`
  closes the session's tabs and starts over, and `tab: "close"` closes them
  and browses nothing. A tab an action opens is adopted and stays in the
  session. At most three tabs stay open; the oldest beyond that is closed,
  never the current tab or the one that opened it. Tabs have short ids
  (`t1`, `t2`, …) in results. A call with no url on a thread without a tab
  fails without browsing, and its text tells the caller to call again with
  the page to start on. `tab: "new"` right after a call whose page refused
  access loads the url in that tab instead, since a refused page holds
  nothing worth keeping; if the user has closed that tab, the call opens the
  new tab it asked for rather than loading over the tab before it.
- **Target ids, not sockets.** The session keeps its tabs' target ids.
  `cdp.rs` has no reader task, so an idle socket would queue events unread
  and leave dialogs unanswered; each call opens a new connection,
  re-attaches (`Page::resume`) and sets each tab up again (viewport, focus
  emulation, the Page domain, the quiet clock, autoconsent for later
  documents), since those end with the DevTools session. Measured under
  300 ms on the fixture page (the suite's test asserts only a 2 s ceiling,
  since its tests run in parallel on one shared Chrome). Every page already open at that
  point is recorded as seen: a tab the user opened from Jev's tab between
  calls (a `target="_blank"` click, a page's timer) is theirs, never adopted
  as the run's tab or closed as a stray; only tabs this call's own inputs
  open are followed.
- **The user moved or closed the tab.** A tab found somewhere else than the
  last call left it is reported ("changed since the last call") and Jev goes
  on from there; that address is compared, and reported, with the
  session's typed secrets scrubbed from it (a GET form's address), so a
  password never comes back through it. A closed popup hands the call back
  to the tab that opened it. When every recorded tab is gone (closed, or
  Chrome restarted) the call opens a new tab at its url or the last one and
  says so (`reopened`, with the reason); with `JEV_ALLOWED_ORIGINS` set, a
  last page outside those origins is not reopened (the call ends
  `blocked`).
- **Carried between calls:** the last eight calls (goal, start and end url,
  title, status, actions, reason), running totals, the typed secrets (still
  scrubbed from later page reads) and the resolved models. Nothing new goes
  to the decision model or the text helper: a continued call's first
  decision request is the same as a fresh call's, and the page itself
  carries the context.
- **One call at a time per session.** A call waits for the one before it,
  within its own deadline, and says how long it waited; one whose deadline
  passes first returns `status: "busy"` and does nothing. Threads run side by
  side.
- **Cleanup.** A session idle for `JEV_SESSION_IDLE_SECS` (default 1200) is
  closed by a sweeper that runs every minute; a ninth session in the process
  pushes out the least recently used idle one. Each process writes its
  sessions' tabs to a ledger in `jev-sessions/` under Roder's config
  directory (loopback endpoints only); a ledger nobody has refreshed for
  twice the idle limit belongs to a dead process, and its tabs are closed.
  Only tabs Jev created or adopted are ever closed. A session leaves the
  registry only under its own lock and marked closed, so a call that was
  waiting for it moves to the thread's new session instead of running on
  one whose tabs nothing would close.
- **Result.** Every result's text names the tab and how the call came to be
  on it, the tabs open, the session's totals, up to four earlier calls, and
  how to go on; `data.session` holds the same. The tool description states
  today's date and the local time zone.

## The result

Roder hands the model a tool result's text, never its data, and the text used
to be one line ("done at <url> (3 actions)") that told the caller to read a
`visible_text` it never received. It is now a digest of the call
(`src/report/digest.rs`), at most 8,000 characters and 120 lines (Roder moves
an output above 20,000 characters or 200 lines into a file):

- **Header:** the status; the session call and tab; today's date, time and
  time zone ("Tonight" means this date, or, when a flow ran past midnight,
  the date the session began on); the address and title, marked
  page-supplied, and the HTTP status; what came of the call (actions,
  decisions, seconds); a `Not offered to Jev:` line when the page held
  controls or select options Jev was not offered (counts only: see `omitted`
  below); what to do next. After `done` that is to check the page
  against the goal, to go on with `url ""`, and not to sign in, reserve, pay
  or send personal details unless the user asked for that step.
- **Session:** tabs open, totals, up to four earlier calls.
- **Page content**, between fixed marker lines that say it is untrusted
  (any run of three or more dashes in page text, ASCII or look-alike,
  becomes "- -", so no page line can draw a marker): why Jev stopped, the steps with
  their effect and the section each control sat in, the click a `done` run
  declined to repeat (`Not clicked:`), the text of frames Jev
  read, the visible headings, the page text (blank and repeated lines
  dropped, short ones joined), and the options Jev can act on (a checkbox,
  radio or switch reads `[checked]` or `[unchecked]`), on screen
  first, grouped by the card or section they sit in (a twin's `context`,
  else the nearest heading before it, which `context.js` now records for
  every control as `section`, outside the fingerprint and the requests; a
  heading that titles one card of a list does not name a control after the
  list); links repeated more than three times are left out as navigation.

Each section has a budget, and what is left of the whole goes to them in
priority order (frames, steps, the click not made, session, headings, options, text, with at least
900 characters kept for the text); every cut says how much it left out.
Measured on the fixture pages through the session layer: 1,211 characters on
`basic.html`, 4,973 (48 lines) on `big.html` with 249 controls, 2,472 on the
reservation results after a slot opened its booking widget; a synthetic page
with 221 controls, 6,000 characters of text, two frames and 30 steps stays at
7,930. `data` keeps everything: every step's `effect` and `context`, the
final page's `controls` (label, kind, role, context, section, value cut to 60
characters, a select's options, and `checked` true or false for a checkbox,
radio or switch, which then has no `value`; never a secret's value), `omitted`
(`controls` the snapshot left out and `options` past the 255 a choice takes;
absent when both are zero) and `page` (`http_status`, `headings`, `frames`).

`JEV_SESSION_LOG=<dir>` appends one JSON line per call to
`<dir>/<thread>.jsonl` (the request, the tab, the full data and the text), for
grading a run afterwards: `roder exec --json` carries only the text.

**Completion check.** `success_condition` (`url_contains`, `text_contains`,
`text_absent`; `src/session/completion.rs`) is checked once, after `done`, on a
fresh look at the tab. Both sides of each comparison are folded first
(`completion/text.rs`): case, whitespace runs (including the line break
`text.js` puts between nodes, and no-break spaces) and zero-width characters do
not matter, so `count: 1` matches `Count:` over `1`. `text.js` now returns the
field values it listed in the page text (`typed_values` on the observation,
`FinalPage.typed_values`, scrubbed like the text), and the check leaves one
matching line out per value: what a field holds is not the page's own text.
A predicate with nothing visible in it is an error, an empty one is skipped.
`completion_verification` carries `status`, the three predicates, `unmet` and
the scope; the digest line says "condition met" or "condition not met" and
names the unmet predicates. Step two, ending a run on the first look that
satisfies `text_contains`, is not built: it needs a live A/B.

After `access_denied` the hint says: if the user asked for that particular
site, tell them and ask how to go on; otherwise the task is not finished: go
on at another site that offers the same thing, with its url and tab
`current` (it loads in the same tab), without asking the user first; tell
the user only when no other site will do. The tool description and schema
carry no benchmark wording: the goal's example is a catalogue task, and a
unit test keeps the booking task's words out of them.

### Live booking benchmark

`scripts/jev-booking-bench.sh [--site resy|tock] [--runs N] [--model M]
[--prompt TEXT] [--earliest HH:MM]` runs a restaurant-booking request
through the installed `roder` binary (`roder exec --json`, Jev's own
profile, the irreversible gate on, `JEV_SESSION_LOG` in the output
directory) and `scripts/jev_booking_grade.py` grades each run: reached the
reservation panel (a listing of slots is reported as `listed_only`, a
partial result that does not pass); the panel's own date, party size and
time (or the address's `date=` and `seats=`) are today, 3 and in the window,
and the run's pages named the area (`--area`); the answer names the panel's
restaurant and a time the panel showed; no commit, sign-in or
personal-data step; one tab across the calls. See "Live booking benchmark" in
[docs/jev-browser.md](../../docs/jev-browser.md) for the criteria and the
eight runs that shaped the fixes above (`fixture_harness/booking_tests.rs`).

## Fallback

Jev acts only on the controls its snapshot offers, with a small action
vocabulary: no screenshots, coordinates, hover, drag or arbitrary keys. The
last live booking run showed where that ends: Jev opened a date picker, the
picker stayed open over the results, every slot click was covered, and the
run stopped `blocked`. When a run ends like that, Roder's own direct CDP
tools (`roder_ext_chrome::direct`) go on **in the same tab**, with its
cookies and page state, as `JEV_FALLBACK` says (`src/fallback/`):

- **When** (`trigger.rs`): the run ended `blocked` because the model answered
  BLOCKED, three steps changed nothing, its targets stayed covered, it
  answered DONE after a covered attempt, it went round in circles
  (`stop_cause` `looped`) or the page never held still for it (`unsettled`);
  the page offered nothing Jev can act on; its own action budget ran
  out; or the run ended `error` because the decision service kept sending
  replies Jev could not use (`stop_cause` `decision_unusable`, trigger kind
  `decision_unusable`). That is the only `error` that falls back. Never after
  `needs_input`, `needs_confirmation`, `access_denied`, a page that did not
  load, a page outside the allowed origins, a confirm or prompt Jev declined,
  a provider failure (unreachable, a refused key, billing or access, a rate
  limit), any other error or a timeout: a different driver does not fix
  those, and the fallback must never be a way around a block or a
  confirmation. `JevRunResult.stop_cause` (new) says which rule ended a run.
- **`auto` (default).** `jev_browse` itself runs a bounded loop (`run.rs`):
  the model reads the tab and calls the full tools (look, screenshot, click at
  a ref or x/y, hover, drag, type, key, scroll, select, navigate, wait) until
  it answers DONE, BLOCKED, NEEDS_INPUT or NEEDS_CONFIRMATION, a rule stops
  it, or a ceiling is reached. It is driven by the model the session is on:
  the Jev extension is given Roder's inference engines when installed (as
  the subagent dispatcher is) and finds the engine by the turn's provider.
  `JEV_FALLBACK_MODEL` (`provider/model`, or a catalog model id; an OpenAI
  model goes through the ChatGPT/Codex sign-in when Roder holds one) pins
  another. The turn's reasoning effort is not handed to tools, so the effort
  is `JEV_FALLBACK_REASONING` (default `low`). A model whose engine runs its
  own agent (Claude Code, Cursor) or takes no tool calls cannot drive it; the
  call then hands over, saying why. The result is one: its `status`, page
  and time are the call's end state, `jev_status` is Jev's own, and
  `drivers` gives each driver's steps, model calls, time and tokens, so the
  fallback's cost is never read as Jev's speed. The text says the same
  ("Drivers: Jev 5 actions, 5 decisions, 3.1 s; fallback (codex/gpt-6-sol
  (low)) 2 tool calls, 3 model calls, 11.0 s, …") and lists the fallback's
  steps among the page content. After it, the tab is read again as Jev reads
  it, so the page, frames and options in the result are Jev's own reading.
- **`handover`.** The result tells the caller that the full tools work on
  this same tab, names them and the tab (`jev_tab_look`, …, `jev_tab_wait`;
  `t1`), says what Jev did and why it stopped, and tells it to go on with
  them instead of retrying `jev_browse` with the same goal or giving up.
  `auto` hands over the same way when no model can drive it or the fallback
  also fails.
- **`off`.** Jev's result as before.

The `jev_tab_*` tools are always registered with `jev_browse` (no `--chrome`
needed) and drive only the tab this thread's Jev session is on, under the
session's lock: they take no tab argument, cannot open or list tabs, and a
thread with no Jev tab gets an error telling it to start with `jev_browse`.
A tab one of them opens is adopted by the session, a secret typed through
them is scrubbed from later reads, and Jev's next call goes on where they
left the tab. `JEV_SESSION_LOG` logs each call.

**Rules inherited (`guard.rs`).** The operator's `JEV_ALLOWED_ORIGINS`
(a navigation outside is refused before it loads; a click that lands outside
ends the fallback `blocked`); the irreversible-action gate, which with no
model question stops at every control its shortlist names (a click whose
label holds a commitment word, a form's submit once a password is filled in
it, Enter in a form that pays or signs in), stricter than Jev; the call's
`authorize_irreversible` counts only when the fallback also marks that press
(and the fallback model is offered that switch only on an authorized call);
Jev's reading of an access block (the fallback stops `access_denied`, never
works around it); a covered control is never pressed, by a click or by
Enter or Space while it has focus; with banner refusal on a cookie banner
may only be refused, and with it off nothing in one is pressed. Secrets: what a password or one-time-code field holds is
never read; what is typed there is reported as `[secret]` and scrubbed from
every later read; a screenshot blacks out every filled secret field on
screen, and none is taken while the page shows a typed secret anywhere.
There is no script evaluation. Page content is marked untrusted (so are Jev's
stated reason and last steps in the fallback's opening message, which can quote
page labels), and the fallback model is told never to follow it, never to sign
in, reserve, pay or send personal details unless the goal asks for that exact
step, and never to solve a CAPTCHA. Policy: `jev_browse`'s approval says the session's model may
go on (auto); the `jev_tab_*` tools that only read (look, screenshot, wait)
run in every mode, and those that act follow `jev_browse` (plan mode denies,
default mode asks for each, accept-all runs unless `authorize_irreversible`).

**Ceilings.** `JEV_FALLBACK_MAX_STEPS` (tool calls, default 20),
`JEV_FALLBACK_MAX_SECONDS` (default 120, within what is left of the host's
deadline) and `JEV_FALLBACK_MAX_TOKENS` (input and output, default
400,000). One that is reached ends the fallback `budget_exceeded` or
`timed_out`, naming it. Older page reads are cut to one line before each
model call and only the newest screenshot is still shown.

## Uncovering a target

A covered target used to be a step that changed nothing. Now, when act.js
finds it covered by a layer laid over the page (positioned fixed, absolute or
sticky, a top-layer dialog or popover, or a dialog, menu, listbox or tooltip
role), `page/uncover.rs` tries, checking the target after each: Escape; the
layer's own close control (a button whose whole name is a close word, or
whose label says close or dismiss; never accept, cancel or done); a press
outside it on nothing that acts, or on a full-page backdrop. The first that
frees the target is recorded on the step (`uncovered`, "pressed Escape to
close …") and the action goes ahead in the same step, with no model call. A
cookie or consent banner is left to banner refusal, and a layer that stayed
is not tried again in the run.

Escape (and Enter) are now sent without a native key code: Escape's Windows
code 27 sent as macOS's native code opened Chrome's "About Chrome" page from
a shown tab, which the fallback fixtures found by counting Chrome's tabs.

## Looking before deciding

- **Access blocks (`block.rs`).** After the first observation the browser
  describes the page (`JevBrowser::describe`, a default method: Jev's `Page`
  reads the navigation's `responseStatus` and the visible headings with
  `describe.js`). A 401, 403 or 429 with a refusal phrase ("Access Denied",
  "unusual traffic", "verify you are human", "captcha", …) or nothing to act
  on, a `cdn-cgi/challenge` address, a `/sorry/` address with a refusal
  status, a marker or nothing to act on (an ordinary `/sorry/out-of-stock`
  page is not one), or a page with
  nothing to act on whose title carries such a phrase ends the call
  `access_denied` before any decision. A page that loaded and merely mentions
  a captcha is not one. A `blocked` run whose final page is one is renamed
  too. Detection and honest reporting only.
- **An empty first look is read again.** A page with nothing to act on and
  under 20 characters of text (a document that committed before it rendered)
  is read up to three more times over about 2.6 s before any decision, and a
  BLOCKED about a page that shows nothing is checked once the same way.
  Neither costs a decision.
- **Frames Jev cannot reach (`page/frames.rs`).** Up to two frames of
  another origin, at least 200 by 100 pixels, shown and on screen, are read
  on every observation: one in the page's process through an isolated
  world, one in another process (another site, or a sandboxed frame) by
  attaching to its target for one evaluate. Their text, 700 characters each,
  is added to the page text as `[frame <origin>] …` lines within its 6,000
  characters (the page's own text is cut first), so the decision model can
  see a booking panel, and to the result. Their controls are not offered,
  so a widget's Reserve button stays out of reach. The step fingerprint
  counts frames by origin, never by text (`page/fingerprint.rs`): a rotating
  ad cannot make a no-op step look like progress, while a widget that opens
  still does. Frame text, frame origins and every control's `section` are
  scrubbed of typed secrets like the page text.

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
- DOM-backed actions carry observed `form` ownership (`id`, field labels and
  submit-button labels), or `null` when unassociated. Native `form` attributes
  take precedence over ancestry. Form metadata copies labels, not field values.
  The observation also includes up to 20 `disabled_controls` as label/role pairs;
  those controls remain unavailable for execution. Form reassociation invalidates
  a previously selected action, and the new metadata participates in freshness.
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
  still dropped. Controls left out by the caps, out of reach or cut off are
  counted in `omitted_actions`, one per control (a select's options and a
  field's "Open" click are not counted again), and the result reports the
  count as `omitted.controls`.
- A control cut off by an ancestor nothing can scroll is dropped: a collapsed
  panel (`height: 0; overflow: hidden`) or `overflow: clip`. A hidden-overflow
  box with room, such as a carousel, is kept, because `act.js` scrolls it.
- Actions are sorted by distance from the viewport centre, capped at 100
  offscreen and 250 in all, and pagers and load-more controls always survive
  the caps (fastbrowse's rules). The caps count controls, so a select's
  options count once. Upstream cuts at 250 actions in document order. A choice
  takes at most 255 targets per operation, so the options of a select past
  that (a 300-option list, or a second long select) are not offered; they are
  counted and reported as `omitted.options`.
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
opener. The page records its tabs in a ledger its owner reads after the
run, so the session keeps them for the next call; closing the page closes
every tab it owns. Upstream read such a click as a no-op and left the tab
open.

**Effects (`effects.rs`).** Each step records `effect`, what it visibly did
("went to …", "Size value: Small -> Large", "showed 2 controls: …",
"nothing visible changed"), next to `page_changed`; controls are paired by
node only within one document. It is on `JevActionRecord` and in the result
text, and not sent to the decision model yet (see the docs).

**Operator limits (`scope.rs`, `runner/ceilings.rs`, `usage.rs`).**
`JEV_ALLOWED_ORIGINS` ends a run `blocked` on any page outside it and is
named in the approval request; unset, any origin goes. A call names no
origins of its own (the per-call `allowed_origins` argument is gone).
`JEV_MAX_ACTIONS` replaces the 60-action budget (model calls stay twice
that), and `JEV_MAX_SECONDS` caps `timeout_seconds`. The result carries
summed decision and text-helper `usage`, `"unknown"` where a provider did not
report a count; a billed call whose answer could not be used counts too.
Upstream has none of these.

**Unusable decision replies (`agent/unusable.rs`).** A reply the decision
service gave that cannot be used is counted, and the same decision is asked
again on the same page, at most twice; a usable reply starts the count again.
That covers a reply that fails validation (an action the page never offered,
probabilities that do not add up, a missing answer, a refusal), which is
billed with the usage the service reported, and a body that cannot be decoded
("Invalid TypeSafe response"), after the HTTP layer's one resend. What an
undecodable body was billed is not known, so it counts as a model call with
no usage, and the run's token sums read `unknown` once one has happened,
never a 0 that would read as free. The third unusable reply in a row ends the
run `error` with `stop_cause` `decision_unusable`, and `stopped_because`
gives the count and the first reason. This is the one `error` that falls back
(trigger kind `decision_unusable`), to a model with the full browser tools in
the same tab. A call that failed instead of answering (no connection, a
timeout, a refused key, billing or access, a rate limit) is no reply: it is
not asked again, and it does not fall back.

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
with the gate on, `needs_confirmation`; `access_denied` for a site that
refused automated access.
`blocked` also covers a start page that did not load. The tool result adds a
fixed `next_step` sentence for every status but `done`. Statuses serialize in
snake case.

**Handoffs.** `needs_input`, `needs_confirmation` and `access_denied` hand a
decision to the caller, so they are not failed tool calls: the tool result's
`is_error` is false and its data has `outcome_class: "handoff"`, which keeps a
caller working through them from tripping Roder's five-errors-in-a-row stop.
The same handoff again (same status, final url and `stopped_because` as the
session's previous call) is the caller not acting on it, and is an error with
`outcome_class: "repeated_handoff"`. The text is the same either way. Every
other status is as before: only `done` and `closed` are not errors, and none
of the others has an `outcome_class`. See `src/handoff.rs`.

**Setup.** `timeout_seconds` covers the whole task (starting Chrome,
connecting, loading, the first observation), cut to the host's remaining
deadline, and a phase that runs out ends `timed_out` naming it; upstream timed
only the loop. `Page.navigate`'s `errorText` is read: transient `net::ERR_…`
errors are retried twice (not `ERR_NAME_NOT_RESOLVED` or `ERR_ABORTED`; in a
session tab that already shows a page an `ERR_ABORTED` that is not a download
is tried once more), then the task ends `blocked` with "could not load (…)" before any decision. A tab
the call opened is closed on every failure before the loop, including a
deadline during the attach, and a close is repeated until Chrome drops the
tab; a close of a tab Chrome already dropped succeeds. A session tab that
fails to load a new url stays the session's. A url must parse as `http(s)`
with a host. Loading a url waits for the new document to commit (a stamp on
the old one must be gone) before the ready check and the settle, so a tab
that already showed a page is not read as ready on the old document.

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
  live runs; re-check it when the corpus grows. It applies to the same
  control again and, since `icon_by_picture` deleted Bob's budget mail at
  0.99 and then his lunch mail at 0.60, to a twin: a different control with
  the same label, when that label names a commitment (`delete`, `buy`,
  `send`...; the irreversible gate's word list). The click not made is in the
  result as `suppressed_click` (kind, label, row, the previous click's row,
  confidence) and as a `Not clicked:` line in the text, so a goal that needs
  the twin too can make it. The 0.7 threshold is untested on twin rows.

**Runs that go round in circles (`agent/loops.rs`).** Upstream's stall rule
needs three executed actions in a row that changed nothing. It misses a menu
that opens and closes (every step changes the page), no-ops with a wait
between them, and a page that never holds still (each decision goes stale and
nothing is recorded), which used to run to the 60-action or 120-call budget or
the timeout; a timeout never falls back. Three caps end such a run `blocked`,
and the fallback takes it over as it does after a stall:
- The fourth time Jev would choose the same control on a page with the same
  fingerprint (waits excepted), it stops before acting: `stop_cause` `looped`,
  and `stopped_because` names the control and how often it was chosen. Three in
  a row that change nothing are the stall rule's, which fires first. The
  fingerprint is compared as the page reports it, digits included: a counter
  that rises with every click is progress (the `step_budget` task, which a
  digit-masking compare ended early on the corpus), so a page with a clock on
  it never repeats a pair this way.
- Three stale decisions in a row, each followed by a look at the page that
  shows nothing new, stop the run: `stop_cause` `unsettled`, with the last
  stale message in `stopped_because`. "Nothing new" is the same view compared
  exactly, nothing masked: address, scroll, text, frames, and each visible
  control's id, kind, state, label and value, as the observation reports them.
  A stale decision that leaves a different view, and any step recorded in
  between, start the count again. A page whose only change is digits (a clock,
  a countdown, "3 minutes ago") is therefore progress to this cap, as it is to
  the pair counter: such a run is left to the action and model-call budgets
  and the timeout, and a timeout does not fall back. What the cap ends is a
  page that reads the same at every look while the click guard keeps changing
  (it compares the text of the form, dialog, card or row around the button,
  below the fold too, and a link's `href`).
- Six waits in a row after which the page was the same stop the run as `looped`.
  Such a wait is also skipped by the stall rule (it neither counts as a no-op
  nor breaks a run of them), so `click, wait, click, wait, click` with nothing
  changing is a stall; a wait that changed the page is progress and breaks it.

A false stop costs one fallback (a median of 15 to 16 s on MiniWoB++), not a
failed task. The limits (4, 3, 6) are untested against a live trace: no
recorded run shows an A-B-A-B loop, so they rest on the fixtures
`toggle-menu.html` and `ticker-form.html` (`clock-form.html` pins the other
side: a clock whose digits are on the page does not end a run). The keyless
corpus test fails if any task ends on one of these two causes.

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
combobox's own input) is accepted. A target still covered after Jev tried
to uncover it (see [Uncovering a target](#uncovering-a-target)) returns the
public `Covered` error; the loop records it as a step with `page_changed: false`,
`text: null` and `covered: true` (serialized only when true), so three in a
row end the run `blocked`. Upstream checks only the centre, and treats a
covered target as a stale page. act.js also names what took the click (its
label, text or tag; a field's value is never read), `Covered::with_cover` keeps
that apart from the fixed message, and the loop records it, scrubbed of typed
secrets, on one line and cut to 100 characters, as `covered_by` on the step. The
digest and the fallback's brief show it as page text; the decision request does
not carry it. A scroll box is scrolled by script, by four
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

**Text helper transport.** Upstream only speaks chat-completions. With a
ChatGPT/Codex sign-in, Roder's default writer is GPT-6 Sol at low reasoning
effort over the Responses API: `roder-ext-openai-responses` maps the same
system prompt and field context into the request, `text.format` holds the
reply to a strict JSON schema for `{"text": string | null}`, the token comes
from `roder-codex-auth`, and a 401 is sent again once only if the stored
token changed meanwhile. A default choice whose sign-in cannot produce a
usable token falls back, for the rest of the run, to the turn's model or the
provider list, and the result's `text_model` names that model with a
`note`; an explicit choice fails instead. The reply then passes the same checks as upstream's.
`JEV_TEXT_MODEL` and `JEV_TEXT_MODEL_REASONING` (`none`, `low`, `medium`,
`high`) pick the model and effort on either path; upstream's
`TEXT_MODEL_REASONING=none` shape is the `none` of the chat path, which the
recorded `field_text_requests.json` still pins.

**Tab.** A foreground task's tab is shown when it is created, before the page
loads, and again whenever a later call goes back to it; upstream's port showed
it only after the first observation. A background task's tab is never brought
forward, and is no longer closed when the call ends. Measured on a windowed
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
reply, after that one resend, is an unusable reply ("Invalid TypeSafe
response; no action executed.") that the loop asks about again, and a
rejection (any 4xx but 401) carries its trimmed body. A reply sent as
server-sent events is read to its end and reduced to its final `response`.

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
`effect`. New `Covered` error (`with_cover`, `cover`); `JevActionRecord.covered`,
`covered_by`,
`target_confidence`, `refused` and `dialogs`; `JevDecision.target_confidence`,
`model` and `call_confidence()`; `JevDecisionRecord.target_confidence` and
`model`. `JevBrowser::act` returns the new `JevActOutcome`, and an
observation may carry `dialogs` (the new `JevDialog`).
`JevBrowser::activate` and `JevEngine::activate` are gone: the built-in
runner shows its tab when it creates it, and a hosted browser shows its own.
`JevStatus` is `#[non_exhaustive]`, serializes in snake case and gains
`BudgetExceeded`, `TimedOut`, `NeedsInput`, `Unavailable`, `Error` and
`NeedsConfirmation`; the new `JevStop` error lets a hosted client, transport
or resolver end a run with one of them. `JevStopCause::DecisionUnusable` is
the cause of a run that ended on unusable decision replies (the one `error` that
falls back), `JevStopCause::Looped`
and `JevStopCause::Unsettled` are the causes of a `blocked` run that went round in
circles or never saw the page settle, and
`JevBilled::unusable(usage, error)` marks a reply a hosted client found
invalid, or could not decode (an empty `usage` reads as unknown), so the loop
asks again. For the gate and banner refusal:
`JevEngineConfig::with_irreversible_gate`, `with_irreversible_authorized`
and `with_cookie_banner_refusal(bool)` (on unless given `false`); `JevDecisionClient::choose_gated` and
`JevBrowser::refuse_cookie_banner`, both provided methods (a client that
keeps the default asks nothing, so the gate stops at every shortlisted
action it chooses; a browser that keeps it refuses nothing);
`JevDecision.irreversible` and `JevDecisionRecord.irreversible`; and the
tool's `authorize_irreversible` argument. For the fallback:
`JevRunResult.stop_cause` and the `JevStopCause` enum,
`JevActOutcome.uncovered` and `JevActionRecord.uncovered`, and
`JevExtension` is a struct (`JevExtension::new()`,
`with_inference_engines`) and `JevToolContributor::new(engines)`, which also
registers the `jev_tab_*` tools. These are breaking changes, with no
compatibility shims.

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
POST; Chrome is a throwaway headless instance on a temporary profile, shared
by the whole test process, where each task has a browser context of its own.
The keyless tier runs tasks one at a time, while the live tier may run several.

`tests/fixtures/evals/sessions.json` holds session tasks: several calls on one
thread through the same `JevSessions::call` the tool makes, each graded on its
page and on its tab (`tab_note`, `same_tab`, `tabs_open`), plus the form posts
over all of them. `keyless_session_corpus_passes` plays each call's plan;
`live_session_corpus` (ignored) asks the hosted model.

| Session task | Covers |
|---|---|
| `catalog_session` | page through a catalog, then go on from that page in the next call |
| `report_session` | a link opens a tab; the next call goes on in that tab |
| `search_then_submit_session` | pick a suggestion in one call, submit the search in the next |
| `navigate_in_session_tab` | a new url loads in the session's tab instead of a new one |
| `reserve_search_session` | search tables on a city page, then pick a slot in the next call; the booking widget opens in a frame of another site, read but not acted in |
| `reserve_signin_wall` | a slot leads to a same-origin sign-in wall; the call stops without filling or submitting it |
| `reserve_change_party` | go on from a results page and change the party size through its summary chip |

Session calls may also expect `frame_text_contains`, `digest_contains` (the
text the caller reads) and `forbid_actions`; `{today}` in an expected text is
today's date as a booking page shows it ("Mon, Sep 28, 2026").

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
| `stall_detection` | three clicks that change nothing end as `blocked` (the stall rule fires before the loop caps) |
| `step_budget` | a run that keeps progressing stops at 60 actions; a counter that rises with every click is not a loop |
| `missing_value` | a value the goal lacks stops the run before any POST |
| `confirm_dialog` | a delete's confirm is dismissed; the run ends `blocked`, saying why |
| `menu_button` | a menu that arrives 500 ms after its button is clicked, then an item |
| `styled_checkbox` | a filter whose hidden checkbox only its label can toggle |
| `date_field` | a native date input set to its ISO value, then submitted |
| `focus_handoff` | typed text follows focus to an editor opened over the field |
| `icon_by_picture` | a div-and-span mail list: an unnamed trash icon in the right row |
| `icon_by_picture_twin` | the same list: an unsure second delete on Bob's other mail, right after the first, is not clicked |
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
| `reserve_wrong_neighbourhood` | results mixing neighbourhoods, one slot label on every card: the slot is taken in a Mission card, and nothing is reserved |
| `access_denied_page` | a page served 403 "Access Denied" ends `access_denied` with no decision |
| `empty_first_look` | a page that commits blank and renders 700 ms later is read again, not judged blocked |
| `popover_escape`, `popover_button`, `popover_outside` | a date picker Jev opened stays open over the results; Jev closes it (Escape, its Close button, a press outside) and takes the slot in the same step |
| `popover_icon_fallback` | a picker only an unlabelled icon closes: Jev stops blocked; the fallback closes it and takes the slot |
| `canvas_fallback` | a button drawn on a canvas, pressed by the fallback at its coordinates |
| `drag_fallback` | a card that moves only by dragging |
| `hover_menu_fallback` | a menu that opens only while the pointer rests on it |
| `keyboard_only_fallback` | a page with nothing to act on, driven by a key |

`icon_by_picture` to `escape_popup` cover generic clickables, field naming,
scroll boxes and keys. With `jev-1.13.0` only `unlabelled_fields` of them
passes live: the model never presses Enter or Escape,
does not pick the suggestion, stops two scrolls short of the end of the
terms, and deletes Bob's second mail after the first. They stay in the
corpus as the record of those gaps. The last seventeen have not run live
yet: the decision service returned HTTP 402 to every request while they
were added. A task with a `fallback` block is one Jev alone cannot finish: its
`expect` is Jev's own end (blocked), and `fallback.expect` the call's after
the fallback; the keyless tier plays `fallback.plan` with a scripted model
that names its targets by label (`ref_of`, `at`). The `gate_` tasks turn the gate on themselves
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
  model alone; `JEV_EVAL_TEXT=model` uses the configured text helper instead
  (`JEV_TEXT_MODEL` and `JEV_TEXT_MODEL_REASONING` pick it), except for a
  password or one-time code the task holds, which still comes from its values
  as a supervisor's resolver would supply it. Each row's telemetry records the
  text model and each text call's latency, usage and outcome (never the
  value); MiniWoB++ rows carry the same under `text`.
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
- `JEV_EVAL_FALLBACK=model` runs the automatic fallback after every task
  Jev could not finish, on `JEV_FALLBACK_MODEL` (default `gpt-6-sol`) at
  `JEV_FALLBACK_REASONING` (default low) through the Codex sign-in (or
  `OPENAI_API_KEY`), with `JEV_EVAL_FALLBACK_STEPS` tool calls (default
  20). Each row is graded twice, Jev alone (the row's `verdict_ok`,
  `truth_ok` and `false_green`) and the call after the fallback (the same
  three under `fallback`, with its steps, model calls, tokens and time), and
  the table prints both totals. MiniWoB++ honours it too: `success` is the
  episode's reward at its end, `jev_success` when Jev stopped, and the
  summary gives both scores and what the fallback cost.
- `JEV_EVAL_TASKS` narrows the run and `JEV_EVAL_CONCURRENCY` (default 2)
  sets how many runs go at once.
- `JEV_EVAL_N=3` runs each task that many times (default 1, at most 20; all
  tasks once before any twice) and prints a tally per task. The paired
  Decisions/Jev eval keeps its own `JEV_EVAL_REPEATS`.
- `JEV_EVAL_STRICT=1` fails the test on a mid-run edit of the tree, on any
  false green, and then on any failed run; with a baseline it fails on any
  task whose pass, `verdict_ok`, `truth_ok` or false-green rate got worse
  than the baseline's instead, so tasks known to fail do not hold a run up.
- The baseline is `tests/fixtures/evals/live-baseline.json`, or the file
  `JEV_EVAL_BASELINE` names. `JEV_EVAL_SAVE_BASELINE=1` writes this run's
  tally there with the commit, a hash of the crate's sources and fixtures,
  a hash of the corpus, the model and the run's switches. It refuses a run
  with fewer than 3 runs per task, a run narrowed by `JEV_EVAL_TASKS`, and a
  run whose tree changed under it (the pin is read at the start and again
  at the end).

Both tiers print a result table (`pass`, `FAIL` or `FALSE-GREEN` per run) and
write one JSONL line per run of a task to `target/jev-evals/` (`keyless.jsonl`,
or `live-<unix seconds>.jsonl`). A row has:

- `verdict_ok`: the agent's claim is the one the task asked for (the final
  status, and whether it gave a reason for stopping);
- `truth_ok`: the page, the recorded POSTs, the DOM probes and the executed
  trace match the task;
- `false_green`: `verdict_ok` and not `truth_ok`, a claim the page does not
  back. A row passes when it is `verdict_ok` and `truth_ok`; there is no
  `pass` field. The keyless corpus asserts `false_green` is 0;
- `repeat` (the run's number, from 1), `failures`, the status, steps, model
  calls, wall time and `input_tokens` when every call reported them;
- `watch`: `looks`, one per reading of the page, with the `offered` count,
  `omitted_actions` (what the snapshot left out) and the `settle` before it
  (`reason` and `waited_ms`, as the page reported them); `laps`, calls and
  milliseconds in `settle`, `snapshot`, `fresh`, `act`, `banner`,
  `describe`, `screenshot`, `decide` and `text`; and `run_ms` against
  `attributed_ms`, the laps summed. Nothing in it reaches the decision
  request, and it holds counts and timings, never page text.

Live rows add per-step confidence, the answering model, request size, input
tokens and the shadow answers. A live run also writes
`live-<unix seconds>.summary.json` beside its rows: the pin at the start and
at the end, anything that changed between them, the tally and the comparison
with the baseline. To add a task, add a page if none fits and an entry to
`tasks.json`; a unit test checks every entry parses and names a real page.
`search-decoy.html` is not in the corpus: it is the page the false-green test
runs on, a search that looks finished and posts nothing, which `tasks.json`
must not hold since its scripted plan ends `done` over a failed grader.

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
  each worker has its own browser context on the one shared Chrome.
- `JEV_MINIWOB_DUMP=1` prints every observation's actions and text.

Rows stream to `target/jev-evals/miniwob-<unix seconds>.jsonl`, one per
episode (task, seed, supported, success, raw reward, reward reason, Jev
status, steps, model calls, text calls, wall time, stop reason, the executed
trace and the first observation's actions). The printed summary gives the
overall score, the score over supported tasks, per-task success rates and the
failure causes.


## Measured Jev browser guidance

The default TypeSafe client supplies concise action history with row/section
context, observable form ownership and disabled controls. Its questions distinguish
current form steps from later fields and require completing the requested workflow
after validation or availability changes. Form associations participate in stale
action checks. Jev still receives text; screenshot input belongs to the optional
OpenAI Decisions backend.

See the [Jev hill-climb report](../../evals/reports/jev-hillclimb/2026-10-08/README.md)
for controlled prompt variants, repeated paired runs, complex development tasks,
a sealed holdout and failure traces. The new fixtures grade exact saved state,
mutation counts and forbidden side effects, including duplicate actions and
stopping at a review screen. These evals supply field values from fixtures and
disable fallback; they measure browser decisions, not text generation.
