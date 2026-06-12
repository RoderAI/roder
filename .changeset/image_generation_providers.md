---
roder-api: minor
roder-app-server: minor
roder-cli: minor
roder-config: minor
roder-configure: patch
roder-core: minor
roder-ext-google-images: minor
roder-ext-openai-images: minor
roder-extension-host: minor
roder-protocol: minor
roder-sdk-python: patch
roder-sdk-typescript: patch
roder-tools: patch
roder-tui: patch
---

# First-party image generation providers (OpenAI GPT Image and Google Gemini Nano Banana)

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
