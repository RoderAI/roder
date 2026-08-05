## 0.1.4 (2026-08-05)

### Fixes

#### Add Ultra mode as a first-class multi-agent mode for any model

Make Codex Ultra's proactive multi-agent policy a concrete Roder mode
(`/ultra`, `thread/set_ultra_mode`, `settings/get.ultraMode`,
`ultra/modeChanged`), available for every model — not only Sol/Terra Ultra
reasoning effort. Sol/Terra Ultra effort still maps to max wire effort and
enables proactive multi-agent without requiring the mode flag.

Also: `task` / `agent_swarm` children inherit the parent thread's live
provider+model (so SuperGrok stays on grok-4.5), and lane `max_concurrent`
can be raised per request for large fanouts instead of hard-failing at the
old scout cap of 4.

## 0.1.3 (2026-06-26)

### Features

#### Cursor fast variants, reasoning params, and stable conversation ids

Expose `composer-2.5-fast` and `gpt-5.5-fast` as first-class catalog models, encode AgentService `fast`/`effort`/`thinking` params from Roder reasoning config, reuse a stable per-thread Cursor `conversation_id`, and open the reasoning submenu when selecting Cursor models that advertise effort options.

## 0.1.2 (2026-06-26)

### Fixes

- Emit estimated token usage from Cursor bidirectional agent turns so the TUI thread token count updates after Cursor/Opus completions.

## 0.1.1 (2026-06-15)

### Fixes

#### Prevent Cursor protobuf decoder panics on malformed payloads

Reject overlong varints and out-of-bounds protobuf fields as decode errors so
unexpected Cursor frames do not stop the agent turn.

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
