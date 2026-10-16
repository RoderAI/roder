# Semantic DOM evidence and experimental Jev prompting


`jev_browse` tool results now include a bounded semantic DOM snapshot inside the
untrusted page-content section. Like Codex browser snapshots, it preserves
landmarks, forms, headings, rows and controls with their observed states instead
of relying solely on flattened text. `node` references identify observed controls;
they do not introduce a new action API. The tree covers the visible composed DOM,
open shadow roots and same-origin frames, plus offered offscreen controls.
It is capped at 100 lines / 6,000 UTF-16 code units before the truncation marker,
with a smaller excerpt in the existing 8,000-character tool digest. Hidden content
and secret field values are excluded, and previously typed secrets are scrubbed.
This is an approximation of semantic DOM structure, not Chrome's full accessibility
tree; cross-origin frames retain their existing separate read-only text path.

To try the experimental prompt using that evidence:

```sh
JEV_PROMPT_PROFILE=dom JEV_IMAGES=1 JEV_MODEL=jev-preview roder
```

`JEV_PROMPT_PROFILE` accepts `workflow` (the default) or `dom`; other values fail.
The `dom` profile supplies structure, form ownership, available targets per
operation, and explicit completion-versus-pending-update criteria. It retains the
same offered actions and irreversible-action gate. It can also run without images.
For paired evals, append `-vision` to any Jev profile, for example
`jev_workflow-vision@jev-preview` or `jev_dom-vision@jev-preview`.

For example, a synthetic account page produces this shape:

```text
- main
  - heading "Account" [level=1]
  - form "Profile"
    - textbox "Name" [node=2] [value="Ada"]
    - combobox "Country" [node=3] [value="Chile"]
    - button "Save" [disabled]
  - status
    - text "Saved account"
```

Nodes link to each model request's offered target through its explicit `node`
field. A disabled button can appear as evidence without becoming an executable
target. This avoids turning page descriptions into permission to act.
