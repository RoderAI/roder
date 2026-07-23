## 0.1.14 (2026-07-23)

### Fixes

#### Add DeepSeek Platform inference provider

Adds first-class `deepseek` provider support labeled "DeepSeek Platform", using
DeepSeek's OpenAI-compatible Chat Completions API at `https://api.deepseek.com/v1`
with `DEEPSEEK_API_KEY` auth and built-in models `deepseek-chat`,
`deepseek-reasoner`, `deepseek-v4-flash`, and `deepseek-v4-pro`.

## 0.1.13 (2026-07-21)

### Fixes

#### Hosted browser authentication and request policy seams

Allow hosted deployments to resolve external bearer credentials into dynamic
tenant contexts, authenticate browser WebSockets through the
`roder.remote.v1` subprotocol, and apply a deployment request policy that can
rewrite or deny JSON-RPC calls before dispatch. Hosted health probes remain
unauthenticated for deployment schedulers.

Open hosted sockets now revalidate their bearer before every request and on a
bounded timer while idle, so external credential expiry and service-account
revocation stop request dispatch and notification delivery without waiting for
the client to reconnect or send another message. The gateway periodically
evicts idle tenant runtimes and stops that lifecycle loop during shutdown.

Externally resolved tenant ids now map to collision-resistant data directories;
existing lowercase slug tenant directories retain their original paths.

`turn/start` can now refresh a thread's volatile MCP bearer token without
persisting the credential in thread metadata.

## 0.1.12 (2026-07-21)

### Fixes

#### Fix provider compaction thrashing and show token/duration summaries

Persist OpenAI/Codex compaction items as soon as the stream emits them so a
later SSE decode failure cannot drop the boundary and re-compact every round.
Surface before/after estimated tokens and elapsed time in the TUI and
app-server item stream.

#### Add bounded lifecycle recovery, cleanup proof, and shutdown diagnostics

Roder now persists redacted per-turn lifecycle records, reconciles interrupted
turns after restart, and reports bounded cleanup ownership rather than treating
an aborted runtime task as proof that provider work was reaped. Local process
tasks drain through graceful signal, forced kill, and reap; remote tasks use the
remote runner cancellation API; and the Claude Code provider uses a vendored SDK
cleanup path with offline real-child regression coverage.

The app-server adds lifecycle notifications, `runtime/drain`, and
`lifecycle/metrics`; the CLI and TUI expose durable recovery state. A shared
`[lifecycle]` configuration controls shutdown budgets, task policy, bounded
process diagnostics, and compatible legacy shutdown fallbacks.

## 0.1.11 (2026-07-10)

### Fixes

#### Match Codex V2 Ultra agent lifecycle semantics

#### Added

- Added Codex V2-style canonical agent trees, full/empty/last-N context forks,
  nested agents, reusable follow-up turns, mailbox-aware waiting, and
  non-destructive interruption.
- Added exact parent model, provider, Ultra reasoning, workspace, policy, tool,
  runner, and live developer-context inheritance for spawned agents.
- Added full `team/started`, `team/member/started`, and terminal result details
  to the app-server protocol.

#### Changed

- `send_message` now queues coordination without starting an idle agent, while
  `followup_task` starts or steers the existing canonical agent thread.
- Child final results and terminal errors are delivered automatically to their
  direct parent, and completed identities remain available for later work.
- Inter-agent delivery now uses typed `MESSAGE`, `NEW_TASK`, and `FINAL_ANSWER`
  envelopes with canonical sender and recipient paths.

#### Fixed

- Fixed spawn-capacity, completion/follow-up, interruption, mailbox batching,
  acknowledgement, restart, and wait races found by comparison with Codex V2
  and Claude Code agent workflows.
- Prevented full-history children from replaying parent orchestration by making
  the newest `NEW_TASK` payload the authoritative child assignment.
- Preserved spawn-time live instructions, developer context, and model
  selection across reusable follow-up turns.
- Prevented interrupted-turn mailbox reservations from stranding queued
  messages or accepting stale delivery acknowledgements.
- Bounded recursive agent paths to five levels below `/root`, rejecting deeper
  spawns before creating team or thread state.

## 0.1.10 (2026-07-07)

### Fixes

#### Make provider browser auth robust on WSL and add Kimi Code API key login

Print auth URLs before opening browsers, fall back to WSL-friendly browser commands, and allow `roder auth login kimi-code --api-key [KEY]`.

