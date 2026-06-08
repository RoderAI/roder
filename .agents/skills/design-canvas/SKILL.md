---
name: design-canvas
description: Use when inspecting, editing, reviewing, exporting, or orchestrating agents on Roder Desktop Design Canvas `~/.roder/design/<project-slug>-<project-id>.roderdesign` documents. Covers the Pencil-like design workflow and required design/* tool order.
version: "1.0.0"
---

# Design Canvas

Use this skill whenever work touches Roder Desktop's structured Design Canvas or a `~/.roder/design/<project-slug>-<project-id>.roderdesign` file.

## Core Rules

1. Treat `.roderdesign` as a structured design document, not as arbitrary JSON to hand-edit.
2. Inspect before mutating:
   - Start with `design/get_editor_state` when available.
   - Use one combined `design/batch_get` for nodes, parent context, patterns, and read depth.
   - Use `design/get_variables` before changing tokens.
3. Mutate only through typed `design/patch` operations:
   - `insert_node`
   - `update_node`
   - `delete_node`
   - `reorder_node`
   - `set_variables`
     For token-only edits, prefer the dedicated `design/set_variables` app-server
     method or `design_set_variables` tool when available.
   - In agent tool calls, the tool is `design_patch` and its top-level argument
     is exactly `{ "operations": [...] }`. Do **not** pass a top-level `patch`;
     `patch` is only a field inside an `update_node` operation.
4. After structural edits, run `design/snapshot_layout` and fix obvious layout diagnostics.
5. Use `design/export_nodes` for visual output and composer attachments; do not invent export paths.
6. For scoped multi-agent work, use `design/spawn_agents` with frame/group/component/instance scope node ids.
7. Respect agent permissions and review requirements when a prompt or manifest includes them.

## Design Workflow

```text
inspect editor state -> batch read relevant scopes -> plan typed patch operations -> patch -> snapshot layout -> export/attach if useful
```

Prefer one high-quality batched read over many tiny reads. Keep patches small, reversible, and explainable.

## Node Semantics

- Valid container scopes include `frame`, `group`, `component`, and `instance`.
- Preserve unknown imported Pencil data under source metadata instead of dropping it.
- Preserve reusable component identity and instance override metadata when duplicating or exporting.
- For prompt nodes, include node bounds, selected context, parent/container context, tokens, and permission rules in the request.

## Desktop UI Expectations

When reviewing or changing the Design Canvas panel, preserve these Pencil-like workflows:

- floating tool rail with shortcuts,
- layers, search, lock/visibility, grouping, arrange, and child layer controls,
- inspector tokens/components/templates/libraries/import/export/history/diagnostics,
- prompt nodes and scoped agent launch plans,
- mini-map, rulers, fit-to-selection/canvas, and on-canvas quick actions,
- attach/review selected SVG and attach all root frames to composer.

## Validation

For desktop TypeScript/UI changes, run the repository's focused formatter, lint, and TypeScript checks for touched files. For app-server/tool changes, run focused Cargo tests for the design module or method being changed.

## Node aliases

`design/get_editor_state`, `design/batch_get`, `design_read`, and `design_batch_get` expose `nodeAliases` such as `n1`, `n2`, and `n3`. Use these short aliases when reasoning and when calling design tools; the app-server/tools resolve aliases to canonical node ids for reads, patches, exports, screenshots, selections, and scoped-agent plans. Mention both alias and name when explaining changes to the user.
