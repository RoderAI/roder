## 0.2.1 (2026-10-09)

### Fixes

#### Native Linux and macOS desktop computer use through Cua Driver

Enable runner-bound Cua desktop tools with observation, grounded pointer and
keyboard input, Unicode value replacement, window controls, permission gating and fresh
screenshots after actions. Reuse the thread's remote runner and credentials.
Keep screenshots intact through Responses, Anthropic, Gemini and ACP replay.
Read large driver results through the runner filesystem rather than truncated
Blaxel process stdout. Include pinned driver provisioning and a live native
calculator acceptance harness.

Add an explicitly selected local macOS backend using the signed CuaDriver app's
daemon. Preserve approvals and image replay, fence cancelled native input,
invalidate grounding across local sessions and daemon generations, and keep
large accessibility trees usable inline. Qualify native macOS Calculator via
the public Roder runtime plus an owned AppKit input fixture. Local macOS never
falls back from a remote runner thread. Exact semantic value replacement has
no delivery-mode argument on the shared canonical tool surface.

Include a full XFCE/X11 desktop workflow with native GUI app launching, file
drag, ODT creation/save/reopen and independent file grading. Add a local
read-only live screen viewer and preserve plain-text Linux driver refusals
without discarding their error status or replaying input.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.3 (2026-09-12)

### Fixes

#### Add Gemini 3.7 Flash and Grok 4.6 to provider model catalogs

Expose Gemini 3.7 Flash and Grok 4.6 through native, Cursor, xAI, SuperGrok,
and OpenRouter integrations with provider-specific context windows and
reasoning controls. Retire Grok 4.5 and Grok Build from active catalogs.

## 0.1.2 (2026-08-20)

### Fixes

#### Accept more tool schemas in Gemini

Remove unsupported `uniqueItems` fields and avoid converting non-string constants into invalid Gemini enum values.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