## 0.1.9 (2026-06-30)

### Fixes

#### Blaxel sandbox runner with pause, resume, detach, and rejoin

Replace the placeholder Blaxel runner passthrough with a first-party Blaxel
Sandboxes provider that drives the real control-plane (`/sandboxes`) and
per-sandbox (process/filesystem/preview) REST APIs.

The remote-runner contract gains optional, defaulted lifecycle support so a
runner-bound thread can pause its sandbox toward standby, resume it, fully
detach (releasing the local session while keeping the sandbox alive), and
rejoin the same sandbox from persisted thread state — including across a
process restart, with no orphan sandbox creation. New `RunnerCapabilities`
flags (`pausable`, `detachable`) and `RemoteRunnerSession`/`RemoteRunnerProvider`
methods (`pause`, `resume`, `detach`, `rejoin_session`) default to no-op/false so
existing providers are unchanged.

Exposed through new app-server JSON-RPC methods (`runners/pause`,
`runners/resume`, `runners/detach`, `runners/rejoin`) and a `roder runners` CLI.
The Blaxel credential is sourced from the environment (`BLAXEL_API_KEY` /
`BL_API_KEY`, with `BL_WORKSPACE`) and never written to session state.

A selected runner now actually routes coding tools into the sandbox: a
runtime-level destination (TUI runner picker or config `default_destination`)
auto-binds new threads when the provider advertises a default workspace via the
new `RemoteRunnerProvider::default_workspace` (Blaxel opts in; other providers
are unchanged). Verified live end to end against a real Blaxel account: TUI
shell/file tools execute inside an Alpine sandbox, and pause/resume/detach/rejoin
work through the CLI.

## 0.1.8 (2026-06-26)

### Features

#### Live agent_swarm progress in the timeline

Render a live "swarm: N/total done" tick on the `agent_swarm` tool row while the
swarm is running, fed by `AgentSwarmProgress` events, instead of an empty row
until the final result. Once the `<agent_swarm_result>` arrives, the row swaps to
the existing compact child grid. When any children fail or abort, the live line
also shows the `(ok, failed, aborted)` breakdown.

#### Prompt to switch policy mode when entering agent-swarm

When agent-swarm mode is enabled from an approval-gating policy mode
(`Default` or `Plan`), the TUI now offers a one-key confirm dialog to switch to
`Accept All` so swarm children don't each stall on a separate tool approval
(roadmap 104, Task 4). Confirming maps to the existing `thread/set_mode`
(`Accept All`); declining keeps the current mode and explains that children will
wait for per-tool approval. Non-gating modes (`Accept All`, `Bypass`) and any
already-open modal suppress the prompt.

### Fixes

#### Per-thread agent-swarm mode

Scope agent-swarm mode to a single thread instead of the whole runtime (roadmap
104 follow-up), so toggling swarm mode on one thread no longer leaks the swarm
reminder into other threads sharing the runtime — aligning swarm mode with the
per-thread `thread/*` contract.

The runtime keeps a per-thread override map alongside the global default
(mirroring the team per-member policy-mode idiom): `set_agent_swarm_mode_for_thread`
stores the override and emits `AgentSwarmModeChanged` with the real `thread_id`,
and `effective_agent_swarm_mode_for_thread` resolves the per-thread override or
falls back to the runtime-global default; the turn loop now consults it. The
`thread/set_agent_swarm_mode` app-server method gains an optional `threadId`
(present -> per-thread, absent -> global, preserving legacy behavior) echoed back
in the result, and the TUI passes its current thread id. `settings/get` continues
to expose the global default. Covered by runtime unit tests (per-thread
isolation, off-override-wins, real-thread-id event) and an app-server e2e test.

## 0.1.7 (2026-06-26)

### Features

#### Agent-swarm mode

Add a Roder-native `agent_swarm` fanout tool and `/agent-swarm` (alias `/swarm`)
commands (roadmap phase 104). A lead model can launch many homogeneous
subagent tasks from one `prompt_template` (with the `{{item}}` placeholder) over
an `items` array, optionally resuming existing agents via `resume_agent_ids`,
and receives an ordered `<agent_swarm_result>` summary with completed/failed/
aborted counts and resumable agent ids. A bounded scheduler paces launches
(initial burst then one per interval), honors an optional concurrency cap,
preserves input order, and supports cooperative cancellation. Configure via
`[agent_swarm]` or `RODER_AGENT_SWARM_*` env. The `/agent-swarm on|off|status`
command toggles a persistent swarm reminder; `/agent-swarm <prompt>` runs one
swarm task.

