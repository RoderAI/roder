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

## 0.2.1 (2026-10-11)

### Fixes

#### Name the program embedding Roder in sign-in hints

Programs that embed the Roder runtime ship their own command line, so a hint
such as "run `roder auth login codex`" means nothing to their users.
`roder_api::cli_identity` lets the embedding program set its identity once at
startup with `set_cli_identity(CliIdentity { name, auth_login, auth_dir })`.

- Every sign-in hint (Codex, SuperGrok, Kimi Code, and Jev's text-model notes)
  now names the configured `auth_login` command, with `{provider}` replaced by
  the provider id. Roder's own default is `roder auth login {provider}`.
- The Codex, SuperGrok and Kimi Code token stores keep their files directly in
  the configured `auth_dir`. Without one they keep using `auth/` under Roder's
  data folder: `RODER_DATA_DIR`, else `RODER_CONFIG_DIR`, else `~/.roder`.
- Reading the identity before one is set uses Roder's defaults without fixing
  them, so an embedding program can still set its own afterwards.
- The Codex sign-in's busy-port error names the configured program.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.3 (2026-07-07)

### Fixes

#### Make provider browser auth robust on WSL and add Kimi Code API key login

Print auth URLs before opening browsers, fall back to WSL-friendly browser commands, and allow `roder auth login kimi-code --api-key [KEY]`.

## 0.1.2 (2026-06-22)

### Fixes

- Stabilize Roder startup, streaming responses, and provider behavior

#### Kimi Code OAuth chat requests omit unsupported OpenAI-compat fields

OAuth turns on `api.kimi.com/coding/v1` no longer send `stream_options` or
`parallel_tool_calls`, which caused 400 responses on the managed Kimi Code API.
Adds configurable flags on the shared chat-completions helper and gates
`should_compact_transcript` to test builds only.

## 0.1.1 (2026-06-16)

### Features

#### Added first-class `kimi-code` (aliases: `kimi`, `moonshot`) inference provider and `roder-ext-kimi-code` crate.

- Kimi Code subscription OAuth uses the managed API (`api.kimi.com/coding/v1`) with Kimi device headers and `kimi-code-cli` User-Agent; API keys still use Moonshot Open Platform (`api.moonshot.ai/v1`).
- Catalog entry + `kimi-for-coding` model (K2.7 Code).
- Device OAuth against `auth.kimi.com` with `roder auth login kimi-code`, TUI/app-server `auth/kimi-code/*`, and token storage under `~/.roder/auth/kimi-code.json`.
- API key fallback via env/config (`KIMI_CODE_API_KEY`, `RODER_KIMI_CODE_API_KEY`).
- Registered via extension host (always available, like SuperGrok).
- Docs: `docs/roder-kimi-code-provider.md`.
- Live smoke test added (opt-in via `RODER_KIMI_CODE_LIVE=1`).
