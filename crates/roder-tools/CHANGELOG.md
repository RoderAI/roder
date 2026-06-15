## 0.1.1 (2026-06-15)

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

#### Make grep and glob tools reliable for agents

Transcript analysis showed 44% of grep calls and 46% of glob calls returned empty results, mostly from the tool contract fighting model conventions. This change:

- treats grep queries as regex by default (models habitually send `a|b` patterns without setting `regex: true`, which previously matched nothing as a literal), with a clear error and literal-mode hint for invalid patterns
- replaces empty zero-match output with an explanation of what was searched (mode, case, scope, engine, file counts) plus remedial hints
- backs glob with globset, adding `{a,b}`, `[..]` and proper `**` support, and resolves absolute or `~` patterns inside the workspace instead of silently matching nothing; patterns outside the workspace now error clearly
- searches explicitly scoped ignored directories (node_modules, dist, gitignored paths) by relaxing ancestor ignore rules when the caller names such a path directly
- passes the canonicalized search path to the engine so `~`, symlink, and case differences no longer silently drop every indexed match
- invalidates the cached search index after shell and exec commands so files they create are found by the next grep
