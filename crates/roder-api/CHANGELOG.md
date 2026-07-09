## 0.1.8 (2026-07-09)

### Features

#### Add Grok 4.5 to xAI and SuperGrok providers

Expose `grok-4.5` (500k context, default high reasoning, low/medium/high) as the
default model for both the `xai` API-key provider and SuperGrok OAuth. Keep
legacy Grok 4.3 / 4.20 and SuperGrok Build/Composer entries selectable.

## 0.1.7 (2026-07-01)

### Features

#### Add Claude Fable 5 to the Cursor provider catalog

Expose `claude-fable-5` (1M context, full effort range including `xhigh`/`max`,
default `high`) as a Cursor AgentService-routed model, matching the Anthropic
and Claude Code catalog entries so Fable 5 is selectable across all three
providers.

## 0.1.6 (2026-06-30)

### Features

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

## 0.1.5 (2026-06-26)

### Features

#### Global agent-swarm rate-limit capacity governor

Add the global capacity-shrink / quiet-window recovery throttle for the
`agent_swarm` scheduler (roadmap 104, Task 3 follow-up), so a swarm backs off as
a whole under sustained provider rate limits instead of every child retrying in
parallel with only per-child backoff.

A shared `RateLimitGovernor` is inert until the first provider rate limit. On the
first rate limit it sizes a global capacity from the children that were active
when it hit, then shrinks by one; later rate limits shrink by one more (floor of
one) no more often than `rate_limit_shrink_interval_ms` (default 2000), and
launches are paced apart while throttled. After a quiet
`rate_limit_recovery_interval_ms` (default 180000, three minutes) with no rate
limit, the swarm recovers one unit of capacity. The normal-phase ramp, overlap,
ordering, and `max_concurrency` cap are unchanged.

New bounded config knobs (`[agent_swarm].rate_limit_shrink_interval_ms`,
`rate_limit_recovery_interval_ms`) and matching
`RODER_AGENT_SWARM_RATE_LIMIT_SHRINK_INTERVAL_MS` /
`RODER_AGENT_SWARM_RATE_LIMIT_RECOVERY_INTERVAL_MS` env overrides resolve the
windows. Covered by fake-clock (`tokio::time::pause`) tests for shrink, recovery,
and a sustained-rate-limit end-to-end run that completes in order without
deadlock.

#### Live agent_swarm progress events

Emit an `AgentSwarmProgress` `RoderEvent` each time a swarm child resolves,
carrying a running `completed/failed/aborted/total` snapshot (roadmap 104,
Task 1 follow-up). This lets a client render a live "N/total done" tick between
`AgentSwarmStarted` and `AgentSwarmCompleted` instead of only the final result.
The scheduler reports incremental progress through a new
`AgentSwarmProgressObserver`; the `agent_swarm` tool bridges it onto a runtime
`AgentSwarmProgressSink` supplied on the tool-execution context, which the
runtime backs with the event bus (and thread-event persistence). Children do
not publish progress; only the lead swarm does.

## 0.1.4 (2026-06-26)

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

#### Enforce agent_swarm as the only tool call in a response

The core turn loop now denies any model response that mixes `agent_swarm` with
other tool calls, or issues multiple `agent_swarm` calls at once (roadmap 104,
Task 2). Each call in the offending batch gets an error tool result with
actionable retry text, so the model re-issues `agent_swarm` by itself and every
`tool_call_id` still receives a response (keeping chat-completions transcripts
valid). Adds `roder_api::subagents::agent_swarm_batch_violation` and the shared
`AGENT_SWARM_TOOL_NAME` constant.

#### Agent-swarm lifecycle events on the event bus

Emit `AgentSwarmStarted` (with the child count) and `AgentSwarmCompleted` (with
completed/failed/aborted counts) `RoderEvent`s when the `agent_swarm` tool runs,
so any app-server/SDK/TUI client can observe a swarm as a whole rather than only
the per-child `Subagent*` traces (roadmap 104, Task 1). The runtime emits these
around tool routing; existing notification mappers fall through their catch-all
arms, so no client breaks.

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

#### Rate-limit-aware agent_swarm scheduling

Swarm children that fail with a provider rate limit are now requeued with
exponential backoff (default 3s, 6s, 12s, ... up to 4 retries) instead of
failing outright (roadmap 104, Task 3). The concurrency permit is held across
the backoff so a rate-limited swarm naturally throttles rather than hammering
the provider, and cancellation still wins promptly. Tunable via
`[agent_swarm].rate_limit_max_retries` / `rate_limit_base_backoff_ms` and the
`RODER_AGENT_SWARM_RATE_LIMIT_*` env vars (retries are clamped to a hard cap so
a swarm can never wait unboundedly).

#### Per-thread MCP bearer token

Let a remote client scope a thread's MCP tool calls to a specific identity (for
Vex: a per-user, per-organization capability token). The client forwards the
token via a new `mcpAuthToken` field on `thread/start`; the app-server records
it in an in-memory `roder_api::mcp_auth` registry keyed by thread id, and the
MCP tool extension reads it during execution to authenticate that thread's tool
calls (falling back to the process default when absent). Tokens are short-lived
and re-supplied on each `thread/start`.