#### Render agent_swarm results as a compact child grid

The TUI timeline now renders an `agent_swarm` tool call with a compact swarm
summary line (`swarm: completed: 2, failed: 1`) grouped under the tool call, and
per-child outcome/item/agent-id rows when expanded, instead of the raw
`<agent_swarm_result>` XML block (roadmap 104, Task 5). Live per-child progress
continues to surface through the existing subagent trace surface.

### Fixes

#### Server-side agent-swarm mode

Move agent-swarm mode from TUI-only client state to runtime/app-server state so
every client benefits (roadmap 104). Adds the `thread/set_agent_swarm_mode`
app-server method, an `agentSwarmMode` field on `settings/get`, and an
`AgentSwarmModeChanged` event. When swarm mode is active the runtime injects the
canonical swarm reminder into each turn's developer instructions
(`Runtime::set_agent_swarm_mode` + `apply_agent_swarm_mode`), so the model is
nudged toward the `agent_swarm` fanout tool regardless of which client drove the
turn. The TUI now toggles swarm mode through the method and no longer prepends
the reminder client-side. Also fixes two pre-existing method-manifest ordering
issues (`auth/kimi-code/*`, `thread/compact`).

#### Cursor fast variants, reasoning params, and stable conversation ids

Expose `composer-2.5-fast` and `gpt-5.5-fast` as first-class catalog models, encode AgentService `fast`/`effort`/`thinking` params from Roder reasoning config, reuse a stable per-thread Cursor `conversation_id`, and open the reasoning submenu when selecting Cursor models that advertise effort options.

## 0.1.6 (2026-06-26)

### Fixes

#### Fix needing a second Ctrl+C to exit a live Claude Code session

Interrupt any in-flight turn during TUI teardown and exit the process cleanly
once the terminal is restored, so a single Ctrl+C fully exits even when an
in-process provider (e.g. Claude Code) has spawned a CLI subprocess whose
runtime tasks would otherwise block shutdown.

## 0.1.5 (2026-06-25)

### Features

#### Render an interactive selection dialog for `request_user_input`

The TUI previously only logged a system line when the model called the
interactive `request_user_input` survey tool, so the question and its options
were never shown and the blocked turn appeared to hang. The TUI now opens a
modal selection dialog listing each option with its description, lets you
navigate with the arrow keys (or `Ctrl+J`/`Ctrl+K`), jump with number keys
`1`-`9`, confirm with `Enter`, and skip with `Esc`. Confirming sends
`thread/resolve_user_input` with the chosen option label keyed by question id;
multi-question surveys are answered one at a time and accumulated before
resolving. The turn timer pauses while the dialog is open and resumes once it is
resolved, and a survey with no answerable options resolves immediately so the
turn never hangs.

The local mock inference provider now also drives `request_user_input` when a
user message contains `FAKE_REQUEST_USER_INPUT`, so the selection dialog can be
exercised end to end without a live provider.

## 0.1.4 (2026-06-24)

### Features

#### Manage external web-search providers from the TUI

Extend the "Web search provider" settings submenu to list the external provider
router options (firecrawl, tavily, perplexity, parallel, synthetic) alongside
the hosted modes, showing each provider's enabled and API-key-configured status
and the active selection. Selecting an external provider persists
`[web_search] mode = "external"`, the chosen `provider`, and enables that
provider's sub-section in user config (applies on restart).

The same hosted modes and external providers are also selectable from the
`Ctrl+P` command palette, and both the menu and palette fall back to reading
providers directly from user config when app-server settings are unavailable.

`settings/get` and `settings/set_web_search` now report the external-router
snapshot via `WebSearchSettings { external_enabled, external_provider, providers }`
and accept an optional `external_provider` selection. New `roder-config`
helpers (`save_web_search_external_provider`, `WebSearchConfig::provider_configured`,
`WebSearchConfig::router_snapshot`, `web_search_router_snapshot`) back the
persistence, status checks, and config-only fallback.

Synthetic web search now auto-configures from the synthetic inference provider:
because both share `SYNTHETIC_API_KEY`, pasting the synthetic provider key
(`providers/configure`) makes the synthetic search provider report as
`key configured`, enables its `[web_search.synthetic]` sub-section, and lets it
resolve the borrowed key at runtime — no separate web-search key entry needed.

