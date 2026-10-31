## 0.6.0 (2026-10-31)

### Breaking Changes

#### Jev saves downloads, waits out pages that clear themselves, and reads the HTTP status from the browser

Breaking: `JevRunResult` gains `downloads` and `JevActionRecord` gains `downloads` (both `Vec<JevDownload>`: the file's `name`, `url`, `path` where the browser saved it, `size_bytes`, and `incomplete` / `repeated` flags). A link to a file or a button that saves one used to read as a click that changed nothing, so the model clicked the export again until the stall rule stopped the run. Jev's browser now saves the downloads of its tabs' browser context under `<temp>/jev-downloads` (each finished file in `kept/<guid>/<name>`), waits after an action for the files it started, and the next observation reports each one, with the start of a readable file's text, before the page text. A new file counts as the step's change; the same file saved again (same name and content) does not, and the stall rule still ends a run that keeps saving it. The tool's text lists the files with where they were saved.

An opening page that may clear itself (a challenge such as "Just a moment...", or a 503, with nothing to act on) is now watched for up to 12 s, read again each time it changes, before it is classified, so a bot check that moves on or a busy site that retries no longer ends the call `access_denied` or costs decisions on an empty page. A plain 403 "Access Denied" or a 429 is not waited on. A 503 with a challenge phrase and nothing to act on is now an access block. A start page that still shows nothing after the empty-page reads, served without an error status, is loaded once more. `JevBrowser` gains two default methods for this, `reload` and `wait_for_change`.

The page's HTTP status (`JevPageFacts.http_status`) is now the one the browser's Network events gave the committed document, so a file the page linked to, or a page script that shadows `performance`, no longer changes it.

#### Jev's irreversible-action gate reads the page's evidence, stops password changes and forbidden downloads

Breaking: `JevDecision` and `JevDecisionRecord` gain `downloads_forbidden: Option<f64>`, so a struct literal of either must set it (`None` when the gate did not ask).

With the opt-in gate on (`JEV_CONFIRM_IRREVERSIBLE=1`), the shortlist of actions it asks about now reads more than the label: the last segment of a link's address (a one-click `/unsubscribe` link labelled "Stop these emails"), the heading or row a control sits under (a dialog's "Yes, I'm sure" under "Delete the Atlas project?"), agreement labels, and buttons that post their form, with the label-and-link evidence asked about first. The commitment words gain `deactivate`, `withdraw`, `terminate`, `renew`, `upgrade`, `downgrade`, `sell`, `bid`, `approve` and the same words in German, French, Spanish, Portuguese, Italian and Dutch (`bestellen`, `commander`, `comprar`...). Each question now quotes the element's label, where it sits, where a link goes and the form a submit sends, and observed actions carry `href`, `download` and `submits` for it. Play, pause and mute controls are never asked about unless they name a committed noun.

Two new stops need no answer about the action: a submit or Enter in a form that sets a new password ends the run `needs_confirmation` unless the call is authorized and confident, and a click the page says saves a file (a `download` attribute, or a link to a `.zip`, `.exe`, `.csv`...) ends it when one more question, asked of the goal alone, says the task rules out downloading, whatever the authorization.

Past the 8-question cap, only an Enter or a click whose label names a commitment fails closed; a click shortlisted only by its address, heading, row or posting form that the cap left unasked is dispatched, so a page of many "Add to cart" forms cannot stop a run. A sign-in wall now compares form ids as the snapshot gives them (numbers), so a missing value in a sign-up form is `needs_input` rather than `needs_login`.

#### Jev ends `needs_login` at sign-in walls and scrubs secrets in every encoding