#### Subagent and swarm children inherit the parent workspace

Subagent (`task`) and agent-swarm children built their tool-execution context
with no handles, so any child file/shell/search tool failed with "workspace
handle is not available" and the child could not do real work. Children now
inherit the lead turn's workspace, remote workspace, process runner, and
context-artifact handles via the new `SubagentDispatcher::dispatch_with_context`
(the parent goal controller and trace sink are intentionally not inherited).
Each child still runs on its own child thread/turn id, so it operates on the
same repository as an independent agent rather than being confused with the
main-line thread.

### Fixes

#### Cursor fast variants, reasoning params, and stable conversation ids

Expose `composer-2.5-fast` and `gpt-5.5-fast` as first-class catalog models, encode AgentService `fast`/`effort`/`thinking` params from Roder reasoning config, reuse a stable per-thread Cursor `conversation_id`, and open the reasoning submenu when selecting Cursor models that advertise effort options.

## 0.1.3 (2026-06-22)

### Features

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

### Fixes

#### Fix grok-composer-2.5-fast image input handling

The `grok-composer-2.5-fast` model does not support image inputs, but Roder's catalog
hardcoded `supports_images: true` for all xAI/SuperGrok models. The `xai_model` macro now
takes a `supports_images` parameter so non-vision models can correctly declare their
capabilities.

The OpenAI Responses provider engine now checks the model's `supports_images` flag before
emitting `input_image` content items in request payloads. This prevents the xAI API error:

  "Image inputs are not supported by this model."

`grok-composer-2.5-fast` is set to `supports_images: false`; all other Grok models keep
their previous `true` value.

## 0.1.2 (2026-06-16)

### Features

#### Fireworks AI inference provider

Add the first-party `fireworks` inference provider with account-scoped model ids, Fireworks-specific API-key configuration, OpenAI-compatible Responses transport, offline model metadata, model discovery, and app-server provider-list coverage.

### Fixes

- Improve context compaction across phases 2–4: prune old tool outputs before full compaction, add LLM state-snapshot summarization with verify/reject, hysteresis coalescing, `/compact` via `thread/compact`, `context.compaction_skipped` metrics, and a Grok-style loop regression fixture. Phase 1 fixes remain: compaction boundary on load, once-per-turn guard, ProviderMetadata exclusion from token estimates, and suffix retention from the last user message.
- SuperGrok now lists only Grok Build 0.1 (500k context) and Grok Composer 2.5 Fast (200k context), with curated catalog metadata instead of raw xAI /models discovery.

#### Added first-class `kimi-code` (aliases: `kimi`, `moonshot`) inference provider and `roder-ext-kimi-code` crate.

- Kimi Code subscription OAuth uses the managed API (`api.kimi.com/coding/v1`) with Kimi device headers and `kimi-code-cli` User-Agent; API keys still use Moonshot Open Platform (`api.moonshot.ai/v1`).
- Catalog entry + `kimi-for-coding` model (K2.7 Code).
- Device OAuth against `auth.kimi.com` with `roder auth login kimi-code`, TUI/app-server `auth/kimi-code/*`, and token storage under `~/.roder/auth/kimi-code.json`.
- API key fallback via env/config (`KIMI_CODE_API_KEY`, `RODER_KIMI_CODE_API_KEY`).
- Registered via extension host (always available, like SuperGrok).
- Docs: `docs/roder-kimi-code-provider.md`.
- Live smoke test added (opt-in via `RODER_KIMI_CODE_LIVE=1`).

## 0.1.1 (2026-06-15)

### Features

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

### Fixes

#### Process-extension protocol 0.2.0 and Cursor SDK remote-agent bridging

Extend the process-extension protocol with subagent-dispatcher and task-executor services, bridge them in the process host, and add app-server e2e coverage for the cursor-sdk-agents TypeScript child.

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.

#### SuperGrok: default to grok-build-0.1, add it to catalog, enable live /models discovery

- Change SuperGrok provider default_model to `grok-build-0.1`.
- Add `grok-build-0.1` (Grok Build) model entry under the `supergrok` provider (rich xAI capabilities: tools, structured, images, configurable reasoning; 256k ctx).
- `SuperGrokEngine::list_models` now plugs into the shared OpenAI-compatible `/models` + `/v1/models` discovery (using the live SuperGrok OAuth access token for Bearer auth). It uses the standard `~/.roder/models-cache.json` (respects RODER_MODELS_* envs for TTL/refresh/path), background refresh on stale, and falls back to the (now updated) static catalog on no-auth or error. This lets Roder surface the latest models and (basic) capabilities from xAI for SuperGrok subscribers without requiring Roder releases.
- Exposed the reusable `discover_models`, `cached_models`, `save_cached_models`, `cache_ttl`, `force_refresh_requested`, and `CachedProviderModels` from `roder-ext-openai-responses` (pub) so other xAI-flavored paths can reuse.
- Updated tests, docs, and examples to reference `grok-build-0.1` for SuperGrok. (Composer 2.5 remains a Cursor-native model.)
- Live validation with real SuperGrok token confirms `/models` returns (among others) `grok-build-0.1` + current Grok variants.
