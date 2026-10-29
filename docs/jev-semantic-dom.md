# Semantic DOM evidence and experimental Jev prompting


Jev, Decisions and direct browser tools share a semantic row contract and Rust
renderer (`roder-ext-chrome::semantic`). It shows landmarks, forms, headings,
table cells, controls and observed state in a bounded viewport hierarchy. After
an action, the next decision receives added, changed and removed rows plus new
page-text lines; unchanged navigation and controls are omitted. An incomplete
observation, changed document or changed URL forces a full view. Private probes
do not advance the model-visible baseline. Full raw observations remain available
inside the runtime for freshness checks and recovery.

Native Jev capture follows the visible composed DOM, open shadow roots and
same-origin frames. Its complete offered action map also retains offscreen
targets. The tree is capped at 100 rows and 6,000 characters; truncation is marked,
and missing rows in a capped view never imply removal. This approximates semantic
structure rather than Chrome's full accessibility tree. Cross-origin frames and
dialogs retain their separate read-only evidence.

Internal `j…` refs resolve through the offered action map sent to the decision
provider. A completed `jev_browse` handoff also reads the same tab through the
direct tools and returns `e…` refs accepted by `jev_tab_*`, seeding the next
manual action's diff. `page.reference_tools` identifies that tool family. Call
`jev_tab_look` for a full current view or fresh refs. If the bounded handoff read
cannot complete before the deadline, the native observation remains available
and its refs continue to identify offered actions. Geometry stays in structured
diagnostic data rather than the semantic text.

The tool digest wraps page content as untrusted text. Hidden content and secret
field values are excluded; known secrets are scrubbed before formatted labels or
text excerpts are shortened. Overlong labels are omitted whole to avoid exposing
an unmatched prefix of a secret.

To try the experimental prompt using that evidence:

```sh
JEV_PROMPT_PROFILE=dom JEV_IMAGES=1 JEV_MODEL=jev-1.14.0 roder
```

`JEV_PROMPT_PROFILE` accepts `workflow` (the default), `dom`, `dom_targets` or `activation`;
other values fail.
The `dom` profile supplies structure, form ownership, available targets per
operation, and explicit completion-versus-pending-update criteria. It retains the
same offered actions and irreversible-action gate. It can also run without images.
For paired evals, append `-vision` to any Jev profile, for example
`jev_workflow-vision@jev-preview` or `jev_dom-vision@jev-preview`.

`dom_targets` uses the same DOM questions as `dom`. When an offered populated
editable field is visibly clipped or internally scrolled, screenshots are scoped to conditional target-selection
questions through their instructions. Operation and irreversible-action questions
then use the full DOM values without image evidence.
The clipping flag comes from observed control scroll/overflow metrics.
This keeps clipped or horizontally scrolled input text in a screenshot from
overriding the complete observed field value when selecting the next operation.
Images remain available for locating controls by their appearance; no additional
model call is needed. Other pages keep the image in shared state, including
confirmation pages where visual outcome evidence remains useful. TypeSafe expands
each target question's image reference, so measure token usage as well as outcomes.
The paired eval name is `jev_dom_targets-vision@jev-preview`.

`activation` adds an experimental completion audit to `dom_targets`. When Jev
selects DONE and supplies a valid conditional click target, a second, text-only
request asks three independent Nouls: whether that activation is required,
whether it has already happened, and whether its prerequisites are ready.
Code combines their estimated probabilities as
`max(0, required + ready - performed - 1)`. If this conservative support exceeds
0.5, it resolves the original offered CLICK target, retaining that target's raw
Choice probabilities and applying the existing irreversible-action gate.
The corrected operation's confidence field contains this derived support;
it is not raw Choice confidence or a guarantee of correctness.

A clearly required, unperformed activation whose prerequisites are unproven
produces an unusable answer, allowing the existing bounded retry/fallback policy
to handle it. Missing or refused unused click targets cannot supply a correction.
This check is limited to one conditional click and does not establish universal
goal completion. It adds a billed request on audited DONE decisions; token and
call totals include both requests. The default remains `workflow` with images
off. To evaluate the audit, use
`JEV_PROMPT_PROFILE=activation JEV_IMAGES=1 JEV_MODEL=jev-1.14.0` or paired eval
variant `jev_activation-vision@jev-1.14.0`.

For example, a synthetic account page produces this shape (refs are illustrative):

```text
Snapshot (act by ref; indentation shows viewport hierarchy):
- main
  - heading "Account" [level=1]
  - form "Profile"
    - j123-2 textbox "Name" = "Ada"
    - j123-3 combobox "Country" = "Chile"
    - button "Save" [disabled]
  - status "Saved account"
```

A disabled button can appear as evidence without becoming an executable target.
Refs describe a target within the supplying browser backend; the offered action
map and existing gates still determine which operations may execute.

## Whole-task delegations and readable text

`jev_browse` accepts the entire browser task, including comparisons and follow-up
product or detail checks. The parent assesses the returned evidence and uses
manual tools only for facts that are missing or contradictory. A driver's DONE
claim still requires evidence; historical observations do not prove current
control state or authorize further actions.

Within one delegation, new observed text lines are retained with their source
URLs in a bounded `evidence` string. The driver receives it on later steps, and
the parent receives it with the result. Repeated lines from the same source are
omitted. Old records are evicted above 8,000 characters with an explicit omission
notice. Secrets learned later are scrubbed again when evidence is returned.
Hosted request paging includes retained evidence alongside current text and DOM;
it does not bypass the request or per-question limit. The result digest allows
18,000 characters / 180 lines, below the long-output spill thresholds.

Direct browser text now describes viewport-visible text records rather than the
beginning of `document.body.innerText`. It excludes hidden and clipped content
and field values; values remain in the existing guarded control observations.
Scrolling changes the text evidence even when the document itself is unchanged.
Whole text records reach the Rust scrubber before display excerpts are cut.
Screenshot encoding is unchanged.

Fallback compaction retains up to 8,000 characters of earlier viewport text with
source URLs after old control lists expire. Its final prose findings are retained
up to 8,000 characters and rendered before action traces; they are not reduced to
a short status line. Uncertain repeated clicks stop as `blocked` with
`uncertain_repeat`, not `done`, and configured fallback can inspect completed
effects before continuing the rest of the task in the same call.

For reading tasks, `jev_tab_look` accepts `text_scope: "document"`. This reads
rendered document text and observed HTTP(S) links across scroll positions without
scrolling or changing the viewport-diff baseline. Hidden content and field values
are excluded. Each response contains at most 6,000 text characters / 100 text
lines, and `next_text_offset` identifies the next page. Restart at zero if the
page changes. Whole records are scrubbed before pagination; the extraction cap
is explicitly reported and is not evidence that omitted text is absent.

When automatic fallback resolves successfully and `JEV_MAX_ACTIONS` is unset,
the session gives the native decision driver 12 actions before escalation.
An explicit operator limit wins. With no available automatic fallback, the
engine's existing default remains. This bounds time spent on an unsuitable
strategy while preserving the complete task within one delegation.