## 0.1.3 (2026-06-22)

### Features

#### Show full provider error response body in the timeline detail popup

Chat Completions provider errors (e.g. Synthetic 502 Bad Gateway) previously
discarded the response body entirely and showed only a generic hint like
"provider server error; response body redacted". Now the error includes the
full response body after a separator, so clicking the error row (or pressing
Enter on it) opens the existing tool-detail popup with the complete provider
response text — the same popup used for shell output and file edits.

The auth credential (bearer token or api-key header value) is scrubbed from
the body before it enters the error message, so the popup is safe to share.
Bodies are capped at 4 KB to avoid unbounded error messages.

The TUI also makes `Error` timeline items selectable and clickable, routes
Enter on a selected error row to the detail modal, and renders the popup with
an "error details" title and "Response body" label when the source is a
provider error rather than a shell command.

### Fixes

- Stabilize Roder startup, streaming responses, and provider behavior

#### Add first-party Synthetic inference provider

Adds the `synthetic` provider using Synthetic's OpenAI-compatible Chat
Completions API. The provider ships built-in `syn:` model aliases
(`syn:large:text` default, plus `syn:small:text`, `syn:large:vision`,
`syn:small:vision`), preserves concrete `hf:{owner}/{model}` ids across config,
discovery, and selection, and resolves credentials only from
`SYNTHETIC_API_KEY`/`RODER_SYNTHETIC_API_KEY` or `[providers.synthetic]`. The
provider is visible without credentials so app-server and TUI can show setup
state, and turn-time inference fails locally with setup guidance when the key
is missing. The TUI provider menu points to the Synthetic dashboard for API-key
setup instead of the generic fallback URL.

## 0.1.2 (2026-06-16)

### Fixes

- Improve context compaction across phases 2–4: prune old tool outputs before full compaction, add LLM state-snapshot summarization with verify/reject, hysteresis coalescing, `/compact` via `thread/compact`, `context.compaction_skipped` metrics, and a Grok-style loop regression fixture. Phase 1 fixes remain: compaction boundary on load, once-per-turn guard, ProviderMetadata exclusion from token estimates, and suffix retention from the last user message.

## 0.1.1 (2026-06-15)

### Features

#### One-command Roder package install (`roder install npm:/git:/path`)

Roder packages bundle process extensions, skills, slash commands, and themes
behind a root `roder.toml` manifest. Install from npm, git (shorthand, SSH,
raw URLs, pinned refs), or local paths; manage with `roder packages
list|resources|enable|disable|approve|filter|sync|init`, `roder remove`,
`roder update`, and ephemeral `-e` loading. Resources surface through the
existing skills/commands/theme registries; the process-extension protocol
gains manifest-declared tool providers served over `tools/call`. New
app-server `packages/*` methods, a `/packages` builtin, and a Packages
palette section round out the surfaces. npm lifecycle scripts stay disabled
unless `--allow-scripts` is passed, and package process extensions never
launch before explicit approval.

#### Roadmap orchestration dashboard and multi-worker fan-out

Redesign the roadmap TUI workspace as an orchestration dashboard with progress
header, status strip, tree-style worker rows, and windowed scrolling. Add
orchestrator prompt rules in `roder-roadmap` and fan-out controls in the TUI:
`S` spawns up to eight workers across ready tasks and `s` spawns one for the
focused task.

### Fixes

- Put Models first in the Ctrl+P menu

#### First-party image generation providers (OpenAI GPT Image and Google Gemini Nano Banana)

Provider-neutral image generation through the core media API: an image-capable
`MediaGenerationRequest`/multi-output `MediaGenerationResponse` contract, a new
`ProvidedService::MediaGenerator` extension service, a runtime media generation
service backing the canonical `media_generate_image` tool with a deterministic
offline fallback, new `roder-ext-openai-images` (`gpt-image-2` plus legacy ids)
and `roder-ext-google-images` (Nano Banana 2/Pro/base) provider crates,
`[media.image_generation]` config, `media/image/providers/list` and
`media/image/generate` app-server methods, `roder media` CLI commands, palette
entries, and regenerated schemas/SDK stubs. Live provider smokes stay opt-in
behind `RODER_OPENAI_IMAGE_LIVE` / `RODER_GEMINI_IMAGE_LIVE`.

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
