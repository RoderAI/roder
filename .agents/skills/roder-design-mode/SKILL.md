---
name: roder-design-mode
description: Use when the user asks to generate, modify, review, export, import, or orchestrate agent work in Roder Desktop Design Mode / Design Canvas. Guides agents through using the Pencil-like design canvas features, design/* tools, prompt nodes, scoped agents, tokens, components, templates, screenshots, and SVG handoffs.
version: "1.0.0"
---

# Roder Design Mode

Use this skill when the user wants to **create or use Design Mode** in Roder Desktop: generating mockups, editing a `~/.roder/design/<project-slug>-<project-id>.roderdesign` document, using the visual canvas, importing `.pen`/JSON designs, creating tokens/components, exporting SVGs, reviewing screenshots, or running scoped design agents.

Users can invoke this skill as `$roder-design-mode` or the shorter `$design` alias. Prefer `$design` in examples and follow-up instructions when autocomplete ambiguity would otherwise match other `roder-*` skills.

Design Mode is a structured, Pencil-like design surface backed by `~/.roder/design/<project-slug>-<project-id>.roderdesign` documents. Treat it as a first-class design system and agent-editable document, not as a free-form drawing file.

## Start Here

1. Clarify the user's desired outcome:
   - generate a new screen/mockup,
   - modify selected design nodes,
   - create or update design tokens,
   - import an existing Pencil/JSON design,
   - export/attach/review SVG output,
   - run scoped agents on frames/groups/components.
2. Inspect current design state before changing it:
   - Prefer `design/get_editor_state` when available.
   - Use `design/batch_get` for specific node ids, parent scopes, search patterns, and shallow subtree reads.
   - Use `design/get_variables` before token/style work.
   - Use `design/snapshot_layout` to understand layout diagnostics.
3. Make changes with typed operations only.
4. Validate layout and provide a visual/export handoff when helpful.

## Core Tool Workflow

Use this default loop:

```text
design/get_editor_state
-> design/batch_get relevant nodes/scopes
-> design/get_variables if style/token work is involved
-> propose concise plan
-> design/patch or design/set_variables
-> design/snapshot_layout
-> design/export_nodes or design/get_screenshot when review/output is needed
```

Stay inside the Design Mode tools. Do **not** inspect Roder source files such as
`crates/roder-tools/src/design.rs`, protocol structs, or desktop components just
to learn operation shapes. The operation contract is listed below. Only inspect
implementation source when a required design tool is missing, failing, or the
user explicitly asks to debug Design Mode itself.

### Agent tool names and strict argument shapes

When you are running as an agent, use the underscore tool names exposed by the
tool system. Do not invent app-server JSON-RPC wrappers.

- Read: `design_read` with `{}`.
- Read nodes/search: `design_batch_get` with `node_ids`, `parent_id`,
  `patterns`, `read_depth`, `search_depth`.
- Patch: `design_patch` with **exactly** `{ "operations": [...] }`.
- Variables: `design_set_variables` with `{ "variables": {...}, "replace": false }`.
- Validate: `design_snapshot_layout` with `{}`.
- Export: `design_export_nodes` with `{ "node_ids": ["n1"] }`.
- Scoped lanes: `design_spawn_agents` with `{ "scope_node_ids": ["n1"], ... }`.

Never call `design_patch` with a top-level `patch` object. The `patch` field is
only valid inside an `update_node` operation:

```json
{
  "operations": [
    {
      "op": "update_node",
      "nodeId": "n1",
      "patch": { "name": "Agent Browser Alias Test" }
    },
    {
      "op": "insert_node",
      "parentId": "n1",
      "node": {
        "id": "rect-green-test",
        "type": "rectangle",
        "name": "Green test rectangle",
        "x": 48,
        "y": 48,
        "width": 180,
        "height": 96,
        "visible": true,
        "fill": { "kind": "color", "value": "#22c55e" }
      }
    }
  ]
}
```

When generating a design, do not stop after tokens. A complete generation should
produce at least one visible root frame with meaningful child nodes, then run
layout validation and export/screenshot if requested.

### Tool and Method Cheat Sheet

- `design/get_editor_state`: compact state, selected ids, rules, available schema.
- `design/batch_get`: read/search nodes by ids, parent, pattern, and depth.
- `design/patch`: mutate nodes with typed operations.
- `design/set_variables` / `design_set_variables`: merge or replace token variables.
- `design/get_variables`: read design tokens.
- `design/snapshot_layout`: get layout diagnostics.
- `design/export_nodes`: export selected nodes/frames/components as SVG files.
- `design/get_screenshot`: get SVG screenshot/data URL fallback for full document or node.
- `design/spawn_agents` / `design_spawn_agents`: plan scoped agent lanes for containers.
- `design/get_guidelines`: read design guidelines when available.

## Patch Operations

Only mutate the document with typed patch operations:

- `insert_node`: add a node under root or a parent container.
- `update_node`: edit node properties.
- `delete_node`: remove a node, usually recursively for containers.
- `reorder_node`: change layer order among root nodes or siblings.
- `set_variables`: merge/replace design tokens when a dedicated variable setter is unavailable.

Do **not** hand-edit `.roderdesign` JSON unless explicitly asked and no design tooling is available.

## What Agents Can Create

Useful node types include:

- containers: `frame`, `group`, `component`, `instance`,
- content: `text`, `image`, `prompt`,
- shapes/vectors: `rectangle`, `ellipse`, `line`, `path`, `icon`.

Common properties:

- geometry: `x`, `y`, `width`, `height`, `rotation`, `opacity`, `cornerRadius`,
- layer state: `visible`, `locked`,
- paint: `fill`, `stroke`, `stroke.width`,
- text: `content`, `fontSize`, `fontWeight`, `textAlign`,
- components: `componentId`, `sourceComponentId`, `reusable`, `overrides`,
- imported data/source metadata: preserve unknown data instead of deleting it.

## Generating Designs

When asked to generate a UI/screen:

1. Choose or create a root `frame` sized for the target surface.
2. Use tokens for color, spacing, and typography where possible.
3. Prefer semantic subtrees:
   - hero sections,
   - cards,
   - forms,
   - navigation/header/footer,
   - reusable components and instances.
4. Keep generated designs simple enough to map to real app components.
5. After patching, run layout diagnostics and fix obvious overlaps/out-of-bounds issues.
6. Export the root frame or get a screenshot for review.

Minimal viable generated screen:

- one root `frame` with a clear name and target size,
- 3-8 meaningful child nodes or groups,
- text content that describes the product/user journey,
- fill/stroke/text styles wired to variables where practical,
- a layout validation result.

If you first call `design/set_variables`, immediately follow with `design/patch`
to apply or create the visible design unless the user only requested token work.

## Using Design Tokens

Before token edits, call `design/get_variables`.

Use variables for:

- colors: `brand.primary`, `surface.card`, `text.muted`,
- spacing: `space.4`, `space.8`, `layout.gutter`,
- typography: `type.heading`, `type.body`, `type.caption`.

For token-only edits:

- prefer `design/set_variables` or `design_set_variables`,
- use `replace: false` for merges unless the user asks to replace the design system,
- after token changes, update affected nodes if the design should immediately reflect the new tokens.

## Components and Instances

Use components when repeated UI elements appear:

- turn reusable containers into `component` nodes,
- create `instance` copies for repeated usage,
- preserve `sourceComponentId` and local `overrides`,
- use `reorder_node` and group/child-layer controls instead of rebuilding subtrees unnecessarily.

When editing an instance, track local differences in `overrides` when the UI/tooling supports it.

## Prompt Nodes and Agent Work

Prompt nodes are on-canvas design tasks. When using them:

- include prompt text,
- include node bounds,
- include selected/current container context,
- include design tokens and component context,
- include safety permissions.

For scoped agent work:

1. Pick container scopes (`frame`, `group`, `component`, `instance`).
2. Use `design/spawn_agents` or `design_spawn_agents` to validate and plan lanes.
3. Respect permission mode:
   - observe/proposal-only,
   - review-gated patch/export,
   - autonomous patch/export if explicitly allowed.
4. Summarize each lane and what it is allowed to mutate.

## Importing and Libraries

For imported `.pen`/JSON content:

- map compatible shapes into Design Mode node types,
- preserve unsupported source data in `source` metadata,
- use `path`/`icon` for vector data when available,
- import tokens/libraries separately when useful,
- run `design/snapshot_layout` after import.

For project-specific `.roderdesign` libraries:

- scan or read library docs if available,
- import reusable components/tokens rather than duplicating everything blindly,
- keep imported source metadata so users can audit origins.

## Review, Export, and Screenshots

For user review:

- use `design/export_nodes` for SVG artifacts,
- use `design/get_screenshot` for a fast SVG screenshot/data URL fallback,
- attach selected/root frame exports to the composer when the user wants iteration,
- include enough context for the reviewer/agent: node id, name, type, bounds, parent, variables, and export path.

If exporting multiple screens, prefer root frames/components.

## Safety Rules

- Never delete or reorder large subtrees without user intent or a clear plan.
- Prefer small, reversible patches.
- Preserve unknown fields and imported source data.
- Respect locked/hidden layers when making visual edits.
- Run layout diagnostics after structural changes.
- Ask for clarification when the requested design direction is ambiguous, high impact, or destructive.
- Avoid long source-code reconnaissance during design generation. The user wants
  a design artifact; use design tools directly and keep momentum toward a patch,
  export, or review artifact.

## Response Pattern

When using this skill, respond with:

1. brief intent: what design action you will take,
2. inspected state summary,
3. patch/export/agent actions performed,
4. validation results,
5. any next review step, such as opening the Design panel, exporting SVG, or running scoped agents.

If a prior Design Mode turn contains failed design tool calls, stale ids, or an old `patch`/`operations` mismatch, start a fresh Design Mode turn with `$design` and re-read `design_get_editor_state` before patching. Do not keep retrying against stale tool-call context.

## Node aliases

`design/get_editor_state`, `design/batch_get`, `design_read`, and `design_batch_get` expose `nodeAliases` such as `n1`, `n2`, and `n3`. Use these short aliases when reasoning and when calling design tools; the app-server/tools resolve aliases to canonical node ids for reads, patches, exports, screenshots, selections, and scoped-agent plans. Mention both alias and name when explaining changes to the user.
