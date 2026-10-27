## 0.3.0 (2026-10-27)

### Breaking Changes

#### Release a coherent runtime API compatibility boundary

Adopt explicit shared-read/exclusive/coordinator scheduling, typed custom-provider
capabilities, and owned worktree metadata. Rust integrators must update
ToolExecutor implementations to execution_mode and initialize the new public
configuration and ownership fields.

Release the shared API's reverse dependency closure together so older consumers
cannot resolve patch-compatible packages that expose incompatible API types.
The CLI package also exports a Rust library, so its compatibility release is
0.4.0. Existing command-line use remains supported.

## 0.2.1 (2026-10-14)

### Features

#### Configure OpenRouter provider routing and service tiers

OpenRouter requests now route by price by default (`provider.sort = "price"`). `[providers.openrouter.routing]` and per-model `[providers.openrouter.model_routing."<slug>"]` tables replace that default and are sent as OpenRouter's `provider` object, so a model can be pinned to specific upstream providers (`only = ["deepinfra/fp8"]`, `allow_fallbacks = false`), capped with `max_price`, or sorted by throughput or latency; `routing = {}` restores OpenRouter's load balancing. `[providers.openrouter] service_tier` (`flex`, `priority`, `ultrafast`) is sent as the request's service tier, and a per-request runtime service tier now reaches OpenRouter as well.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.2 (2026-09-12)

### Fixes

#### Add Gemini 3.7 Flash and Grok 4.6 to provider model catalogs

Expose Gemini 3.7 Flash and Grok 4.6 through native, Cursor, xAI, SuperGrok,
and OpenRouter integrations with provider-specific context windows and
reasoning controls. Retire Grok 4.5 and Grok Build from active catalogs.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
