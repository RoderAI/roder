---
name: design
description: Alias for $roder-design-mode. Use when the user asks to generate, edit, inspect, export, or orchestrate Roder Desktop Design Canvas work.
---

# Design Mode Alias

This is a short alias for `$roder-design-mode`.

When this skill is invoked, immediately follow the full Roder Design Mode workflow:

1. Load or inspect the active Design Canvas document with `design_get_editor_state` or `design_read`.
2. Use simple node aliases (`n1`, `n2`, …) from tool results when referencing nodes.
3. Mutate with `design_patch` using a top-level `operations` array only; never use a top-level `patch` argument.
4. Use `design_set_variables` for token-only changes.
5. Validate with `design_snapshot_layout` and export/attach/review when requested.

Do not inspect desktop implementation source unless the design tools are unavailable or the user asks for implementation work.
