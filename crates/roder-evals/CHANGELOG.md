## 0.3.0 (2026-10-03)

### Breaking Changes

#### Bind hosted external tools to one authenticated connection

Hosted external tool threads require an executor binding before a turn starts.
Only that connection receives execution requests and may resolve them with the
current lease and turn. Disconnect, unbind, and takeover terminate pending
requests. Metadata readback supports recovery without replaying effects.
The TypeScript SDK adds a generic executor helper with cancellation and duplicate
request suppression for hosts that own their existing notification loop.

Custom TypeScript transports expose a synchronous closedSignal so executor callbacks abort immediately on transport loss. The unused local startupTimeoutMs option is removed.

Release the reverse dependency closure together so registry builds share the new protocol and core types.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.3 (2026-09-26)

### Fixes

#### Publish today's content under fresh crate versions

`roder-api` 0.1.21, `roder-core` 0.1.19, `roder-app-server` 0.1.17,
`roder-evals` 0.1.2 and `roder-ext-openai-responses` 0.1.10 were published to
crates.io on 2026-09-23 from a branch that predates the Codex agent backend,
the Jev browser tool, the Chrome bridge work and the Grok 4.7 catalog entries.
crates.io versions are immutable, so those numbers cannot carry the current
code and the registry copies do not match the git tags of the same version.

Bump them so the released content reaches crates.io under versions that
describe it. `roder-ext-jev` and `roder-ext-codex-backend` are bumped with
them: both are new crates whose first publish must depend on a registry
`roder-api` that actually contains `roder_api::backend`, which the 0.1.21
registry copy does not.

Adds the package-local READMEs both new crates need to publish.

## 0.1.2 (2026-09-26)

### Fixes

#### Per-turn OpenAI service tier (Fast mode), and GPT-6 Sol and Luna

A caller can now choose the OpenAI service tier for a turn.
`StartTurnRequest::service_tier_override` (for example `"priority"` for Fast
mode) is carried to every inference round of the turn as
`RuntimeHints::service_tier`. The OpenAI Responses provider sends it as the
top-level `service_tier` request field only on the OpenAI profile; OpenRouter,
xAI, and Fireworks never receive it. `None` keeps the provider default.

The tier OpenAI reports it actually served (`response.service_tier`) is recorded
on `TokenUsage::service_tier`, so a biller can tell a request that ran fast from
one that was downgraded to `"default"` under load. Both new fields are optional
and default to absent when older payloads are deserialized.

`StartTurnRequest` gains a public field, so code that builds it with a struct
literal must add `service_tier_override: None`.

The OpenAI/Codex catalog adds `gpt-6-sol` (efforts `low` through `max`, like
`gpt-6-astra`) and `gpt-6-luna` (efforts `low` through `max`, like
`gpt-5.6-luna`), so per-turn reasoning validation accepts them.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
