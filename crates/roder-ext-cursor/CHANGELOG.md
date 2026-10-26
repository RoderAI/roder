## 0.2.2 (2026-10-26)

### Fixes

#### Resume threads from topic events and scheduled follow-ups

Add agent tools and JSON-RPC methods for thread-scoped topic subscriptions,
one-shot reminders, recurring timer ticks, publication, inspection, and cancellation.
Deliver follow-ups into idle threads with retained conversation context, coalesce
pending work while busy, and stop abandoned watchers with expiration and delivery
limits. User interruption and thread archive discard pending subscriptions.

Subscriptions live in the harness process. Durable independent scheduled runs
continue to use the existing app-server automation scheduler.

Expose next tick and last admitted delivery timestamps for inspecting live watchers.

Fix Codex follow-up requests by sending volatile developer context with the supported
developer input role. Clarify nullable timer fields so one-shot reminders are not
accidentally registered as recurring timers.

Preserve exact integer types in Cursor MCP arguments so timer delays, delivery
limits, and TTL values pass runtime integer validation.

Tell final-delivery turns that their subscription is already exhausted, avoiding
redundant cancellation calls after one-shot reminders.

## 0.2.1 (2026-10-23)

### Features

#### Restore reliable Cursor native tool execution

Preserve structured, policy-approved results across provider-native tool loops.
Cursor reads, searches, listings, writes, deletes, and shell replies now use the
runtime executor's output and failures. Add current client-side MCP dispatch,
default runtime tool discovery and MCP state replies, explicit unsupported-call
errors, official turn-end usage, and reliable stream
failure and heartbeat cleanup.

Persist native checkpoints and KV blobs to preserve thread continuity. Serve KV
GETs, prefetch restored blobs, and import Roder history explicitly for new Cursor
conversations and compaction summaries. Use the same bidirectional transport for
text-only API calls and tool-enabled turns.

### Fixes

#### Refresh Cursor Gemini models

Add Gemini 4 Argon with a 1M-token context window and high effort by default.
Send its registry-supported reasoning_effort parameter instead of generic legacy
parameters that Cursor rejects for this model.
Replace Cursor's stale Gemini 3.1 Pro preview id with gemini-3.1-pro and prevent
preview ids from returning through discovery or older caches.

#### Remove retired Cursor models from model selection

Remove Cursor Grok 4.3, which the service rejects as deprecated. Prevent retired
Cursor models and their effort/fast variants from reappearing through stale
caches or live discovery, including the already-retired GPT-5.5 variants.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.5 (2026-09-12)

### Fixes

#### Add Gemini 3.7 Flash and Grok 4.6 to provider model catalogs

Expose Gemini 3.7 Flash and Grok 4.6 through native, Cursor, xAI, SuperGrok,
and OpenRouter integrations with provider-specific context windows and
reasoning controls. Retire Grok 4.5 and Grok Build from active catalogs.

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