Breaking: a run that reaches a sign-in it cannot make now ends with the new status `needs_login` (`JevStatus::NeedsLogin`) instead of `needs_input` or `blocked`, and `JevRunResult` gains `wall` (`JevWall` with `kind` `password`, `one_time_code` or `sign_in` as `JevWallKind`, the page's `origin`, and the labels of the `fields` it asked for). It applies when the goal gives no value for a password or one-time-code field (or a field in the same form as one), when a `blocked` run ends on a page that shows a sign-in or single sign-on wall, and to an HTTP 401 with nothing to act on, which was `access_denied`. `needs_login` is a handoff (not a failed tool call), never falls back, and its next step asks the user to sign in in the tab or give the code. A field for a new password, or any field of a form that holds one (a sign-up), is still `needs_input`.

A typed password or code is now scrubbed from page reads, addresses, evidence and the result as typed, percent-encoded (a GET form's `password=p%40ss+word%26`, `encodeURIComponent`), JSON-escaped and with its whitespace collapsed. Scrubbing twice changes nothing, and the host of an address on the origin the secret was typed on keeps its name.

### Features

#### Reach controls under fixed bands, name what a cover offers, follow late tabs

A target under a band fixed to the window (an anchored ad, a sticky footer, a chat bubble) is scrolled clear of it, or, where the page cannot scroll, pressed, typed into or chosen from the keyboard, with the control's guard and what covers it checked again right before the key; nothing in the band is pressed, and a dialog, menu, popover or full-window cover is never pressed through. Enter in a field a dialog or backdrop covers is now `Covered`. A covered step also records `covered_controls`, up to eight controls the cover offers to press (`Covered::with_controls`, `Covered::controls`, `COVER_CONTROLS`), shown in the digest and the fallback's brief and never sent to the decision service. A click act.js reads as opening a new window (a link or submit naming `_blank`, a control that says it opens a new tab or window, an inline `window.open` handler) waits up to 5 s for its tab, so a tab a page opens after a fetch or a timer is followed on the click that opened it.

#### Ask the text model before an unsure repeat; opt-in consult and light recovery

When Jev is unsure and about to repeat the click it just made, which the repeat guard would stop, the text model is now asked first (`JevConsultant`, `JevEngineConfig::with_consultant`, with Roder's text helper as the consultant). If it names the same control, the click goes ahead. If it names another control, that control is acted on instead, still gated. If it names nothing, the guard stops as before. This lets a feed's second "Load more" through: `mock_infinite_scroll` used to end `blocked` with 40 of 60 items. A repeated commitment, a repeat that adds something (such as "Add to cart") or posts a form, and a twin still stop.

Two fastbrowse tiers ship off by default, because they did not measurably help on the live corpus or MiniWoB++:

- `JEV_CONSULT_BELOW=0.55` (`with_consult_below`) consults before any step that acts under that call confidence. It may act on another observed control, which is still gated, with no answers and no confidence.
- `JEV_LIGHT_RECOVERIES=2` (`with_light_recoveries`) gives a stalled, covered or looping run up to that many one-step recoveries before it ends `blocked`. A recovery spends the stall's evidence.

`JEV_CONSULT=0` turns the consultant off entirely. The consultant is never offered a cookie-banner choice. Its calls are counted as text calls with their usage, and the run's `recoveries` record what each one did.

#### Fill several fields of one form from one text call

When Jev first types into an empty field of a `<form>` that has other empty
text fields, the text helper is asked once for all of them (after
fastbrowse's form writer). The chosen field's value is typed now, and the
steps that later choose the other fields type the values it wrote without
another text call, as long as each field is still the same node, reads the
same and is still empty, and no button or link click, Enter or key has run
since (any of which may submit the form or show what it rejected). Each field is still chosen by its own decision. A
form with a password or one-time-code field is never batched, so no secret
reaches the text model this way; a field the reply leaves out, one that
changed since, and a form whose call failed fall back to one call per field.

`JevTextValueResolver` gains `resolve_form`, returning `JevFormValues`; its
default answers `None`, so a supervisor's resolver keeps answering one field
at a time.

#### Route Jev decisions through OpenRouter

`JEV_DECISION_PROVIDER=openrouter` sends Jev's System One requests to `https://openrouter.ai/api/v1/systemone` with `OPENROUTER_API_KEY` (or the configured `openrouter` provider key), text only. `JEV_DECISION_URL` overrides the URL of either hosted Jev route. Run usage now carries the dollar `cost` a host reports when every call reported one, and live eval rows, tallies and summaries record decision cost and the model that answered.

#### Jev's settle waits for the page's own requests and ignores loader class names

After an input, a page that is quiet and unchanged while a fetch or XHR request that input started is still open is no longer read before the response lands: the settle waits for the response and then for quiet again, within the 2.5 s cap, and reports `reason: "response"`. Each tab now enables the Network domain for this. Class and id names such as `spinner`, `loader` or `skeleton` no longer count as loading indicators; only `aria-busy`, `role="progressbar"` and an unvalued `progress` hold the settle, so pages that merely name their components after loaders no longer pay a 1.5 s wait on every step.

### Fixes

#### Short date line in decision requests

Jev decision requests carry today's date and the next two months only, as fastbrowse's do; the list of the next seven days goes to the field writer alone. With the list, the decision model answered BLOCKED on a one-control page in 6 of 400 requests, against 0 of 450 without.

#### Answer one-option choices locally and recover from shed Jev requests

A hosted Jev decision no longer sends a target choice with a single option: it is answered locally (that option, probability 1). A batch the service answers with a 5xx twice in a row is asked one question at a time within the retries it had left, and a large step that is still refused is asked again without its off-screen targets, then paged smaller, instead of ending the run `unavailable`. The decision's `usage.context_budget.shed` counts these retries.

#### Read frames of another process on kept sessions, within a 5 s deadline each

Each tab's setup turns on `Target.setAutoAttach` for iframes, and the connection keeps the sessions Chrome attaches to frames of another process (`cdp/frames.rs`), so reading such a frame's text is one evaluate instead of a target listing, an attach and a detach (about 90 ms down to 4 to 8 ms per read on the fixture site; pages without such frames drop `Target.getTargets` from every observation). Each frame's read now has its own 5 s deadline instead of the connection's 30 s, and a frame of another process that missed it gets 0.5 s on later reads for 30 s, so an advertisement whose renderer stopped answering costs one observation 5 s, not every observation 30 s. Frames stay read-only.

#### Install Jev's observation scripts once per document, and keep freshness guards as digests

Each document now installs `snapshot.js` and `context.js` once, under a
read-only global with a random per-process key. The scripts are registered
with `Page.addScriptToEvaluateOnNewDocument`, and an older document installs
them lazily on its first read. Each observation and freshness check then
sends a short call (about 660 bytes) instead of about 70 KB of script.

Guards keep their structured fields but hold a digest of the text around
the control. The page key and freshness marker hold digests of the form
state, title, page text, disabled controls and on-screen actions, so stale
diagnostics still name the field that changed.

#### Multi-step eval tasks, a work gate, and transient provider time kept apart

The eval corpus gains ten multi-step tasks ported from fastbrowse's mock site (a sign-in and basket, an order the shop rejects until its postcode is corrected, a six-item cart, a send that fails with 503 behind a thank-you page, a hidden instruction, a honeypot field, a consent banner with a delayed modal, an infinite feed, a pager to exhaust, and a wizard rejected for a choice two steps back), each graded on what the site holds and checked against runs that stop short and say DONE. The live tally now records median decision calls, text calls and wall time per task, and its baseline comparison flags a task whose median calls, input tokens or decision cost rose past a quarter as `MORE WORK`, which `JEV_EVAL_STRICT=1` fails on; wall time never gates. The hosted decision transport counts the time its requests lose to transient provider failures (retried attempts and backoff waits), and live eval rows report it as `transient_ms`, apart from the wall time.

#### Richer step effects for Jev, and per-snapshot name and visibility caches

A step's `effect` now names an HTTP status of 400 or more that a navigation
landed on, and the fields its form still needs when an action set nothing
(the snapshot marks required-and-empty and `aria-invalid` fields
`blocking`). A click on an option, checkbox, radio or switch that set no value
says so, and the stall rule counts it as a step that changed nothing; an
autocomplete pick that leaves the typed text as it was counts as set.

The observation reads each element's visibility, name and cursor once per
snapshot, and `context.js` each title's and scope's text once per run;
the caches end with the call, so later scripts read the page as it is.

#### Relative dates, form-mode rule and text-helper routing

Today's date given to Jev and the text helper now lists the next seven days with their weekdays, and the text helper is told to look a relative day ("next Monday") up there rather than count it, and to write a date as a date, never as a weekday. The next-action rules choose a form's mode (tab, trip or account type) before typing and set requested filters before submitting. Text-helper requests to OpenRouter carry `provider: {sort: "latency", require_parameters: true}` (dropped for the run after a 404 says no endpoint of the model takes every parameter), and a reply cut off at the output cap (`finish_reason: length`) is asked once more with four times the cap. A code fence around or after the text helper's JSON reply is ignored rather than failing the run.

## 0.5.2 (2026-10-30)

### Fixes

#### Read browser observations as shared semantic trees and changes

Group visible controls and table cells under page landmarks, forms and lists, show focus and state, and
return changes after actions instead of repeating every control. Explicit looks
recover the full viewport snapshot. Keep geometry in structured evidence and retain page
text, secret scrubbing, stable refs and conservative truncation markers.

Bound hosted Jev requests against both the request and per-question context
limits after prompt and image expansion. Offer additional text and target pages
on demand, preserve safety questions and original action identities, and retain
usage across context reads and bounded repacking.

Use the same semantic row contract, change detection and renderer in native Jev,
Decisions and direct browser tools. Parent handoffs expose refs accepted by
`jev_tab_*` and seed the next action diff without an extra look.

Read direct browser text from the visible viewport, including passive paragraphs
and scroll-panel clipping. Keep bounded, source-linked observations across a
single Jev delegation and return them to the parent, so a multi-page task need
not be repeated through manual tools. Page retained evidence through the same
request-budget controls as current text.

Do not turn an uncertain repeated click into a successful completion claim.
Suppress the duplicate action, report `uncertain_repeat`, and let configured
fallback continue unmet requirements in the same delegation with retained evidence.

Preserve bounded historical page text during fallback compaction and return the
fallback's prose findings before action traces, rather than discarding research
facts and shortening the final answer to a status line.

Add bounded document-text reads to `jev_tab_look` for research without repeated
scrolling. Keep secret scrubbing before pagination and the viewport baseline
unchanged. When automatic recovery is available, use a 12-action native budget
unless the operator sets `JEV_MAX_ACTIONS` explicitly.

Offer paginated rendered document reads inside the native decision loop. Replan
once after an uncertain repeated navigation, retaining the stricter stop for
consequential actions. Revalidate an exact navigation target after a stale
observation without another model call only when its identity, destination,
context and form state remain unchanged. Record bounded field-level freshness
and recovery diagnostics, and accept a bounded requirements checklist for
multi-part tasks. Add a live one-tool fallback/hand-back experiment without
changing the default fallback policy.

In explicit research mode, preload bounded document text on arrival, prioritize
main content while retaining the rest of the rendered document, and advance the
caller checklist one requirement at a time. Record each completed requirement as
a driver claim with its available evidence sources. Research completion checks
document identity; interaction completion retains full observation freshness,
and explicit success predicates still apply.

## 0.5.1 (2026-10-29)

### Fixes

#### Keep Jev screenshots from resizing visible Chrome windows

Jev now uses a windowed browser's natural viewport and display scale instead of
forcing a fixed non-Retina viewport. Its screenshots share the direct CDP image
normalization, keeping CSS coordinates accurate without resizing Chrome during
capture. Headless Jev sessions retain their fixed viewport.

## 0.5.0 (2026-10-27)

### Breaking Changes

#### Release a coherent runtime API compatibility boundary

Adopt explicit shared-read/exclusive/coordinator scheduling, typed custom-provider
capabilities, and owned worktree metadata. Rust integrators must update
ToolExecutor implementations to execution_mode and initialize the new public
configuration and ownership fields.

Release the shared API's reverse dependency closure together so older consumers
cannot resolve patch-compatible packages that expose incompatible API types.
The CLI package also exports a Rust library, so its compatibility release is
0.4.0. Existing command-line use remains supported.

## 0.4.6 (2026-10-26)

### Fixes

#### Add GPT-6.1 Terra and retire GPT-6 Sol

Expose `gpt-6.1-terra` with a 1M context window and standard prices of $1 / $0.05 / $5 per 1M tokens. The default GPT model is now `gpt-6.1-sol`. `gpt-6-sol` stays resolvable but is hidden from selectors.

## 0.4.5 (2026-10-17)

### Features

#### Add semantic DOM evidence and completion audits to visual Decisions

Select `JEV_DECISIONS_PROFILE=dom` for semantic page structure and form/workflow
questions, or `activation` for clipped-field image routing and typed completion
audits. Embedded clients can use `DecisionsBrowserProfile`. Text-only operation
preserves the selected profile. Independent target and operation batches retain
their original answers and aggregate actual usage; existing action gates apply.

Secret fields report observed presence without misleading empty value placeholders.
Component requests and audits reserve the remaining browser decision budget and
count toward reported calls on answered and billed-error paths.

Custom decision clients can reserve each component through the public
`JevDecisionTransport::decide_budgeted()` boundary. Observed component counts
remain available when a client omits `usage.calls`.

### Fixes

#### Audit pending activations before experimental Jev completion

Add opt-in `JEV_PROMPT_PROFILE=activation`, extending `dom_targets` with a
text-only completion check. Separate typed judgments assess whether the offered
conditional click is required, already performed and ready. Conservative support
can recover a pending click while retaining its original target probabilities
and irreversible-action gate. Corrected operation confidence records derived
support, and usage includes the additional request. Default behavior is unchanged.

#### Opt into TypeSafe image input for Jev decisions

`JEV_IMAGES=1` sends the current viewport screenshot with each hosted Jev decision, fitted to TypeSafe's limits (PNG or JPEG, 280 to 1568 px per side) and referenced from the request state, with the `TypeSafe-Danger-Zone-Images: true` header. Screenshots are suppressed around recognized secret fields, as for Decisions. Off by default; the text-only request is unchanged.

#### Expose semantic DOM evidence in Jev browser results

Browser results include a bounded semantic DOM tree with page hierarchy, roles,
control states and observed node references. Secret fields and typed secrets stay
redacted. `JEV_PROMPT_PROFILE=dom` opts into the experimental DOM-grounded Jev
prompt; use `JEV_IMAGES=1` to evaluate it with screenshots. The default prompt
remains Workflow. The paired eval supports image input for every Jev profile.

Candidate rectangles are scaled into the fitted screenshot's pixels when the
viewport image is resized, so visual references stay aligned.

For clipped populated editable fields, `JEV_PROMPT_PROFILE=dom_targets` keeps screenshots
in conditional target questions while selecting operations and evaluating
irreversible actions from DOM evidence. Other pages retain shared visual evidence.

DOM evidence preserves styled toggle states and controls nested in headings, and
redacts JSON-escaped secret echoes. Offscreen DOM updates do not invalidate an
otherwise unchanged page-level decision.

## 0.4.4 (2026-10-11)

### Fixes

#### Name the program embedding Roder in sign-in hints

Programs that embed the Roder runtime ship their own command line, so a hint
such as "run `roder auth login codex`" means nothing to their users.
`roder_api::cli_identity` lets the embedding program set its identity once at
startup with `set_cli_identity(CliIdentity { name, auth_login, auth_dir })`.

- Every sign-in hint (Codex, SuperGrok, Kimi Code, and Jev's text-model notes)
  now names the configured `auth_login` command, with `{provider}` replaced by
  the provider id. Roder's own default is `roder auth login {provider}`.
- The Codex, SuperGrok and Kimi Code token stores keep their files directly in
  the configured `auth_dir`. Without one they keep using `auth/` under Roder's
  data folder: `RODER_DATA_DIR`, else `RODER_CONFIG_DIR`, else `~/.roder`.
- Reading the identity before one is set uses Roder's defaults without fixing
  them, so an embedding program can still set its own afterwards.
- The Codex sign-in's busy-port error names the configured program.

## 0.4.3 (2026-10-10)

### Features

#### Jev names what covered a click

A step whose click was covered now names what covered it: `JevActionRecord.covered_by`, `Covered::with_cover` and `Covered::cover`. The name is page text, scrubbed, on one line and cut to 100 characters, and the digest and the fallback prompt show it as untrusted. The hosted chooser request is unchanged.

The fallback's opening message now labels Jev's stop reason as untrusted page text, collapses it to one line and cuts it to 300 characters, because a `looped` stop quotes the page's own control label; the standing fallback instructions say the reason and the last steps are untrusted too.

#### Jev retries an unusable decision reply and falls back when it stays unusable

Jev asks the decision again, up to twice, when the service's reply cannot be used. That includes a reply body that cannot be decoded, after the HTTP layer's one resend; it is counted with unknown usage. If the replies stay unusable the run ends `error` with the new `JevStopCause::DecisionUnusable`, whose stop reason gives the reply count and the first reason. With the default `JEV_FALLBACK=auto` the frontier model then takes over with the new `decision_unusable` fallback trigger, unless Jev declined a page dialog during the run; `handover` tells the caller to go on with the browser tools instead, and `off` leaves the error as it was. New public `JevBilled::unusable` marks a billed reply the loop may ask again about. Every other `error` (a refused key, billing or rate limit, a server error, an unreachable provider) still does not fall back.

#### jev_browse handoff statuses are outcomes, not tool failures

A first `needs_input`, `needs_confirmation` or `access_denied` result from `jev_browse` is no longer a failed tool call (`is_error` false, `data.outcome_class` "handoff"). An identical repeat on the same page is an error again (`repeated_handoff`). Other statuses and the result text are unchanged.

#### Jev ends runs that go round in circles

Jev ends a run that goes round in circles instead of burning its budget. The fourth identical (page, control) pair, six waits in a row that change nothing, and three stale decisions in a row with nothing new on the page each end the run blocked with the new stop causes `looped` and `unsettled`, and the frontier fallback takes over as it does after a stall. The stale cap compares pages exactly: a page whose only change is digits (a clock, a countdown) counts as progress and is left to the action and model-call budgets, as the same-pair counter already treats it. An unchanged wait no longer hides a stall. New public `JevStopCause::Looped` and `JevStopCause::Unsettled`.

#### jev_browse success_condition matches text more reliably and adds text_absent

`jev_browse` `success_condition` gains `text_absent`, compares `text_contains` and `text_absent` ignoring case, and all three ignoring whitespace runs, no-break spaces and zero-width characters (`url_contains` keeps the case of the path, query and fragment, as before), leaves form-field values out of the page text being matched, and reports the unmet predicates (condition met / not met).

#### Jev stops before clicking the wrong twin row

An unsure delete (or other commit-named click) on a twin control right after a click on the same label that changed the page now ends the run done without clicking, instead of clicking the wrong twin. The run result gains `suppressed_click` (`JevSuppressedClick`, `JevSuppressedKind`) and the result text a "Not clicked:" line. The old same-control rule also reports what it held back.

#### jev_browse says what it did not offer Jev, and reports checkbox state correctly

`jev_browse` results now say what the page held that Jev was not offered: a counts-only "Not offered to Jev" header line and an `omitted` field (`controls` the snapshot left out, counted per control instead of per action, and `options` past the 255 a choice takes, such as a 300-option select). Checkboxes, radios and switches now report `checked` true/false in the result's controls and read [checked]/[unchecked] in the options list, instead of the HTML value "on". `JevControl` gains `checked`, `JevRunResult` gains `omitted`, and `JevOmitted` is new public API.

### Fixes

#### jev_browse hints name the other browser tool families

The blocked next-step hint and the missing-key errors for `jev_browse` now name the other browser tool families (`chrome_*`, `browser_use_*`, `webwright.*`) without claiming they are available.

#### Jev eval rows split verdict from truth

Jev eval rows split the old pass flag into `verdict_ok`, `truth_ok` and `false_green` and carry per-phase laps, `omitted_actions` and settle timing. The live tier gains `JEV_EVAL_N` repeats, a pinned per-task baseline with mid-run-edit detection, and a search-decoy false-DONE fixture. All of it is test-only code under `fixture_harness`; the one production edit (`Page::settle` returning the page's settle reply) does not change behaviour. Docs updated.

#### Jev fallback offers its screenshot tool only where the model can see the result

The Jev fallback offers its screenshot tool only to a model whose engine forwards tool-result images (`tool_result_image_input`), not merely one that takes user images, and a model without the tool is no longer told to use one.

## 0.4.2 (2026-10-09)

### Features

#### Use OpenAI Decisions as an alternative browser decision backend

Set JEV_DECISION_PROVIDER=openai to select browser actions with the Decisions
API using an OpenAI API key. Keep the shared Chrome session, text helper,
action validation, approval policy, and optional irreversible-action gate.

Expose screenshot-based selection among host-supplied native computer action
batches, with a blocked outcome and normal host-owned permissions. Add strict
live browser and computer evals and register the backend's environment settings
in distribution metadata.

Resolve single-target choices locally because the live Decisions API requires
at least two choices. Validate authenticated browser tasks and screenshot-based
native computer execution against the live endpoint.

Add a reproducible paired live Decisions/Jev browser benchmark with alternating
provider order, repeated outcome grading, latency, and provider-reported usage.

Use Decisions-specific browser questions, action-effect context and fresh viewport
images, with a text-only option and screenshot suppression around recognized
secrets. Preserve per-question refusals, stopping only when a required selection
is refused and keeping safety checks closed. Add controlled optimization variants,
wire-level diagnostics and held-out browser fixtures.

Improve Jev's text evidence with native form ownership, disabled controls and
concise contextual history. Add staged-workflow guidance and a reproducible
Jev prompt hill-climb with compound development fixtures, sealed holdout tasks,
side-effect grading, negative browser traces and candidate-order stress probes.

Keep screenshot suppression across calls after short secret input and continue with text evidence if optional capture fails. Scrub secret echoes from form and disabled-control metadata.

## 0.4.1 (2026-10-01)

### Features

#### Preserve browser observations and execute Desktop input through CDP

Keep screenshot text and original resolution in Responses tool replay, including
when older images are removed to fit the request budget. Attach Chrome extension
screenshots to the model input. Use stable document-scoped refs, real CDP input,
validated typing targets, isolated script state, and explicit action failures in the Desktop fallback.
Isolate browser-use servers by thread, serialize actions with fresh page state,
and stop the owned process tree when an in-flight tool is cancelled. Cover
browser observations and permission rejection through the ACP transport.
Give each browser-use server a private profile and file directories, enforce
optional operator navigation ceilings, and bound agent steps. Preserve Desktop
tab identity across enumeration changes. Release held input and screenshot
masks on cancellation or failure, including sessions retained by their owner.

Cancel a pending Chrome bridge command on dispatch timeout or dropped futures,
and label the paired extension action observations as untrusted page data.

Support optional caller-defined Jev completion conditions checked against fresh
UI state. Reject premature model DONE and permit bounded fallback recovery;
report unverified model completion explicitly when no conditions were supplied.

### Fixes

#### Validate native browser registration and recover failed input cleanup

Declare the native computer tool in Chrome's extension manifest, reserve its
name from external function tools, and reconcile cancelled cached sessions.
Retry failed input releases before reusing a session and attempt all key, mouse,
and screenshot-mask cleanup independently. Scrub overlapping secrets and every
nested element observation. Reject malformed Desktop origin and tab settings,
check origin scope before resolving targets, and disable arbitrary eval under
an origin ceiling. Honor the configured eval provider endpoint, validate native
compaction, preserve generic image-reopen guidance, and accept documented empty
Jev completion conditions and sparse serialized controls.

## 0.4.0 (2026-09-29)

### Breaking Changes

#### When Jev cannot progress, Roder's full browser tools go on in the same tab

Jev acts only on the controls its snapshot offers, with a small action
vocabulary. When a run ends because of that (the model answered BLOCKED,
three steps changed nothing, its targets stayed covered, the page offered
nothing Jev can act on, or its budget ran out), Roder now falls back to its
own direct CDP tools on the same tab, with its cookies and page state, as
`JEV_FALLBACK` says:

- `auto` (default): `jev_browse` runs a bounded loop driven by the session's
  model (or `JEV_FALLBACK_MODEL`, at `JEV_FALLBACK_REASONING`, default low)
  with look, screenshot, click at a ref or x/y, hover, drag, type, any key,
  scroll, select, navigate and wait, and returns one result: the call's end
  state, Jev's own status as `jev_status`, and `drivers` with each driver's
  steps, model calls, time and tokens. `JEV_FALLBACK_MAX_STEPS`,
  `JEV_FALLBACK_MAX_SECONDS` and `JEV_FALLBACK_MAX_TOKENS` bound it.
- `handover`: the result tells the caller the full tools work on this same
  tab, names them (`jev_tab_*`) and the tab, and says where Jev stopped.
  `auto` hands over too when no model can drive it or it also fails.
- `off`: Jev's result as before.

It never runs after `needs_input`, `needs_confirmation`, `access_denied` or a
page that did not load, and it inherits Jev's rules: the allowed origins, the
irreversible-action gate (stricter, with no model to clear a shortlisted
control, and stopping any press into another site's frame), approvals and
policy modes, cookie banners refused but never accepted (untouched with
refusal off), a covered control never pressed (by a click or by Enter or
Space while it has focus), secrets never read and reported as `[secret]`,
screenshots with filled secret fields blacked out and withheld while a typed
secret shows, and page content marked untrusted.

The `jev_tab_*` tools are registered with `jev_browse` and drive only the
thread's Jev tab, under the session's lock. `roder-ext-chrome` makes its
direct CDP toolset public as `roder_ext_chrome::direct` (a `DirectSession` on
a tab by endpoint and target id, a `DirectGuard` for the owner's rules,
`direct_tools` to bind it as model-facing tools, and the shared `devtools`
helpers Jev's own connection now uses); Roder Desktop's integrated browser
fallback goes through the same client and gains a screenshot.

Jev also uncovers a covered target before giving up: Escape, the covering
layer's own close control, or a press outside it, then the action goes ahead
in the same step. Escape and Enter are sent without a native key code, which
on macOS made Escape open Chrome's "About Chrome" page from a shown tab.

Breaking: `JevExtension` is a struct (`JevExtension::new()`,
`with_inference_engines`); `JevToolContributor::new(engines)`;
`JevActOutcome` and `JevActionRecord` gain `uncovered`; `JevRunResult` gains
`stop_cause` (the new `JevStopCause`).

#### Jev results show the page, and Jev reports access blocks and reads widget frames

The model only ever reads a tool result's text, and `jev_browse` used to give
it one line ("done at <url> (3 actions)"). The text is now a bounded digest
of the call, at most 8,000 characters and 120 lines:

- A header with the status, the session call and tab, today's date, time and
  time zone, the page's address, title and HTTP status (marked
  page-supplied), the outcome and what to do next. After `done` the caller is
  told to check the page against the goal, how to continue with `url ""`, and
  not to sign in, reserve, pay or send personal details unless the user asked.
- The session's tabs, totals and earlier calls.
- Between marker lines that say it is untrusted: why Jev stopped, each step
  with the section its control sat in and its effect, the text of frames Jev
  read, the headings, the page text, and the options Jev can act on, grouped
  by the card or section they sit in.

The result's data gains `controls`, `page` (`http_status`, `headings`,
`frames`) and each step's `effect` and `context`. `JevActionRecord` gains a
`context` field; `JevControl`, `JevPageFacts` and `JevFrameText` are new
public types, and `JevBrowser` gains a default `describe` method.
`JEV_SESSION_LOG=<dir>` appends one JSON line per call for grading.

Jev also looks before it decides:

- A site that refuses automated access (HTTP 401, 403 or 429 with a refusal
  page, a challenge address, or an "Access Denied" page with nothing to act
  on) ends the call with the new status `access_denied` before any decision,
  with the evidence. Jev only reports it and never tries to get around it.
- A first observation that shows nothing yet is read again for up to about
  2.6 s, and a BLOCKED about a blank page is checked once, at no decision's
  cost.
- The text of up to two large, visible frames of another origin (a booking
  widget) is read into the page text and the result; their controls are not
  offered. `JEV_FRAME_TEXT=0` turns this off.

#### Jev keeps one browser session per thread

`jev_browse` calls on one Roder thread now share a session: the first call
opens a tab, and later calls go on in that tab from where the last one
stopped instead of opening a new tab each time.

- `url` is optional: `""` continues on the tab's page without reloading; a url
  loads in the same tab (after its new document commits) and is not reloaded
  when the tab is already there. A new `tab` argument takes `current`
  (default), `new`, `reset` or `close`. At most three tabs stay open; tabs an
  action opens are kept; tabs are no longer closed at the end of a call,
  background ones included.
- The per-call `allowed_origins` argument is removed. A call may go to any
  origin unless the operator sets `JEV_ALLOWED_ORIGINS`. `null`, `""` and `[]`
  mean "not given" for every optional argument.
- A session keeps its last eight calls, running totals, typed secrets (still
  scrubbed from later page reads) and resolved models; each result's text
  names the tab and how the call came to be on it, the tabs open, the totals,
  earlier calls and how to continue, and `data.session` holds the same. The
  tool description states today's date and time zone.
- A closed or moved tab is reported and recovered; overlapping calls on one
  thread wait for each other (or return `busy`); idle sessions close after
  `JEV_SESSION_IDLE_SECS` (default 1200), and a per-process ledger lets a
  later process close the tabs of one that died.
- The approval names the thread's tab and where Jev works in it; `tab:
  "close"` is allowed in every policy mode.
- `JevRunResult` gains a crate-private field, so it can no longer be built
  outside the crate.

### Features

#### Jev fixes from a live booking benchmark

A new live benchmark (`scripts/jev-booking-bench.sh`, graded by
`scripts/jev_booking_grade.py`) runs a restaurant-booking request through the
installed `roder` binary and grades where it got, what the answer said, that
no commit, sign-in or personal-data step was taken, and that the calls stayed
in one tab. Its runs led to these changes:

- `tab: "new"` right after a call whose page refused access loads the url in
  that tab instead of leaving the refused page open beside a new one; the
  result says why (`tab_note: "navigated"` with a `tab_detail`).
- After `access_denied` the hint says the task is not finished, to go on at
  another site with its url in the same tab, and that switching sites needs
  no confirmation; the tool description says the same.
- The `url` description says the thread's first call needs one, and a first
  call without one says what to send instead of only refusing.
- A heading that titles one card of a list no longer names controls after the
  list (a results map's links were listed as the last card's options).
- In a session tab that already shows a page, a load Chrome abandons
  (`net::ERR_ABORTED`, not a download) is tried once more.

### Fixes

#### Jev session and result fixes from review

- A continued call no longer adopts, or closes, tabs the user opened from
  Jev's tab between calls: every page open when the call re-attaches is
  recorded as seen, and only tabs this call's own inputs open are followed.
- A session leaves the registry (close, idle sweep, eviction) only under its
  own lock and marked closed; a call that was waiting for it moves to the
  thread's new session instead of opening tabs nothing would ever close.
- A `new` tab asked for right after `access_denied` loads in the refused tab
  only while that tab is still open; otherwise the call opens the new tab it
  asked for rather than loading over the tab before it.
- With `JEV_ALLOWED_ORIGINS` set, a call whose tabs are gone does not reopen
  a last page outside those origins; it ends `blocked` without loading it.
- The address a continued call finds its tab at is scrubbed of typed secrets
  before it is compared and reported, so a password in a GET form's address
  never reaches `session.moved_to` or the result text.
- Frame text, frame origins and every control's `section` are scrubbed of
  typed secrets like the rest of the page.
- The step fingerprint counts frames of another origin by origin, not text,
  so a rotating ad frame cannot defeat the stall rule.
- A `/sorry/` address counts as a bot challenge only with a refusal status,
  a refusal phrase or nothing to act on.
- The result text defuses any run of three or more dashes, ASCII or
  look-alike, so page text cannot draw a marker line; and when a flow runs
  past midnight it says the user's "tonight" is the date the session began
  on (`session.began_on`).
- The tool's goal example and wording no longer carry the booking
  benchmark's task, and the advice to move to another site after
  `access_denied` without asking applies only when the user named no
  particular site.
- The booking benchmark grader passes only runs that reached the reservation
  panel, and reads its date, party size and time off the panel.

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
