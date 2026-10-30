# Native browser recovery and research

`jev_browse` can keep a complete browser task inside one delegation. The parent
should assess the returned evidence and use manual tools only for missing or
contradictory facts. The final user reply remains ordinary prose with sources.

## Task kinds and requirements

`task_kind` is `interaction` by default. This mode uses full observation freshness
before accepting DONE. Use it for tasks that must leave a particular UI state,
selection, submitted form or mutation. Action identity, form state, hit testing,
origin restrictions, secret scrubbing and irreversible-action gates still apply.

Use `task_kind: "research"` for reading and comparing source facts. This mode
reads one bounded document page on arrival and accepts research completion from
retained observations while checking that the same document remains loaded.
It does not require a saved-form confirmation for a reading task. Explicit
`success_condition` predicates still run and can reject completion.

An optional `requirements` list supplies up to 12 nonempty strings, each at most
500 bytes. In research mode the native loop works on one requirement at a time,
retaining the overall task's constraints. A DONE claim advances that requirement;
only the last one can finish the delegation. The result records
`requirement_completed` events with source references and
`verification: "driver_claim"`. These are claims to inspect, not independent
proofs. Checkpoints do not reset action, provider-request or time budgets.

```json
{
  "url": "https://example.com/catalogue",
  "task_kind": "research",
  "goal": "Compare the listed plans and verify the selected plan's limits. Do not register or buy.",
  "requirements": [
    "Compare published prices and included features.",
    "Open the selected plan's details and verify its limits."
  ]
}
```

Embedding hosts can opt into `JevEngineConfig::with_research` and the validated
`with_requirements` builder. A custom `JevBrowser` advertises document reads and
implements `document_text`; browsers without them keep the existing observation
path. `fresh_document` defaults to the stronger full-observation check. The
built-in Chrome implementation compares document identity and URL for research
completion while keeping the stronger guards for actual actions.

## Reading and evidence

The native `READ_DOCUMENT` operation uses the same rendered-text extractor as
`jev_tab_look` with `text_scope: "document"`. It includes observed HTTP(S) links
and offscreen rendered text, puts a main landmark's content before repeated
navigation when available, and retains the rest of the rendered document.
Hidden content and field values are excluded. It does not scroll the viewport.

Complete text is scrubbed before pages of at most 6,000 characters and 100 lines
enter bounded retained evidence. The extractor caps scanning at 20,000 nodes and
100,000 characters and marks omissions; missing extracted text is not proof of
absence. Further native reads continue the document snapshot. Dispatched content
changes and an external rescue invalidate that snapshot before another read.

The result's `evidence` is historical source data. It must not be treated as proof
of current control state or as instructions. The same request and per-question
budget controls page retained evidence before sending it to Jev.

## Local recovery

A stale action reports the changed fields, such as destination, form state,
context, scroll or page text. Diagnostics name fields without copying their raw
values. One navigation can be revalidated without another model call only when a
fresh observation proves the exact node, label, full target guard, document, URL
and form state unchanged. The normal input guard and hit test run again. A changed
target is never replaced by another control sharing its label.

An uncertain repeated navigation is suppressed. Once per run the native loop can
refresh and replan with that duplicate excluded. Observed HTTP(S) anchors can
qualify even when ARIA exposes them as menuitems. Buttons, commitment labels and
high irreversible probabilities retain the stop. The result records bounded
`recoveries` diagnostics, including a suppressed navigation and its replan.

When these bounded attempts cannot progress, the configured full fallback can
continue inside the same call. It inherits scope, gates and secrets. It cannot
bypass missing input, required confirmation or refused access.

The one-tool Terra rescue followed by native hand-back is an opt-in **evaluation
experiment**, not the default fallback policy. Run it with
`bounded_rescue_handback`, `JEV_EVAL_FALLBACK=model` and the usual model settings.
The same native engine retains its budget, history, requirements and guards.
The paired corpus runner also accepts `JEV_EVAL_FALLBACK=model` for a full-fallback
control. Combined grading uses the complete goal even when a native driver
finishes a formerly unsupported task without needing fallback.

See [the evaluation report](validation/jev-native-recovery-2026-10-28.md) for
build provenance, live outcomes and the limits of the measurements.
