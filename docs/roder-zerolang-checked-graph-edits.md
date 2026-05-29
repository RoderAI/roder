# Roder Zerolang Checked Graph Edits

Roder exposes Zerolang as a first-party tool provider for graph-first Zero code changes. The tools shell out to a local `zero` binary; Roder does not vendor or install the compiler.

## Setup

Binary resolution order:

1. `RODER_ZERO_BIN`
2. `[zerolang].binary` in `~/.roder/config.toml`
3. `zero` on `PATH`

Example config:

```toml
[zerolang]
binary = "/path/to/zero"
timeout_seconds = 30
artifact_dir = ".zero/roder"
```

Diagnose the local compiler:

```sh
roder zerolang doctor
roder zerolang check examples/hello.0
roder zerolang graph-dump --out .zero/roder/hello.program-graph examples/hello.0
```

## Tool Surface

The model-facing tools are:

```text
zerolang_skills_get
zerolang_check
zerolang_graph_dump
zerolang_graph_view
zerolang_fix_plan
zerolang_edit
zerolang_graph_roundtrip
```

There is no `zeolang_edit` alias.

Use `zerolang_skills_get` before making Zero-specific changes when syntax or workflow details are unclear. The skill content comes from the active compiler binary, so it stays version-matched.

## Checked Edit Loop

Use graph inspection before editing:

```sh
zero graph dump --json src/main.0
```

Then call `zerolang_edit` with the inspected `graphHash`, node IDs, and field or node-hash preconditions. Roder builds patch text internally:

```json
{
  "input": "src/main.0",
  "graphHash": "graph:74f634ccb5b77646",
  "operations": [
    {
      "op": "set",
      "node": "#89f1bc7e",
      "field": "value",
      "expect": "65",
      "value": "66"
    }
  ]
}
```

Use structured Roder operation objects in `operations`; this is not the same shape as `zero_graph_patch` raw patch arguments. Use `node`, not `id`, and quote semantic values as strings. The example above generates:

```text
zero-program-graph-patch v1
expect graphHash "graph:74f634ccb5b77646"
set node="#89f1bc7e" field="value" expect="65" value="66"
```

`zerolang_edit` runs:

```sh
zero graph patch --json <input> --patch-text <generated-patch>
zero graph check --json <input>
zero check --json <input>
```

when validation is enabled and the edit writes `.0` source.

## Relationship To Text Edits

`edit`, `multi_edit`, and `apply_patch` remain the normal tools for non-Zero files and broad source-text changes. Prefer `zerolang_edit` for mechanical Zero source changes that can be expressed against ProgramGraph facts, especially renames, literal/value changes, inserts, deletes, and node replacements. The graph path gives the compiler the stale-graph hash, node IDs, expected field values, formatting, reparsing, and semantic checks in one operation.

Derived `.program-graph` artifacts should normally stay under `.zero/roder/` and should not be committed unless the user explicitly asks for graph artifacts.

## Live Smoke

Normal tests use fake `zero` binaries. A real compiler smoke is opt-in:

```sh
RODER_ZERO_LIVE=1 RODER_ZERO_BIN=/path/to/zero cargo test -p roder-ext-zerolang live_zero -- --ignored --nocapture
```
