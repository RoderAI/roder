# Changelog

## 0.1.2 (2026-06-30)

### Features

#### Dependency refresh and runner lifecycle method manifest

Register the `runners/pause`, `runners/resume`, `runners/detach`, and
`runners/rejoin` methods in the app-server method manifest and regenerate the
checked-in JSON schema and the TypeScript/Python generated client types so they
expose the runner lifecycle surface.

Refresh dependencies across the workspace and SDKs after validating each change:

- Rust: semver-compatible lockfile updates plus major bumps of `which`
  (6 -> 8), `tokio-tungstenite` (0.28 -> 0.29), `rcgen` (0.13 -> 0.14),
  `rusqlite` (0.38 -> 0.40), and `sqlx` (0.8 -> 0.9). Fixed a `time` 0.3.52
  deprecation (`format_description::parse` -> `parse_borrowed`). The
  `agent-client-protocol-schema` 1.x major is deferred because it renames the
  ACP type surface and needs a dedicated ACP-compliance migration.
- TypeScript/edit-tools: bump `@types/node` to v26.
- Python SDK: refresh the uv lock (anyio, pytest, pyright, idna).

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

### Fixes

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

## 0.0.0

- Initial public SDK release with generated method manifest types, low-level JSON-RPC client, transports, high-level agent/run helpers, and fake app-server fixtures.
