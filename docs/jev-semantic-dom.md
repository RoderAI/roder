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
