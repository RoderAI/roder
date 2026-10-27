## 0.2.0 (2026-10-27)

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

## 0.1.3 (2026-10-11)

### Features

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

## 0.1.2 (2026-07-07)

### Fixes

#### Make provider browser auth robust on WSL and add Kimi Code API key login

Print auth URLs before opening browsers, fall back to WSL-friendly browser commands, and allow `roder auth login kimi-code --api-key [KEY]`.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
