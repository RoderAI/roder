## 0.1.3 (2026-09-12)

### Fixes

#### Add GPT-6 Astra, Gemini 3.8 Flash, and Claude Fable 5.1

Expose `gpt-6-astra` (1.05M context, efforts up to `max`) on the OpenAI/Codex
catalog, `gemini-3.8-flash` on both the Gemini API and Vertex AI providers, and
`claude-fable-5-1` on the direct Anthropic provider and through the local Claude
Code harness. Provider defaults are unchanged.

Fable 5.1 rejects forced tool choice, so the Anthropic request mapping now sends
`tool_choice: auto` for that model instead of `any`/`tool`.

## 0.1.2 (2026-07-21)

### Fixes

#### Honor provider compaction boundaries

Drop pre-compaction history after OpenAI server-side compaction items so long sessions no longer re-send and re-compact the full window every request. Treat provider compaction as a local transcript boundary, and map Anthropic local context summaries correctly when the emergency client path runs.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
